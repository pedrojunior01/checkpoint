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

    /// Carrega o manifesto de catálogo remoto da nuvem (se existir).
    async fn get_manifest(&self) -> anyhow::Result<Option<CloudCatalogManifest>> {
        let tmp_dir = std::env::temp_dir();
        let tmp_path = tmp_dir.join(format!(
            "savevault_manifest_{}.json",
            uuid::Uuid::new_v4().as_simple()
        ));
        if (self.download("savevault_manifest.json", &tmp_path).await).is_err() {
            let _ = std::fs::remove_file(&tmp_path);
            return Ok(None);
        }
        let content = std::fs::read_to_string(&tmp_path)?;
        let _ = std::fs::remove_file(&tmp_path);
        let manifest: CloudCatalogManifest = serde_json::from_str(&content)?;
        Ok(Some(manifest))
    }

    /// Salva ou atualiza o manifesto de catálogo remoto na nuvem.
    async fn save_manifest(&self, manifest: &CloudCatalogManifest) -> anyhow::Result<()> {
        let tmp_dir = std::env::temp_dir();
        let tmp_path = tmp_dir.join(format!(
            "savevault_manifest_{}.json",
            uuid::Uuid::new_v4().as_simple()
        ));
        let json = serde_json::to_string_pretty(manifest)?;
        std::fs::write(&tmp_path, json)?;
        let res = self.upload(&tmp_path, "savevault_manifest.json").await;
        let _ = std::fs::remove_file(&tmp_path);
        res?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl CloudProvider for Box<dyn CloudProvider> {
    async fn upload(
        &self,
        local_path: &std::path::Path,
        cloud_path: &str,
    ) -> anyhow::Result<String> {
        (**self).upload(local_path, cloud_path).await
    }

    async fn download(
        &self,
        remote_file_id: &str,
        local_path: &std::path::Path,
    ) -> anyhow::Result<()> {
        (**self).download(remote_file_id, local_path).await
    }

    async fn delete(&self, remote_file_id: &str) -> anyhow::Result<()> {
        (**self).delete(remote_file_id).await
    }

    async fn get_manifest(&self) -> anyhow::Result<Option<CloudCatalogManifest>> {
        (**self).get_manifest().await
    }

    async fn save_manifest(&self, manifest: &CloudCatalogManifest) -> anyhow::Result<()> {
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
