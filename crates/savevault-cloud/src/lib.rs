//! savevault-cloud
//!
//! Integração com provedores de cloud. MVP: Google Drive via OAuth 2.0.

pub mod auth;
pub mod google_drive;
pub mod local_folder;
pub mod onedrive;
pub mod onedrive_auth;

/// Interface comum para provedores de cloud.
///
/// Todas as operações são assíncronas e retornam `anyhow::Result`.
/// O `cloud_path` semântico depende de cada provedor — consulte a documentação
/// de cada implementação.
#[async_trait::async_trait]
pub trait CloudProvider: Send + Sync {
    /// Faz upload de um arquivo local para o provedor de cloud.
    /// Retorna o identificador remoto único gerado pelo provedor (ex.: Google Drive File ID).
    async fn upload(
        &self,
        local_path: &std::path::Path,
        cloud_path: &str,
    ) -> anyhow::Result<String>;

    /// Faz download de um arquivo do provedor de cloud para o disco local usando o ID remoto.
    async fn download(
        &self,
        remote_file_id: &str,
        local_path: &std::path::Path,
    ) -> anyhow::Result<()>;

    /// Remove um arquivo do provedor de cloud usando o ID remoto.
    async fn delete(&self, remote_file_id: &str) -> anyhow::Result<()>;
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
