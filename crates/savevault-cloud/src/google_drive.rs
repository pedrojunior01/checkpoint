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
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::auth::AuthService;
use crate::{CloudCatalogManifest, CloudError, CloudProvider, CloudResult, RemoteFileInfo};

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
/// - Rejeita identificador vazio;
/// - Rejeita nomes de arquivos/caminhos (ex.: extensões `.json`, `.dat`, `.sav`, `.sl2`).
pub fn validate_drive_file_id(remote_file_id: &str) -> Result<(), CloudError> {
    let trimmed = remote_file_id.trim();
    if trimmed.is_empty() {
        return Err(CloudError::InvalidRemoteId(
            "Identificador remoto não pode ser vazio".to_string(),
        ));
    }
    if remote_file_id.contains('/')
        || remote_file_id.contains('\\')
        || remote_file_id.contains('?')
        || remote_file_id.contains('#')
        || remote_file_id.contains('&')
        || remote_file_id
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(CloudError::InvalidRemoteId(format!(
            "Identificador remoto inválido: '{}'. Esperado Google Drive File ID opaco, não caminho ou parâmetros de URL.",
            remote_file_id
        )));
    }
    // Rejeita tentativas de path traversal puramente relativas como '.', '..', '...', etc.
    if remote_file_id.chars().all(|c| c == '.') {
        return Err(CloudError::InvalidRemoteId(format!(
            "Identificador remoto inválido: '{}'. Tentativa de path traversal detectada.",
            remote_file_id
        )));
    }
    // Rejeita explicitamente nomes de arquivos passados como ID
    if remote_file_id.ends_with(".json")
        || remote_file_id.ends_with(".dat")
        || remote_file_id.ends_with(".sav")
        || remote_file_id.ends_with(".sl2")
    {
        return Err(CloudError::InvalidRemoteId(format!(
            "Identificador remoto inválido: '{}'. Nome de arquivo não pode ser usado como File ID.",
            remote_file_id
        )));
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
    folder_cache: Arc<RwLock<HashMap<String, String>>>,
}

impl GoogleDriveProvider {
    /// Cria um novo provider usando as credenciais do Google Cloud Console.
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            auth: AuthService::new(client_id, client_secret),
            client: build_default_http_client(),
            api_base: DRIVE_API_BASE.to_string(),
            upload_base: DRIVE_UPLOAD_BASE.to_string(),
            folder_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Cria um provider para testes apontando para endpoints mockados.
    #[doc(hidden)]
    pub fn with_custom_endpoints(auth: AuthService, api_base: String, upload_base: String) -> Self {
        Self {
            auth,
            client: build_default_http_client(),
            api_base,
            upload_base,
            folder_cache: Arc::new(RwLock::new(HashMap::new())),
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
        Self::from_env_or_config(None, None)
    }

    /// Tenta criar um provider a partir das variáveis de ambiente ou de parâmetros de configuração.
    pub fn from_env_or_config(
        client_id: Option<String>,
        client_secret: Option<String>,
    ) -> Option<Self> {
        let cid = std::env::var("SAVEVAULT_GOOGLE_CLIENT_ID")
            .ok()
            .or(client_id)
            .filter(|s| !s.trim().is_empty())?;
        let csec = std::env::var("SAVEVAULT_GOOGLE_CLIENT_SECRET")
            .ok()
            .or(client_secret)
            .filter(|s| !s.trim().is_empty())?;
        Some(Self::new(cid, csec))
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
    async fn get_json(&self, url: &str, token: &str) -> Result<serde_json::Value, CloudError> {
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
            let error_msg = extract_drive_error(&body, &text);
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err(CloudError::Authentication(format!(
                    "HTTP {}: {}",
                    status, error_msg
                )));
            }
            if status.as_u16() == 404 {
                return Err(CloudError::FileNotFound(format!("HTTP 404: {}", error_msg)));
            }
            return Err(CloudError::Api {
                status: Some(status.as_u16()),
                message: format!("Drive API GET erro {}: {}", status, error_msg),
            });
        }

        if body.is_null() && !text.is_empty() {
            return Err(CloudError::Other(anyhow!(
                "Resposta JSON inválida da Drive API: {}",
                text
            )));
        }

        Ok(body)
    }

    /// Busca ou cria uma pasta no Drive com o nome especificado.
    /// Retorna o ID da pasta. Utiliza cache em memória para evitar round-trips HTTP desnecessários.
    async fn get_or_create_folder(
        &self,
        token: &str,
        name: &str,
        parent_id: Option<&str>,
    ) -> Result<String, CloudError> {
        let cache_key = format!("{}:{}", parent_id.unwrap_or("root"), name);
        {
            let cache = self.folder_cache.read().await;
            if let Some(id) = cache.get(&cache_key) {
                debug!("Cache hit para pasta '{}' no Drive: {}", name, id);
                return Ok(id.clone());
            }
        }

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
                    let mut cache = self.folder_cache.write().await;
                    cache.insert(cache_key, id.to_string());
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
        let body: serde_json::Value =
            serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);

        if !status.is_success() {
            let error_msg = extract_drive_error(&body, &text);
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err(CloudError::Authentication(format!(
                    "HTTP {}: {}",
                    status, error_msg
                )));
            }
            return Err(CloudError::Api {
                status: Some(status.as_u16()),
                message: format!(
                    "Falha ao criar pasta '{}': HTTP {}: {}",
                    name, status, error_msg
                ),
            });
        }

        let id = body["id"]
            .as_str()
            .ok_or_else(|| CloudError::Other(anyhow!("Resposta inválida ao criar pasta")))?
            .to_string();

        info!("Pasta '{}' criada no Drive: {}", name, id);
        {
            let mut cache = self.folder_cache.write().await;
            cache.insert(cache_key, id.clone());
        }
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
    ) -> Result<String, CloudError> {
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
    ) -> Result<reqwest::Url, CloudError> {
        validate_drive_file_id(remote_file_id)?;

        let mut url = reqwest::Url::parse(&self.api_base).map_err(|e| {
            CloudError::Other(anyhow!(
                "Base URL da API inválida '{}': {}",
                self.api_base,
                e
            ))
        })?;

        {
            let mut segments = url.path_segments_mut().map_err(|_| {
                CloudError::Other(anyhow!(
                    "Não é possível manipular segmentos de caminho na URL base: {}",
                    self.api_base
                ))
            })?;
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
    /// `cloud_path` pode ser `<game_uuid>/<file_uuid>/<filename>` ou `"savevault_manifest.json"`.
    /// Retorna o `drive_file_id` remoto (Google Drive File ID alfanumérico opaco).
    async fn upload(&self, local_path: &Path, cloud_path: &str) -> CloudResult<String> {
        let token = self.auth.get_fresh_access_token().await.map_err(|e| {
            CloudError::Authentication(format!("Falha ao renovar token do Google Drive: {}", e))
        })?;

        // Tratamento especial para o manifesto na raiz do SaveVault
        if cloud_path == "savevault_manifest.json" {
            let root_id = self
                .get_or_create_folder(&token, DRIVE_ROOT_FOLDER, None)
                .await?;
            let existing = self
                .find_file_by_name("savevault_manifest.json", Some(DRIVE_ROOT_FOLDER))
                .await?;
            let file_bytes = tokio::fs::read(local_path).await?;

            if let Some(info) = existing {
                let update_url = format!("{}/files/{}?uploadType=media", self.upload_base, info.id);
                let res = self
                    .client
                    .patch(update_url)
                    .header(AUTHORIZATION, format!("Bearer {}", token))
                    .header(
                        reqwest::header::CONTENT_TYPE,
                        "application/json; charset=UTF-8",
                    )
                    .body(file_bytes)
                    .send()
                    .await?;

                let status = res.status();
                if !status.is_success() {
                    let text = res.text().await.unwrap_or_default();
                    if status.as_u16() == 401 || status.as_u16() == 403 {
                        return Err(CloudError::Authentication(format!(
                            "HTTP {}: {}",
                            status, text
                        )));
                    }
                    return Err(CloudError::Api {
                        status: Some(status.as_u16()),
                        message: format!("Falha ao atualizar manifesto: HTTP {}: {}", status, text),
                    });
                }
                return Ok(info.id);
            } else {
                let metadata = serde_json::json!({
                    "name": "savevault_manifest.json",
                    "parents": [root_id]
                });
                let metadata_part = reqwest::multipart::Part::text(metadata.to_string())
                    .mime_str("application/json; charset=UTF-8")
                    .map_err(|e| CloudError::Other(e.into()))?;

                let file_part = reqwest::multipart::Part::bytes(file_bytes)
                    .mime_str("application/json")
                    .map_err(|e| CloudError::Other(e.into()))?
                    .file_name("savevault_manifest.json");

                let form = reqwest::multipart::Form::new()
                    .part("metadata", metadata_part)
                    .part("file", file_part);

                let res = self
                    .client
                    .post(format!(
                        "{}/files?uploadType=multipart&fields=id,name",
                        self.upload_base
                    ))
                    .header(AUTHORIZATION, format!("Bearer {}", token))
                    .multipart(form)
                    .send()
                    .await?;

                let status = res.status();
                if !status.is_success() {
                    let text = res.text().await.unwrap_or_default();
                    if status.as_u16() == 401 || status.as_u16() == 403 {
                        return Err(CloudError::Authentication(format!(
                            "HTTP {}: {}",
                            status, text
                        )));
                    }
                    return Err(CloudError::Api {
                        status: Some(status.as_u16()),
                        message: format!("Falha ao criar manifesto: HTTP {}: {}", status, text),
                    });
                }
                let body: serde_json::Value = res.json().await?;
                let id = body["id"].as_str().unwrap_or_default().to_string();
                return Ok(id);
            }
        }

        // Decompõe o cloud_path em partes
        let parts: Vec<&str> = cloud_path.splitn(3, '/').collect();
        let (game_uuid, file_uuid, filename) = match parts.as_slice() {
            [g, f, n] => (*g, *f, *n),
            _ => {
                return Err(CloudError::Other(anyhow!(
                    "cloud_path inválido: '{}'. Esperado: '<game_uuid>/<file_uuid>/<filename>'",
                    cloud_path
                )))
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

        let file_bytes = tokio::fs::read(local_path).await.map_err(CloudError::Io)?;

        let mime = mime_from_extension(local_path);

        let metadata = serde_json::json!({
            "name": filename,
            "parents": [folder_id]
        });

        let metadata_part = reqwest::multipart::Part::text(metadata.to_string())
            .mime_str("application/json; charset=UTF-8")
            .map_err(|e| CloudError::Other(e.into()))?;

        let file_part = reqwest::multipart::Part::bytes(file_bytes)
            .mime_str(mime)
            .map_err(|e| CloudError::Other(e.into()))?
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
            let body_val: serde_json::Value =
                serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
            let error_msg = extract_drive_error(&body_val, &text);
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err(CloudError::Authentication(format!(
                    "HTTP {}: {}",
                    status, error_msg
                )));
            }
            return Err(CloudError::Api {
                status: Some(status.as_u16()),
                message: format!(
                    "Falha no upload de '{}' para o Drive: HTTP {}: {}",
                    filename, status, error_msg
                ),
            });
        }

        let upload_resp: DriveUploadResponse =
            serde_json::from_str(&text).map_err(|e| CloudError::Api {
                status: Some(status.as_u16()),
                message: format!(
                    "Resposta de upload inválida do Google Drive: {}. Body: {}",
                    e, text
                ),
            })?;

        if upload_resp.id.trim().is_empty() {
            return Err(CloudError::Api {
                status: Some(status.as_u16()),
                message: "Google Drive retornou upload com campo 'id' vazio".to_string(),
            });
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
    async fn download(&self, remote_file_id: &str, local_path: &Path) -> CloudResult<()> {
        validate_drive_file_id(remote_file_id)?;

        let url = self.build_file_url(remote_file_id, Some(&[("alt", "media")]))?;
        let token = self.auth.get_fresh_access_token().await.map_err(|e| {
            CloudError::Authentication(format!("Falha ao renovar token do Google Drive: {}", e))
        })?;

        let res = self
            .client
            .get(url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        let status = res.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            let text = res.text().await.unwrap_or_default();
            return Err(CloudError::Authentication(format!(
                "HTTP {}: {}",
                status, text
            )));
        }
        if status.as_u16() == 404 {
            return Err(CloudError::FileNotFound(format!(
                "Arquivo remoto ID '{}' não encontrado (HTTP 404)",
                remote_file_id
            )));
        }
        if !status.is_success() {
            let text = res.text().await.unwrap_or_default();
            return Err(CloudError::Api {
                status: Some(status.as_u16()),
                message: format!(
                    "Falha ao baixar arquivo do Drive (id: {}): HTTP {} - {}",
                    remote_file_id, status, text
                ),
            });
        }

        let bytes = res.bytes().await?;

        let parent_dir = local_path.parent().unwrap_or_else(|| Path::new("."));
        if !parent_dir.exists() {
            tokio::fs::create_dir_all(parent_dir).await?;
        }

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let temp_file = parent_dir.join(format!(".savevault_dl_tmp_{:x}", nanos));

        if let Err(e) = tokio::fs::write(&temp_file, &bytes).await {
            let _ = tokio::fs::remove_file(&temp_file).await;
            return Err(CloudError::Io(e));
        }

        if let Err(e) = tokio::fs::rename(&temp_file, local_path).await {
            let _ = tokio::fs::remove_file(&temp_file).await;
            return Err(CloudError::Io(e));
        }

        info!(
            "Download concluído com escrita atômica: Drive/{} → '{}'",
            remote_file_id,
            local_path.display()
        );

        Ok(())
    }

    /// Remove um arquivo do Google Drive pelo seu File ID.
    async fn delete(&self, remote_file_id: &str) -> CloudResult<()> {
        validate_drive_file_id(remote_file_id)?;

        let url = self.build_file_url(remote_file_id, None)?;
        let token = self.auth.get_fresh_access_token().await.map_err(|e| {
            CloudError::Authentication(format!("Falha ao renovar token do Google Drive: {}", e))
        })?;

        let res = self
            .client
            .delete(url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        let status = res.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            let text = res.text().await.unwrap_or_default();
            return Err(CloudError::Authentication(format!(
                "HTTP {}: {}",
                status, text
            )));
        }

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

        let text = res.text().await.unwrap_or_default();
        Err(CloudError::Api {
            status: Some(status.as_u16()),
            message: format!(
                "Falha ao deletar arquivo Drive/{}: HTTP {} - {}",
                remote_file_id, status, text
            ),
        })
    }

    /// Localiza um arquivo remoto pelo nome dentro de uma pasta no Google Drive.
    async fn find_file_by_name(
        &self,
        filename: &str,
        folder: Option<&str>,
    ) -> CloudResult<Option<RemoteFileInfo>> {
        if !self.is_authenticated() {
            return Err(CloudError::Unauthenticated);
        }
        let token = self.auth.get_fresh_access_token().await.map_err(|e| {
            CloudError::Authentication(format!("Falha ao renovar token do Google Drive: {}", e))
        })?;

        let parent_clause = if let Some(folder_name) = folder {
            let folder_query = format!(
                "name='{}' and mimeType='application/vnd.google-apps.folder' and trashed=false",
                folder_name
            );
            let folder_url = format!(
                "{}/files?q={}&fields=files(id,name)&spaces=drive",
                self.api_base,
                urlencoding_query(&folder_query),
            );
            let folder_resp = self.get_json(&folder_url, &token).await?;
            let folder_id = folder_resp["files"]
                .as_array()
                .and_then(|arr| arr.first())
                .and_then(|f| f["id"].as_str())
                .map(|s| s.to_string());

            match folder_id {
                Some(id) => format!(" and '{}' in parents", id),
                None => return Ok(None),
            }
        } else {
            String::new()
        };

        let query = format!("name='{}' and trashed=false{}", filename, parent_clause);
        let url = format!(
            "{}/files?q={}&fields=files(id,name,size,trashed)&spaces=drive",
            self.api_base,
            urlencoding_query(&query),
        );

        let response = self.get_json(&url, &token).await?;
        if let Some(files) = response["files"].as_array() {
            if let Some(first) = files.first() {
                if let Some(id) = first["id"].as_str() {
                    let name = first["name"].as_str().unwrap_or(filename).to_string();
                    let size_bytes = first["size"].as_str().and_then(|s| s.parse::<u64>().ok());
                    return Ok(Some(RemoteFileInfo {
                        id: id.to_string(),
                        name,
                        size_bytes,
                    }));
                }
            }
        }
        Ok(None)
    }

    /// Faz download de um arquivo remoto especificando seu caminho/nome relativo.
    async fn download_by_path(&self, remote_path: &str, local_path: &Path) -> CloudResult<()> {
        let filename = std::path::Path::new(remote_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(remote_path);

        let folder = if remote_path.contains('/') {
            std::path::Path::new(remote_path)
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
        } else {
            Some(DRIVE_ROOT_FOLDER)
        };

        let file_info = self.find_file_by_name(filename, folder).await?;
        match file_info {
            Some(info) => self.download(&info.id, local_path).await,
            None => {
                if filename == "savevault_manifest.json" {
                    Err(CloudError::ManifestNotFound)
                } else {
                    Err(CloudError::FileNotFound(remote_path.to_string()))
                }
            }
        }
    }

    /// Carrega e faz parse do manifesto remoto `savevault_manifest.json` do Google Drive.
    async fn get_manifest(&self) -> CloudResult<Option<CloudCatalogManifest>> {
        if !self.is_authenticated() {
            return Err(CloudError::Unauthenticated);
        }

        let file_info = self
            .find_file_by_name("savevault_manifest.json", Some(DRIVE_ROOT_FOLDER))
            .await?;
        let info = match file_info {
            Some(i) => i,
            None => return Ok(None),
        };

        let tmp_dir = std::env::temp_dir();
        let tmp_path = tmp_dir.join(format!(
            "savevault_manifest_{}.json",
            uuid::Uuid::new_v4().as_simple()
        ));

        let dl_res = self.download(&info.id, &tmp_path).await;
        if let Err(e) = dl_res {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(e);
        }

        let content = tokio::fs::read_to_string(&tmp_path).await.map_err(|e| {
            let _ = std::fs::remove_file(&tmp_path);
            CloudError::Io(e)
        })?;
        let _ = tokio::fs::remove_file(&tmp_path).await;

        let manifest: CloudCatalogManifest = serde_json::from_str(&content).map_err(|e| {
            CloudError::InvalidManifest(format!(
                "JSON inválido no manifesto do Google Drive: {}",
                e
            ))
        })?;

        Ok(Some(manifest))
    }

    /// Salva ou atualiza o manifesto remoto `savevault_manifest.json` no Google Drive.
    /// Se já existir na pasta `SaveVault`, atualiza in-place para prevenir arquivos duplicados.
    async fn save_manifest(&self, manifest: &CloudCatalogManifest) -> CloudResult<()> {
        if !self.is_authenticated() {
            return Err(CloudError::Unauthenticated);
        }
        let token = self.auth.get_fresh_access_token().await.map_err(|e| {
            CloudError::Authentication(format!("Falha ao renovar token do Google Drive: {}", e))
        })?;

        let root_id = self
            .get_or_create_folder(&token, DRIVE_ROOT_FOLDER, None)
            .await?;
        let existing = self
            .find_file_by_name("savevault_manifest.json", Some(DRIVE_ROOT_FOLDER))
            .await?;

        let json_bytes = serde_json::to_string_pretty(manifest)
            .map_err(|e| CloudError::InvalidManifest(e.to_string()))?
            .into_bytes();

        if let Some(info) = existing {
            let update_url = format!("{}/files/{}?uploadType=media", self.upload_base, info.id);
            let res = self
                .client
                .patch(update_url)
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/json; charset=UTF-8",
                )
                .body(json_bytes)
                .send()
                .await?;

            let status = res.status();
            if !status.is_success() {
                let text = res.text().await.unwrap_or_default();
                if status.as_u16() == 401 || status.as_u16() == 403 {
                    return Err(CloudError::Authentication(format!(
                        "HTTP {}: {}",
                        status, text
                    )));
                }
                return Err(CloudError::Api {
                    status: Some(status.as_u16()),
                    message: format!(
                        "Falha ao atualizar manifesto existente no Drive: HTTP {}: {}",
                        status, text
                    ),
                });
            }
            info!(
                "Manifesto atualizado com sucesso no Google Drive (file_id: {})",
                info.id
            );
        } else {
            let metadata = serde_json::json!({
                "name": "savevault_manifest.json",
                "parents": [root_id]
            });

            let metadata_part = reqwest::multipart::Part::text(metadata.to_string())
                .mime_str("application/json; charset=UTF-8")
                .map_err(|e| CloudError::Other(e.into()))?;

            let file_part = reqwest::multipart::Part::bytes(json_bytes)
                .mime_str("application/json")
                .map_err(|e| CloudError::Other(e.into()))?
                .file_name("savevault_manifest.json");

            let form = reqwest::multipart::Form::new()
                .part("metadata", metadata_part)
                .part("file", file_part);

            let res = self
                .client
                .post(format!(
                    "{}/files?uploadType=multipart&fields=id,name",
                    self.upload_base
                ))
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .multipart(form)
                .send()
                .await?;

            let status = res.status();
            if !status.is_success() {
                let text = res.text().await.unwrap_or_default();
                if status.as_u16() == 401 || status.as_u16() == 403 {
                    return Err(CloudError::Authentication(format!(
                        "HTTP {}: {}",
                        status, text
                    )));
                }
                return Err(CloudError::Api {
                    status: Some(status.as_u16()),
                    message: format!(
                        "Falha ao criar manifesto no Drive: HTTP {}: {}",
                        status, text
                    ),
                });
            }
            let body_text = res.text().await.unwrap_or_default();
            let val: serde_json::Value =
                serde_json::from_str(&body_text).unwrap_or(serde_json::Value::Null);
            let new_id = val["id"].as_str().unwrap_or("unknown");
            info!(
                "Manifesto criado com sucesso no Google Drive (file_id: {})",
                new_id
            );
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extrai a mensagem de erro da resposta da Google Drive API de forma consistente.
fn extract_drive_error(body: &serde_json::Value, raw_text: &str) -> String {
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

/// Retorna o MIME type baseado na extensão do arquivo.
fn mime_from_extension(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("zip") => "application/zip",
        Some("json") => "application/json",
        Some("toml") | Some("txt") | Some("log") | Some("ini") | Some("cfg") => "text/plain",
        Some("xml") => "application/xml",
        Some("sqlite") | Some("db") | Some("sqlite3") => "application/vnd.sqlite3",
        _ => "application/octet-stream",
    }
}

/// Percent-encoding simples e eficiente para uso em parâmetros de query da URL.
/// Otimizado com buffer pré-alocado para evitar alocações por caractere.
fn urlencoding_query(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => result.push(c),
            _ => {
                use std::fmt::Write;
                let _ = write!(result, "%{:02X}", c as u32);
            }
        }
    }
    result
}
