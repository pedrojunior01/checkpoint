//! Google Drive provider — OAuth 2.0, sem armazenar senha.
//! TODO v0.2: implementar fluxo OAuth e endpoints da API.

use anyhow::Result;
use tracing::info;

#[derive(Default)]
pub struct GoogleDriveProvider;

impl GoogleDriveProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl crate::CloudProvider for GoogleDriveProvider {
    async fn upload(&self, local_path: &std::path::Path, cloud_path: &str) -> Result<()> {
        info!("TODO v0.2: upload {} → {}", local_path.display(), cloud_path);
        Ok(())
    }

    async fn download(&self, cloud_path: &str, local_path: &std::path::Path) -> Result<()> {
        info!("TODO v0.2: download {} → {}", cloud_path, local_path.display());
        Ok(())
    }

    async fn delete(&self, cloud_path: &str) -> Result<()> {
        info!("TODO v0.2: delete {}", cloud_path);
        Ok(())
    }
}
