//! savevault-cloud
//!
//! Integração com provedores de cloud. MVP: Google Drive via OAuth 2.0.

pub mod auth;
pub mod google_drive;
pub mod local_folder;
pub mod onedrive;
pub mod onedrive_auth;

/// Entrada de metadados de snapshot armazenado na nuvem para recuperação de desastre (F-05).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CloudManifestEntry {
    pub game_name: String,
    pub platform: String,
    pub save_location_path: String,
    pub relative_file_path: String,
    pub file_hash: String,
    pub version: u32,
    pub cloud_file_id: String,
    pub cloud_path: String,
    pub created_at: String,
}

/// Catálogo geral de backups persistido na raiz do provedor de nuvem (F-05).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, PartialEq, Eq)]
pub struct CloudCatalogManifest {
    pub schema_version: u32,
    pub updated_at: String,
    pub entries: Vec<CloudManifestEntry>,
}

/// Metadados de um arquivo localizado remotamente no provedor de nuvem.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RemoteFileInfo {
    /// Identificador remoto único (ex.: Google Drive File ID, OneDrive Item ID, caminho relativo local).
    pub id: String,
    /// Nome de exibição do arquivo remoto (ex.: "savevault_manifest.json").
    pub name: String,
    /// Tamanho em bytes, se fornecido pela API do provedor.
    pub size_bytes: Option<u64>,
}

/// Erros categorizados retornados pelas operações com provedores de nuvem.
#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    #[error("Usuário não autenticado no provedor de nuvem")]
    Unauthenticated,

    #[error("Erro de autenticação no provedor de nuvem: {0}")]
    Authentication(String),

    #[error("Manifesto de nuvem não encontrado")]
    ManifestNotFound,

    #[error("Manifesto de nuvem corrompido ou inválido: {0}")]
    InvalidManifest(String),

    #[error("Arquivo remoto não encontrado: {0}")]
    FileNotFound(String),

    #[error("Erro de rede ao comunicar com a nuvem: {0}")]
    Network(String),

    #[error("Erro da API do provedor de nuvem ({status:?}): {message}")]
    Api {
        status: Option<u16>,
        message: String,
    },

    #[error("Identificador remoto inválido: {0}")]
    InvalidRemoteId(String),

    #[error("Erro de I/O local: {0}")]
    Io(#[from] std::io::Error),

    #[error("Erro interno do provedor de nuvem: {0}")]
    Other(#[from] anyhow::Error),
}

impl From<String> for CloudError {
    fn from(s: String) -> Self {
        CloudError::Other(anyhow::anyhow!(s))
    }
}

impl From<&str> for CloudError {
    fn from(s: &str) -> Self {
        CloudError::Other(anyhow::anyhow!(s.to_string()))
    }
}

impl From<serde_json::Error> for CloudError {
    fn from(err: serde_json::Error) -> Self {
        CloudError::InvalidManifest(err.to_string())
    }
}

impl From<reqwest::Error> for CloudError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() || err.is_connect() {
            CloudError::Network(err.to_string())
        } else if let Some(status) = err.status() {
            let code = status.as_u16();
            if code == 401 || code == 403 {
                CloudError::Authentication(format!("HTTP {}: {}", code, err))
            } else if code == 404 {
                CloudError::FileNotFound(format!("HTTP 404: {}", err))
            } else {
                CloudError::Api {
                    status: Some(code),
                    message: err.to_string(),
                }
            }
        } else {
            CloudError::Network(err.to_string())
        }
    }
}

pub type CloudResult<T> = std::result::Result<T, CloudError>;

/// Interface comum para provedores de cloud.
///
/// Todas as operações são assíncronas e retornam `CloudResult<T>`.
/// O `cloud_path` semântico depende de cada provedor — consulte a documentação
/// de cada implementação.
#[async_trait::async_trait]
pub trait CloudProvider: Send + Sync {
    /// Faz upload de um snapshot local para o provedor de cloud.
    /// Retorna o identificador remoto único gerado pelo provedor (ex.: Google Drive File ID).
    async fn upload(&self, local_path: &std::path::Path, cloud_path: &str) -> CloudResult<String>;

    /// Faz download de um snapshot do provedor de cloud para o disco local usando o ID remoto único.
    async fn download(&self, remote_file_id: &str, local_path: &std::path::Path)
        -> CloudResult<()>;

    /// Remove um snapshot do provedor de cloud usando seu ID remoto único.
    async fn delete(&self, remote_file_id: &str) -> CloudResult<()>;

    /// Localiza um arquivo remoto pelo nome dentro de uma pasta opcional.
    /// Retorna `Some(RemoteFileInfo)` se encontrado, ou `None` se inexistente.
    async fn find_file_by_name(
        &self,
        filename: &str,
        folder: Option<&str>,
    ) -> CloudResult<Option<RemoteFileInfo>> {
        let _ = (filename, folder);
        Ok(None)
    }

    /// Faz download de um arquivo remoto pelo seu caminho/nome relativo.
    async fn download_by_path(
        &self,
        remote_path: &str,
        local_path: &std::path::Path,
    ) -> CloudResult<()> {
        let filename = std::path::Path::new(remote_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(remote_path);
        let folder = std::path::Path::new(remote_path)
            .parent()
            .and_then(|p| p.to_str())
            .filter(|s| !s.is_empty());
        if let Some(info) = self.find_file_by_name(filename, folder).await? {
            self.download(&info.id, local_path).await
        } else {
            Err(CloudError::FileNotFound(remote_path.to_string()))
        }
    }

    /// Carrega o manifesto de catálogo remoto da nuvem (se existir).
    /// Retorna `Ok(Some(manifest))` se encontrado e válido, `Ok(None)` se inexistente.
    /// Erros de autenticação, rede ou manifesto corrompido retornam `Err(CloudError)`.
    async fn get_manifest(&self) -> CloudResult<Option<CloudCatalogManifest>> {
        let tmp_dir = std::env::temp_dir();
        let tmp_path = tmp_dir.join(format!(
            "savevault_manifest_{}.json",
            uuid::Uuid::new_v4().as_simple()
        ));
        let download_res = self
            .download_by_path("savevault_manifest.json", &tmp_path)
            .await;
        match download_res {
            Ok(()) => {
                let content = std::fs::read_to_string(&tmp_path)?;
                let _ = std::fs::remove_file(&tmp_path);
                let manifest: CloudCatalogManifest = serde_json::from_str(&content)
                    .map_err(|e| CloudError::InvalidManifest(format!("JSON inválido: {}", e)))?;
                Ok(Some(manifest))
            }
            Err(CloudError::ManifestNotFound) | Err(CloudError::FileNotFound(_)) => {
                let _ = std::fs::remove_file(&tmp_path);
                Ok(None)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp_path);
                Err(e)
            }
        }
    }

    /// Salva ou atualiza o manifesto de catálogo remoto na nuvem.
    /// Garante atualização in-place sem duplicatas.
    async fn save_manifest(&self, manifest: &CloudCatalogManifest) -> CloudResult<()>;
}

#[async_trait::async_trait]
impl CloudProvider for Box<dyn CloudProvider> {
    async fn upload(&self, local_path: &std::path::Path, cloud_path: &str) -> CloudResult<String> {
        (**self).upload(local_path, cloud_path).await
    }

    async fn download(
        &self,
        remote_file_id: &str,
        local_path: &std::path::Path,
    ) -> CloudResult<()> {
        (**self).download(remote_file_id, local_path).await
    }

    async fn delete(&self, remote_file_id: &str) -> CloudResult<()> {
        (**self).delete(remote_file_id).await
    }

    async fn find_file_by_name(
        &self,
        filename: &str,
        folder: Option<&str>,
    ) -> CloudResult<Option<RemoteFileInfo>> {
        (**self).find_file_by_name(filename, folder).await
    }

    async fn download_by_path(
        &self,
        remote_path: &str,
        local_path: &std::path::Path,
    ) -> CloudResult<()> {
        (**self).download_by_path(remote_path, local_path).await
    }

    async fn get_manifest(&self) -> CloudResult<Option<CloudCatalogManifest>> {
        (**self).get_manifest().await
    }

    async fn save_manifest(&self, manifest: &CloudCatalogManifest) -> CloudResult<()> {
        (**self).save_manifest(manifest).await
    }
}

// Re-exports de conveniência
pub use auth::AuthService;
pub use google_drive::GoogleDriveProvider;
pub use local_folder::{detect_cloud_folders, DetectedCloudFolder, LocalFolderProvider};
pub use onedrive::OneDriveProvider;
pub use onedrive_auth::OneDriveAuthService;

/// Verifica se existem credenciais salvas do Google Drive no Windows Credential Manager.
pub fn is_google_authenticated() -> bool {
    keyring::Entry::new(auth::KEYRING_SERVICE, "google_drive_refresh_token")
        .and_then(|e| e.get_password())
        .is_ok()
}

/// Verifica se existem credenciais salvas do OneDrive no Windows Credential Manager.
pub fn is_onedrive_authenticated() -> bool {
    keyring::Entry::new(
        auth::KEYRING_SERVICE,
        onedrive_auth::KEYRING_ONEDRIVE_REFRESH,
    )
    .and_then(|e| e.get_password())
    .is_ok()
}
