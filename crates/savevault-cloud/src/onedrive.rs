//! Microsoft OneDrive Provider — implementação via Microsoft Graph API v1.0.
//!
//! Organização dos arquivos no OneDrive:
//! ```text
//! OneDrive/
//!   Aplicativos/ (ou Apps/)
//!     SaveVault/                      <- pasta do aplicativo (special/approot)
//!       <game-uuid>/                  <- pasta por jogo
//!         <file-uuid>/                <- pasta por arquivo de save
//!           v1__<filename>.<ext>      <- snapshots versionados
//!           v2__<filename>.<ext>
//! ```
//!
//! Princípios de Segurança e Robustez:
//! - Escopo restrito à pasta do aplicativo: `Files.ReadWrite.AppFolder` (Invariante Least Privilege).
//! - Identificadores remotos opacos validados contra path traversal e injeções (Invariante SEC-01).
//! - Operações de download com escrita atômica via staging temporário.
//! - Idempotência: verificação pré-upload para reaproveitar arquivos idênticos já enviados.
//! - Tolerância a 404 na remoção para idempotência distribuída.

use anyhow::{anyhow, Result};
use reqwest::header::AUTHORIZATION;
use std::path::Path;
use std::time::Duration;
use tracing::{info, warn};

use crate::google_drive::validate_drive_file_id;
use crate::onedrive_auth::OneDriveAuthService;
use crate::CloudProvider;

/// URL base padrão da Microsoft Graph API v1.0.
pub const ONEDRIVE_API_BASE: &str = "https://graph.microsoft.com/v1.0";

/// Timeout padrão para estabelecimento de conexão TCP/TLS.
pub const ONEDRIVE_DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Timeout padrão por requisição HTTP para operações no OneDrive.
pub const ONEDRIVE_DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

fn build_default_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(ONEDRIVE_DEFAULT_CONNECT_TIMEOUT)
        .timeout(ONEDRIVE_DEFAULT_REQUEST_TIMEOUT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Provedor Microsoft OneDrive utilizando a Microsoft Graph API v1.0.
pub struct OneDriveProvider {
    auth: OneDriveAuthService,
    client: reqwest::Client,
    api_base: String,
}

impl OneDriveProvider {
    /// Cria uma nova instância a partir das credenciais da aplicação no Azure / Microsoft Entra.
    pub fn new(client_id: String, client_secret: Option<String>) -> Self {
        Self {
            auth: OneDriveAuthService::new(client_id, client_secret),
            client: build_default_http_client(),
            api_base: ONEDRIVE_API_BASE.to_string(),
        }
    }

    /// Cria um provider para testes com endpoints personalizados (mock server).
    #[doc(hidden)]
    pub fn with_custom_endpoints(auth: OneDriveAuthService, api_base: String) -> Self {
        Self {
            auth,
            client: build_default_http_client(),
            api_base,
        }
    }

    /// Permite injetar um cliente HTTP configurado especificamente (ex: timeouts reduzidos).
    #[doc(hidden)]
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    /// Tenta instanciar o provedor a partir das variáveis de ambiente:
    /// - `SAVEVAULT_ONEDRIVE_CLIENT_ID` (obrigatório)
    /// - `SAVEVAULT_ONEDRIVE_CLIENT_SECRET` (opcional para clientes públicos nativos)
    pub fn from_env() -> Option<Self> {
        let client_id = std::env::var("SAVEVAULT_ONEDRIVE_CLIENT_ID").ok()?;
        let client_secret = std::env::var("SAVEVAULT_ONEDRIVE_CLIENT_SECRET").ok();
        Some(Self::new(client_id, client_secret))
    }

    /// Retorna `true` se o usuário possui credencial salva no sistema.
    pub fn is_authenticated(&self) -> bool {
        self.auth.is_authenticated()
    }

    /// Inicia o fluxo interativo de login no navegador.
    pub async fn login(&self) -> Result<()> {
        self.auth.login_interactive().await
    }

    /// Remove as credenciais salvas no cofre do sistema (logout).
    pub fn logout(&self) -> Result<()> {
        self.auth.logout()
    }

    /// Constrói a URL para operações diretas sobre itens por ID remoto seguro.
    pub fn build_item_url(
        &self,
        remote_file_id: &str,
        subpath: Option<&str>,
    ) -> Result<reqwest::Url> {
        validate_drive_file_id(remote_file_id)?;

        let mut url = reqwest::Url::parse(&self.api_base)
            .map_err(|e| anyhow!("Base URL da API inválida '{}': {}", self.api_base, e))?;

        {
            let mut segments = url.path_segments_mut().map_err(|_| {
                anyhow!(
                    "Não é possível manipular segmentos de caminho na URL base: {}",
                    self.api_base
                )
            })?;
            segments
                .pop_if_empty()
                .push("me")
                .push("drive")
                .push("items")
                .push(remote_file_id);
            if let Some(sub) = subpath {
                segments.push(sub);
            }
        }

        Ok(url)
    }

    /// Faz uma requisição GET autenticada retornando o JSON de resposta.
    async fn get_json(&self, url: &str, token: &str) -> Result<serde_json::Value> {
        let res = self
            .client
            .get(url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        let body: serde_json::Value =
            serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);

        if !status.is_success() {
            let error_msg = extract_graph_error(&body, &text);
            return Err(anyhow!(
                "Microsoft Graph API GET erro {}: {}",
                status,
                error_msg
            ));
        }

        Ok(body)
    }
}

/// Estrutura tipada de resposta da Microsoft Graph API para arquivos.
#[derive(serde::Deserialize, Debug, Clone)]
pub struct OneDriveItemResponse {
    pub id: String,
    pub name: Option<String>,
    pub size: Option<u64>,
}

#[async_trait::async_trait]
impl CloudProvider for OneDriveProvider {
    /// Faz o upload de um arquivo para o Microsoft OneDrive dentro do approot do SaveVault.
    ///
    /// `cloud_path` deve ter o formato: `<game_uuid>/<file_uuid>/<filename>`
    /// Retorna o `id` opaco do item gerado pelo OneDrive.
    async fn upload(&self, local_path: &Path, cloud_path: &str) -> Result<String> {
        let token = self.auth.get_fresh_access_token().await?;

        let clean_path = cloud_path.trim_start_matches('/');
        if clean_path.is_empty() || clean_path.contains("..") {
            return Err(anyhow!("cloud_path inválido: '{}'", cloud_path));
        }

        // Idempotência: verifica se o arquivo já existe no approot com o mesmo caminho
        let check_url = format!(
            "{}/me/drive/special/approot:/{}:",
            self.api_base, clean_path
        );
        if let Ok(existing_resp) = self.get_json(&check_url, &token).await {
            if let Some(id) = existing_resp["id"].as_str() {
                if !id.trim().is_empty() {
                    info!(
                        "Arquivo '{}' já existente no OneDrive (id: {}). Reutilizando ID remoto com garantia de idempotência.",
                        clean_path, id
                    );
                    return Ok(id.to_string());
                }
            }
        }

        let file_bytes = tokio::fs::read(local_path).await.map_err(|e| {
            anyhow!(
                "Falha ao ler arquivo local '{}': {}",
                local_path.display(),
                e
            )
        })?;

        // Microsoft Graph API upload direto no approot com criação automática de pastas
        let upload_url = format!(
            "{}/me/drive/special/approot:/{}:/content",
            self.api_base, clean_path
        );

        let res = self
            .client
            .put(&upload_url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(file_bytes)
            .send()
            .await?;

        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        if !status.is_success() {
            let body_val: serde_json::Value =
                serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
            let error_msg = extract_graph_error(&body_val, &text);
            return Err(anyhow!(
                "Falha no upload de '{}' para o OneDrive: HTTP {}: {}",
                clean_path,
                status,
                error_msg
            ));
        }

        let item: OneDriveItemResponse = serde_json::from_str(&text).map_err(|e| {
            anyhow!(
                "Resposta de upload inválida do OneDrive: {}. Body: {}",
                e,
                text
            )
        })?;

        if item.id.trim().is_empty() {
            return Err(anyhow!("OneDrive retornou resposta de upload sem 'id'"));
        }

        info!(
            "Upload concluído: '{}' → OneDrive/{} (id: {})",
            local_path.display(),
            clean_path,
            item.id
        );

        Ok(item.id)
    }

    /// Faz o download de um arquivo do OneDrive para o disco local de forma atômica.
    async fn download(&self, remote_file_id: &str, local_path: &Path) -> Result<()> {
        let url = self.build_item_url(remote_file_id, Some("content"))?;
        let token = self.auth.get_fresh_access_token().await?;

        let res = self
            .client
            .get(url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        let status = res.status();
        if !status.is_success() {
            let text = res.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Falha ao baixar arquivo do OneDrive (id: {}): HTTP {} - {}",
                remote_file_id,
                status,
                text
            ));
        }

        let bytes = res.bytes().await?;

        let parent_dir = local_path.parent().unwrap_or_else(|| Path::new("."));
        if !parent_dir.exists() {
            tokio::fs::create_dir_all(parent_dir).await?;
        }

        // Escrita atômica via arquivo temporário de staging no mesmo diretório
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let temp_file = parent_dir.join(format!(".savevault_onedrive_dl_tmp_{:x}", nanos));

        if let Err(e) = tokio::fs::write(&temp_file, &bytes).await {
            let _ = tokio::fs::remove_file(&temp_file).await;
            return Err(anyhow!(
                "Falha ao gravar arquivo temporário de download: {}",
                e
            ));
        }

        if let Err(e) = tokio::fs::rename(&temp_file, local_path).await {
            let _ = tokio::fs::remove_file(&temp_file).await;
            return Err(anyhow!(
                "Falha ao mover arquivo baixado para destino final: {}",
                e
            ));
        }

        info!(
            "Download do OneDrive concluído com escrita atômica: id {} → '{}'",
            remote_file_id,
            local_path.display()
        );

        Ok(())
    }

    /// Remove um arquivo do OneDrive pelo seu ID remoto.
    async fn delete(&self, remote_file_id: &str) -> Result<()> {
        let url = self.build_item_url(remote_file_id, None)?;
        let token = self.auth.get_fresh_access_token().await?;

        let res = self
            .client
            .delete(url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        let status = res.status();

        // 204 No Content = sucesso; 404 Not Found = tolerado (já não existe)
        if status.is_success() || status.as_u16() == 404 {
            if status.as_u16() == 404 {
                warn!(
                    "Arquivo OneDrive/{} não encontrado (já deletado).",
                    remote_file_id
                );
            } else {
                info!("Arquivo OneDrive/{} removido com sucesso.", remote_file_id);
            }
            return Ok(());
        }

        let text = res.text().await.unwrap_or_default();
        Err(anyhow!(
            "Falha ao deletar arquivo OneDrive/{}: HTTP {} - {}",
            remote_file_id,
            status,
            text
        ))
    }
}

/// Extrai a mensagem de erro padronizada da Microsoft Graph API.
fn extract_graph_error(body: &serde_json::Value, raw_text: &str) -> String {
    body["error"]["message"]
        .as_str()
        .unwrap_or_else(|| {
            if raw_text.trim().is_empty() {
                "erro desconhecido"
            } else {
                raw_text
            }
        })
        .to_string()
}
