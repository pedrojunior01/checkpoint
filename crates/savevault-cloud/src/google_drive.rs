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
use tracing::{debug, info, warn};

use crate::auth::AuthService;
use crate::CloudProvider;

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
}

impl GoogleDriveProvider {
    /// Cria um novo provider usando as credenciais do Google Cloud Console.
    ///
    /// As credenciais (`client_id`, `client_secret`) devem vir das variáveis
    /// de ambiente `SAVEVAULT_GOOGLE_CLIENT_ID` e `SAVEVAULT_GOOGLE_CLIENT_SECRET`,
    /// ou serem lidas de um arquivo de credenciais local (não versionado).
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            auth: AuthService::new(client_id, client_secret),
            client: reqwest::Client::new(),
        }
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
        let body: serde_json::Value = res.json().await?;

        if !status.is_success() {
            return Err(anyhow!(
                "Drive API GET erro {}: {}",
                status,
                body["error"]["message"].as_str().unwrap_or("desconhecido")
            ));
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
            DRIVE_API_BASE,
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
            .post(format!("{}/files", DRIVE_API_BASE))
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .json(&metadata)
            .send()
            .await?;

        let status = res.status();
        let body: serde_json::Value = res.json().await?;

        if !status.is_success() {
            return Err(anyhow!(
                "Falha ao criar pasta '{}': {}",
                name,
                body["error"]["message"].as_str().unwrap_or("erro desconhecido")
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
}

// ---------------------------------------------------------------------------
// Implementação da trait CloudProvider
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl CloudProvider for GoogleDriveProvider {
    /// Faz o upload de um arquivo local para o Google Drive.
    ///
    /// `cloud_path` deve ter o formato: `<game_uuid>/<file_uuid>/<filename>`
    /// Ex: `"a1b2c3/d4e5f6/save.dat_v3"`
    async fn upload(&self, local_path: &Path, cloud_path: &str) -> Result<()> {
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
                DRIVE_UPLOAD_BASE
            ))
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .multipart(form)
            .send()
            .await?;

        let status = res.status();
        let body: serde_json::Value = res.json().await?;

        if !status.is_success() {
            return Err(anyhow!(
                "Falha no upload de '{}' para o Drive: {}",
                filename,
                body["error"]["message"].as_str().unwrap_or("erro desconhecido")
            ));
        }

        info!(
            "Upload concluído: '{}' → Drive/{} (id: {})",
            local_path.display(),
            cloud_path,
            body["id"].as_str().unwrap_or("?")
        );

        Ok(())
    }

    /// Faz o download de um arquivo do Google Drive para o disco local.
    ///
    /// `cloud_path` aqui deve ser o **Drive File ID** do arquivo.
    async fn download(&self, cloud_file_id: &str, local_path: &Path) -> Result<()> {
        let token = self.auth.get_fresh_access_token().await?;

        let url = format!("{}/files/{}?alt=media", DRIVE_API_BASE, cloud_file_id);

        let res = self
            .client
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        let status = res.status();
        if !status.is_success() {
            return Err(anyhow!(
                "Falha ao baixar arquivo do Drive (id: {}): HTTP {}",
                cloud_file_id,
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
            cloud_file_id,
            local_path.display()
        );

        Ok(())
    }

    /// Remove um arquivo do Google Drive pelo seu File ID.
    async fn delete(&self, cloud_file_id: &str) -> Result<()> {
        let token = self.auth.get_fresh_access_token().await?;

        let res = self
            .client
            .delete(format!("{}/files/{}", DRIVE_API_BASE, cloud_file_id))
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        let status = res.status();

        // 204 No Content = sucesso; 404 = já não existe (toleramos)
        if status.is_success() || status.as_u16() == 404 {
            if status.as_u16() == 404 {
                warn!(
                    "Arquivo Drive/{} não encontrado (já foi deletado?)",
                    cloud_file_id
                );
            } else {
                info!("Arquivo Drive/{} removido.", cloud_file_id);
            }
            return Ok(());
        }

        Err(anyhow!(
            "Falha ao deletar arquivo Drive/{}: HTTP {}",
            cloud_file_id,
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
