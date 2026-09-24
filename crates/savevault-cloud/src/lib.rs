//! savevault-cloud
//!
//! Integração com provedores de cloud. MVP: Google Drive via OAuth 2.0.
//! Implementação completa no v0.2.

pub mod google_drive;

/// Interface comum para provedores de cloud.
#[async_trait::async_trait]
pub trait CloudProvider: Send + Sync {
    /// Faz upload de um arquivo local para a cloud.
    async fn upload(&self, local_path: &std::path::Path, cloud_path: &str) -> anyhow::Result<()>;

    /// Faz download de um arquivo da cloud para o local.
    async fn download(&self, cloud_path: &str, local_path: &std::path::Path) -> anyhow::Result<()>;

    /// Remove um arquivo da cloud.
    async fn delete(&self, cloud_path: &str) -> anyhow::Result<()>;
}
