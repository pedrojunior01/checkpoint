//! savevault-cloud
//!
//! Integração com provedores de cloud. MVP: Google Drive via OAuth 2.0.

pub mod auth;
pub mod google_drive;

/// Interface comum para provedores de cloud.
///
/// Todas as operações são assíncronas e retornam `anyhow::Result`.
/// O `cloud_path` semântico depende de cada provedor — consulte a documentação
/// de cada implementação.
#[async_trait::async_trait]
pub trait CloudProvider: Send + Sync {
    /// Faz upload de um arquivo local para o provedor de cloud.
    async fn upload(&self, local_path: &std::path::Path, cloud_path: &str) -> anyhow::Result<()>;

    /// Faz download de um arquivo do provedor de cloud para o disco local.
    async fn download(
        &self,
        cloud_path: &str,
        local_path: &std::path::Path,
    ) -> anyhow::Result<()>;

    /// Remove um arquivo do provedor de cloud.
    async fn delete(&self, cloud_path: &str) -> anyhow::Result<()>;
}

// Re-exports de conveniência
pub use auth::AuthService;
pub use google_drive::GoogleDriveProvider;
