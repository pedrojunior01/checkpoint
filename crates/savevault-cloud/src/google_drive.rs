//! Google Drive provider — implementação real via OAuth 2.0 + API v3.
//!
//! Organização dos arquivos no Drive:
//! ```text
//! SaveVault/                          <- pasta raiz criada pelo app
//!   <game-uuid>/                      <- pasta por jogo
//!     <file-uuid>/                    <- pasta por arquivo de save
//!       v1__<filename>.<ext>          <- snapshots versionados
//!       v2__<filename>.<ext>
//! ```

use anyhow::{anyhow, Result};
use reqwest::header::AUTHORIZATION;
use std::path::Path;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::auth::AuthService;
use crate::CloudProvider;

/// Timeout padrão para estabelecimento de conexão TCP/TLS (DNS + Handshake).
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout padrão por requisição HTTP para operações com o Google Drive.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

fn build_default_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
        .timeout(DEFAULT_REQUEST_TIMEOUT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Valida se um `remote_file_id` é um identificador remoto opaco seguro.
///
/// Invariante de Segurança (SEC-01):
/// - Rejeita caracteres de caminho (`/`, `\`), prevenindo path traversal;
/// - Rejeita componentes relativos (`..`, `...`, ou strings compostas unicamente por pontos);
/// - Rejeita delimitadores de URL (`?`, `#`, `&`), prevenindo query/fragment injection;
/// - Rejeita caracteres de controle ASCII (`c.is_control()`) e espaços em branco;
/// - Rejeita identificador vazio.
///
/// Não utiliza uma whitelist artificial para não quebrar compatibilidade com IDs
/// opacos válidos gerados pela Google Drive API.
pub fn validate_drive_file_id(remote_file_id: &str) -> Result<()> {
    let trimmed = remote_file_id.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Identificador remoto não pode ser vazio"));
    }
    if remote_file_id.contains('/')
        || remote_file_id.contains('\\')
        || remote_file_id.contains('?')
        || remote_file_id.contains('#')
        || remote_file_id.contains('&')
        || remote_file_id.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(anyhow!(
            "Identificador remoto inválido: '{}'. Esperado Google Drive File ID opaco, não caminho ou parâmetros de URL.",
            remote_file_id
        ));
    }
    // Rejeita tentativas de path traversal puramente relativas como '.', '..', '...', etc.
    if remote_file_id.chars().all(|c| c == '.') {
        return Err(anyhow!(
            "Identificador remoto inválido: '{}'. Tentativa de path traversal detectada.",
            remote_file_id
        ));
    }
    Ok(())
}

/// Nome da pasta raiz criada no Google Drive do usuário.
const DRIVE_ROOT_FOLDER: &str = "SaveVault";
/// URL base da Google Drive API v3.
const DRIVE_API_BASE: &str = "https://www.googleapis.com/drive/v3";
/// URL base para uploads multipart na Drive API.
const DRIVE_UPLOAD_BASE: &str = "https://www.googleapis.com/upload/drive/v3";

// ---------------------------------------------------------------------------
// GoogleDriveProvider
// ---------------------------------------------------------------------------

/// Provedor Google Drive com autenticação OAuth 2.0.
pub struct GoogleDriveProvider {
    auth: AuthService,
    client: reqwest::Client,
    api_base: String,
    upload_base: String,
}

impl GoogleDriveProvider {
    /// Cria um novo provider usando as credenciais do Google Cloud Console.
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            auth: AuthService::new(client_id, client_secret),
            client: build_default_http_client(),
            api_base: DRIVE_API_BASE.to_string(),
            upload_base: DRIVE_UPLOAD_BASE.to_string(),
        }
    }

    /// Cria um provider para testes apontando para endpoints mockados.
    #[doc(hidden)]
    pub fn with_custom_endpoints(
        auth: AuthService,
        api_base: String,
        upload_base: String,
    ) -> Self {
        Self {
            auth,
            client: build_default_http_client(),
            api_base,
            upload_base,
        }
    }

    /// Permite injetar um cliente HTTP configurado especificamente (ex: com timeouts reduzidos para testes).
    #[doc(hidden)]
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    /// Tenta criar um provider a partir das variáveis de ambiente.
    /// Retorna `None` se as variáveis não estiverem definidas.
    pub fn from_env() -> Option<Self> {
        let client_id = std::env::var("SAVEVAULT_GOOGLE_CLIENT_ID").ok()?;
        let client_secret = std::env::var("SAVEVAULT_GOOGLE_CLIENT_SECRET").ok()?;
        Some(Self::new(client_id, client_secret))
    }

    /// Retorna `true` se o usuário já realizou o login (há um refresh token salvo).
    pub fn is_authenticated(&self) -> bool {
        self.auth.is_authenticated()
    }

    /// Inicia o fluxo interativo de login via navegador.
    pub async fn login(&self) -> Result<()> {
        self.auth.login_interactive().await
    }

    /// Remove as credenciais salvas (logout).
    pub fn logout(&self) -> Result<()> {
        self.auth.logout()
    }

    // -----------------------------------------------------------------------
    // Helpers internos
    // -----------------------------------------------------------------------

    /// Faz uma requisição autenticada e devolve o JSON de resposta.
    async fn get_json(&self, url: &str, token: &str) -> Result<serde_json::Value> {
        let res = self
            .client
            .get(url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        let body: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);

        if !status.is_success() {
            let error_msg = body["error"]["message"]
                .as_str()
                .unwrap_or(if text.is_empty() { "desconhecido" } else { &text });
            return Err(anyhow!(
                "Drive API GET erro {}: {}",
                status,
                error_msg
            ));
        }

        if body.is_null() && !text.is_empty() {
            return Err(anyhow!("Resposta JSON inválida da Drive API: {}", text));
        }

        Ok(body)
    }

    /// Busca ou cria uma pasta no Drive com o nome especificado.
    /// Retorna o ID da pasta.
    async fn get_or_create_folder(
        &self,
        token: &str,
        name: &str,
        parent_id: Option<&str>,
    ) -> Result<String> {
        // Monta a query de busca
        let parent_clause = parent_id
            .map(|id| format!(" and '{}' in parents", id))
            .unwrap_or_default();

        let query = format!(
            "name='{}' and mimeType='application/vnd.google-apps.folder' and trashed=false{}",
            name, parent_clause
        );

        let url = format!(
            "{}/files?q={}&fields=files(id,name)&spaces=drive",
            self.api_base,
            urlencoding_query(&query),
        );

        let response = self.get_json(&url, token).await?;

        if let Some(files) = response["files"].as_array() {
            if let Some(first) = files.first() {
                if let Some(id) = first["id"].as_str() {
                    debug!("Pasta '{}' encontrada no Drive: {}", name, id);
                    return Ok(id.to_string());
                }
            }
        }

        // Não encontrou: cria a pasta
        let mut metadata = serde_json::json!({
            "name": name,
            "mimeType": "application/vnd.google-apps.folder"
        });

        if let Some(parent) = parent_id {
            metadata["parents"] = serde_json::json!([parent]);
        }

        let res = self
            .client
            .post(format!("{}/files", self.api_base))
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .json(&metadata)
            .send()
            .await?;

        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        let body: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);

        if !status.is_success() {
            let error_msg = body["error"]["message"]
                .as_str()
                .unwrap_or(if text.is_empty() { "erro desconhecido" } else { &text });
            return Err(anyhow!(
                "Falha ao criar pasta '{}': HTTP {}: {}",
                name,
                status,
                error_msg
            ));
        }

        let id = body["id"]
            .as_str()
            .ok_or_else(|| anyhow!("Resposta inválida ao criar pasta"))?
            .to_string();

        info!("Pasta '{}' criada no Drive: {}", name, id);
        Ok(id)
    }

    /// Garante que existe a hierarquia de pastas no Drive para um backup:
    /// `SaveVault/<game_uuid>/<file_uuid>/`
    ///
    /// Retorna o ID da pasta folha (file_uuid).
    async fn ensure_backup_path(
        &self,
        token: &str,
        game_uuid: &str,
        file_uuid: &str,
    ) -> Result<String> {
        let root_id = self
            .get_or_create_folder(token, DRIVE_ROOT_FOLDER, None)
            .await?;

        let game_id = self
            .get_or_create_folder(token, game_uuid, Some(&root_id))
            .await?;

        let file_id = self
            .get_or_create_folder(token, file_uuid, Some(&game_id))
            .await?;

        Ok(file_id)
    }

    /// Constrói com segurança a URL para uma operação em `/files/{drive_file_id}`
    /// utilizando `reqwest::Url` e manipulação segura de segmentos de caminho.
    pub fn build_file_url(
        &self,
        remote_file_id: &str,
        query: Option<&[(&str, &str)]>,
    ) -> Result<reqwest::Url> {
        validate_drive_file_id(remote_file_id)?;

        let mut url = reqwest::Url::parse(&self.api_base)
            .map_err(|e| anyhow!("Base URL da API inválida '{}': {}", self.api_base, e))?;

        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| anyhow!("Não é possível manipular segmentos de caminho na URL base: {}", self.api_base))?;
            segments.pop_if_empty().push("files").push(remote_file_id);
        }

        if let Some(params) = query {
            url.query_pairs_mut().extend_pairs(params.iter().copied());
        }

        Ok(url)
    }
}

/// Estrutura tipada de resposta de upload/metadados da Google Drive API.
#[derive(serde::Deserialize, Debug, Clone)]
pub struct DriveUploadResponse {
    pub id: String,
    pub name: Option<String>,
    pub size: Option<String>,
}

// ---------------------------------------------------------------------------
// Implementação da trait CloudProvider
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl CloudProvider for GoogleDriveProvider {
    /// Faz o upload de um arquivo local para o Google Drive.
    ///
    /// `cloud_path` deve ter o formato: `<game_uuid>/<file_uuid>/<filename>`
    /// Retorna o `drive_file_id` remoto (Google Drive File ID alfanumérico opaco).
    ///
    /// Idempotência: antes de fazer o upload, verifica se o arquivo já existe na pasta
    /// de destino do Drive (por exemplo, após falha do processo logo após upload anterior).
    /// Se já existir, reutiliza o ID existente sem criar duplicatas remotas.
    async fn upload(&self, local_path: &Path, cloud_path: &str) -> Result<String> {
        let token = self.auth.get_fresh_access_token().await?;

        // Decompõe o cloud_path em partes
        let parts: Vec<&str> = cloud_path.splitn(3, '/').collect();
        let (game_uuid, file_uuid, filename) = match parts.as_slice() {
            [g, f, n] => (*g, *f, *n),
            _ => {
                return Err(anyhow!(
                    "cloud_path inválido: '{}'. Esperado: '<game_uuid>/<file_uuid>/<filename>'",
                    cloud_path
                ))
            }
        };

        let folder_id = self
            .ensure_backup_path(&token, game_uuid, file_uuid)
            .await?;

        // Verificação de idempotência: se o arquivo já existe no Drive na mesma pasta, reutiliza o ID
        let check_query = format!(
            "name='{}' and '{}' in parents and trashed=false",
            filename, folder_id
        );
        let check_url = format!(
            "{}/files?q={}&fields=files(id,name,size)&spaces=drive",
            self.api_base,
            urlencoding_query(&check_query)
        );
        if let Ok(existing_resp) = self.get_json(&check_url, &token).await {
            if let Some(files) = existing_resp["files"].as_array() {
                if let Some(first) = files.first() {
                    if let Some(id) = first["id"].as_str() {
                        if !id.trim().is_empty() {
                            info!(
                                "Arquivo '{}' já existente no Drive (id: {}). Reutilizando ID remoto com garantia de idempotência.",
                                filename, id
                            );
                            return Ok(id.to_string());
                        }
                    }
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

        // Detecta o MIME type pelo conteúdo (usa octet-stream como fallback seguro)
        let mime = mime_from_extension(local_path);

        // Metadados do arquivo no Drive
        let metadata = serde_json::json!({
            "name": filename,
            "parents": [folder_id]
        });

        // Upload Multipart: metadados JSON + binário do arquivo numa única requisição
        let metadata_part = reqwest::multipart::Part::text(metadata.to_string())
            .mime_str("application/json; charset=UTF-8")?;

        let file_part = reqwest::multipart::Part::bytes(file_bytes)
            .mime_str(mime)?
            .file_name(filename.to_string());

        let form = reqwest::multipart::Form::new()
            .part("metadata", metadata_part)
            .part("file", file_part);

        let res = self
            .client
            .post(format!(
                "{}/files?uploadType=multipart&fields=id,name,size",
                self.upload_base
            ))
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .multipart(form)
            .send()
            .await?;

        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        if !status.is_success() {
            let error_msg = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| if text.is_empty() { "erro desconhecido".to_string() } else { text });
            return Err(anyhow!(
                "Falha no upload de '{}' para o Drive: HTTP {}: {}",
                filename,
                status,
                error_msg
            ));
        }

        let upload_resp: DriveUploadResponse = serde_json::from_str(&text)
            .map_err(|e| anyhow!("Resposta de upload inválida do Google Drive: {}. Body: {}", e, text))?;

        if upload_resp.id.trim().is_empty() {
            return Err(anyhow!("Google Drive retornou upload com campo 'id' vazio"));
        }

        info!(
            "Upload concluído: '{}' → Drive/{} (drive_file_id: {})",
            local_path.display(),
            cloud_path,
            upload_resp.id
        );

        Ok(upload_resp.id)
    }

    /// Faz o download de um arquivo do Google Drive para o disco local.
    ///
    /// `remote_file_id` deve ser o **Google Drive File ID** opaco alfanumérico.
    async fn download(&self, remote_file_id: &str, local_path: &Path) -> Result<()> {
        let url = self.build_file_url(remote_file_id, Some(&[("alt", "media")]))?;

        let token = self.auth.get_fresh_access_token().await?;

        let res = self
            .client
            .get(url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        let status = res.status();
        if !status.is_success() {
            return Err(anyhow!(
                "Falha ao baixar arquivo do Drive (id: {}): HTTP {}",
                remote_file_id,
                status
            ));
        }

        let bytes = res.bytes().await?;

        if let Some(parent) = local_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::write(local_path, &bytes).await?;

        info!(
            "Download concluído: Drive/{} → '{}'",
            remote_file_id,
            local_path.display()
        );

        Ok(())
    }

    /// Remove um arquivo do Google Drive pelo seu File ID.
    async fn delete(&self, remote_file_id: &str) -> Result<()> {
        let url = self.build_file_url(remote_file_id, None)?;

        let token = self.auth.get_fresh_access_token().await?;

        let res = self
            .client
            .delete(url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        let status = res.status();

        // 204 No Content = sucesso; 404 = já não existe (toleramos)
        if status.is_success() || status.as_u16() == 404 {
            if status.as_u16() == 404 {
                warn!(
                    "Arquivo Drive/{} não encontrado (já foi deletado?)",
                    remote_file_id
                );
            } else {
                info!("Arquivo Drive/{} removido.", remote_file_id);
            }
            return Ok(());
        }

        Err(anyhow!(
            "Falha ao deletar arquivo Drive/{}: HTTP {}",
            remote_file_id,
            status
        ))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Retorna o MIME type baseado na extensão do arquivo.
fn mime_from_extension(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("zip") => "application/zip",
        Some("json") => "application/json",
        Some("toml") => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Percent-encoding simples para uso em parâmetros de query da URL.
fn urlencoding_query(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}
