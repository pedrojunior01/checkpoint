//! Provedor de Nuvem via Pasta Sincronizada Local (OneDrive, Google Drive Desktop, Dropbox, etc.).
//!
//! Princípio de Funcionamento:
//! - Permite sincronização imediata sem OAuth 2.0, Client ID ou chaves de API.
//! - Os arquivos de snapshot são mantidos em uma pasta gerenciada por um cliente de nuvem
//!   instalado no sistema (ex.: %OneDrive%\SaveVault, G:\Meu Drive\SaveVault).
//! - O aplicativo oficial da Microsoft/Google cuida do upload e download em background.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::{CloudCatalogManifest, CloudError, CloudProvider, CloudResult, RemoteFileInfo};

/// Informações sobre uma pasta de nuvem detectada no computador do usuário.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedCloudFolder {
    /// Nome amigável do serviço (ex.: "Microsoft OneDrive", "Google Drive").
    pub name: String,
    /// Caminho da pasta raiz sincronizada da nuvem.
    pub sync_folder: PathBuf,
    /// Caminho recomendado para os backups do SaveVault dentro dessa nuvem.
    pub savevault_folder: PathBuf,
    /// Tipo de provedor: "onedrive", "google_drive", "dropbox", "icloud", "custom".
    pub provider_type: String,
    /// Se a pasta está acessível no disco.
    pub is_available: bool,
}

/// Detecta pastas sincronizadas de serviços de nuvem conhecidos no Windows.
pub fn detect_cloud_folders() -> Vec<DetectedCloudFolder> {
    let mut detected = Vec::new();
    let user_profile = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\Default".into());
    let user_path = PathBuf::from(&user_profile);

    // 1. Microsoft OneDrive
    let mut onedrive_candidates = Vec::new();
    if let Ok(p) = std::env::var("OneDrive") {
        onedrive_candidates.push(PathBuf::from(p));
    }
    if let Ok(p) = std::env::var("OneDriveConsumer") {
        onedrive_candidates.push(PathBuf::from(p));
    }
    if let Ok(p) = std::env::var("OneDriveCommercial") {
        onedrive_candidates.push(PathBuf::from(p));
    }
    onedrive_candidates.push(user_path.join("OneDrive"));

    for candidate in onedrive_candidates {
        if candidate.exists() && candidate.is_dir() {
            detected.push(DetectedCloudFolder {
                name: "Microsoft OneDrive".to_string(),
                savevault_folder: resolve_cloud_app_folder(&candidate),
                sync_folder: candidate,
                provider_type: "onedrive".to_string(),
                is_available: true,
            });
            break;
        }
    }

    // 2. Google Drive para Desktop (G:\Meu Drive, G:\My Drive ou pasta em USERPROFILE)
    let gdrive_candidates = vec![
        PathBuf::from(r"G:\Meu Drive"),
        PathBuf::from(r"G:\My Drive"),
        PathBuf::from(r"H:\Meu Drive"),
        PathBuf::from(r"H:\My Drive"),
        user_path.join("Google Drive"),
        user_path.join("Google Drive").join("Meu Drive"),
    ];

    for candidate in gdrive_candidates {
        if candidate.exists() && candidate.is_dir() {
            detected.push(DetectedCloudFolder {
                name: "Google Drive".to_string(),
                savevault_folder: resolve_cloud_app_folder(&candidate),
                sync_folder: candidate,
                provider_type: "google_drive".to_string(),
                is_available: true,
            });
            break;
        }
    }

    // 3. Dropbox
    let dropbox_path = user_path.join("Dropbox");
    if dropbox_path.exists() && dropbox_path.is_dir() {
        detected.push(DetectedCloudFolder {
            name: "Dropbox".to_string(),
            savevault_folder: resolve_cloud_app_folder(&dropbox_path),
            sync_folder: dropbox_path,
            provider_type: "dropbox".to_string(),
            is_available: true,
        });
    }

    // 4. iCloud Drive
    let icloud_path = user_path.join("iCloudDrive");
    if icloud_path.exists() && icloud_path.is_dir() {
        detected.push(DetectedCloudFolder {
            name: "iCloud Drive".to_string(),
            savevault_folder: resolve_cloud_app_folder(&icloud_path),
            sync_folder: icloud_path,
            provider_type: "icloud".to_string(),
            is_available: true,
        });
    }

    detected
}

/// Resolve a pasta de backup dentro da raiz de sincronização:
/// usa `checkpoint` se existir, ou fallback para `SaveVault` se o usuário já tiver saves anteriores.
fn resolve_cloud_app_folder(root: &Path) -> PathBuf {
    let checkpoint = root.join("checkpoint");
    let legacy = root.join("SaveVault");
    if checkpoint.exists() {
        checkpoint
    } else if legacy.exists() {
        legacy
    } else {
        checkpoint
    }
}

/// Provedor que sincroniza backups através de uma pasta local gerenciada por um serviço de nuvem.
pub struct LocalFolderProvider {
    sync_root: PathBuf,
}

impl LocalFolderProvider {
    /// Cria uma nova instância apontando para o diretório de destino da nuvem.
    pub fn new(sync_root: PathBuf) -> Self {
        Self { sync_root }
    }

    /// Retorna o diretório raiz configurado para sincronização.
    pub fn sync_root(&self) -> &Path {
        &self.sync_root
    }

    /// Limpa e sanitiza um caminho relativo para prevenir path traversal.
    fn sanitize_rel_path(path_str: &str) -> CloudResult<PathBuf> {
        let clean = path_str.trim().replace('\\', "/");
        let clean = clean.trim_start_matches('/');

        if clean.is_empty() {
            return Err(CloudError::InvalidRemoteId(
                "Caminho relativo de nuvem não pode ser vazio".to_string(),
            ));
        }

        let rel = PathBuf::from(&clean);
        for comp in rel.components() {
            match comp {
                std::path::Component::ParentDir => {
                    return Err(CloudError::InvalidRemoteId(format!(
                        "Tentativa de path traversal detectada: {}",
                        path_str
                    )));
                }
                std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                    return Err(CloudError::InvalidRemoteId(format!(
                        "Caminho absoluto não permitido: {}",
                        path_str
                    )));
                }
                _ => {}
            }
        }

        Ok(rel)
    }
}

#[async_trait::async_trait]
impl CloudProvider for LocalFolderProvider {
    async fn upload(&self, local_path: &Path, cloud_path: &str) -> CloudResult<String> {
        let rel_path = Self::sanitize_rel_path(cloud_path)?;
        let target_path = self.sync_root.join(&rel_path);

        // Se o arquivo local já for exatamente o mesmo destino, não precisa copiar
        if let (Ok(can_local), Ok(can_target)) =
            (local_path.canonicalize(), target_path.canonicalize())
        {
            if can_local == can_target {
                return Ok(rel_path.to_string_lossy().replace('\\', "/"));
            }
        }

        if let Some(parent) = target_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Cópia atômica via arquivo temporário
        let tmp_path =
            target_path.with_extension(format!("tmp_{}", uuid::Uuid::new_v4().as_simple()));
        tokio::fs::copy(local_path, &tmp_path).await?;
        tokio::fs::rename(&tmp_path, &target_path).await?;

        Ok(rel_path.to_string_lossy().replace('\\', "/"))
    }

    async fn download(&self, remote_file_id: &str, local_path: &Path) -> CloudResult<()> {
        let rel_path = Self::sanitize_rel_path(remote_file_id)?;
        let source_path = self.sync_root.join(&rel_path);

        if !source_path.exists() {
            return Err(CloudError::FileNotFound(format!(
                "Arquivo remoto '{}' não encontrado na pasta de nuvem '{}'",
                remote_file_id,
                self.sync_root.display()
            )));
        }

        if let (Ok(can_src), Ok(can_dest)) = (source_path.canonicalize(), local_path.canonicalize())
        {
            if can_src == can_dest {
                return Ok(());
            }
        }

        if let Some(parent) = local_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let tmp_path =
            local_path.with_extension(format!("tmp_dl_{}", uuid::Uuid::new_v4().as_simple()));
        tokio::fs::copy(&source_path, &tmp_path).await?;
        tokio::fs::rename(&tmp_path, local_path).await?;

        Ok(())
    }

    async fn delete(&self, remote_file_id: &str) -> CloudResult<()> {
        let rel_path = Self::sanitize_rel_path(remote_file_id)?;
        let target_path = self.sync_root.join(&rel_path);

        if target_path.exists() {
            let _ = tokio::fs::remove_file(&target_path).await;
        }

        Ok(())
    }

    async fn find_file_by_name(
        &self,
        filename: &str,
        folder: Option<&str>,
    ) -> CloudResult<Option<RemoteFileInfo>> {
        let target_dir = match folder {
            Some(f) if !f.trim().is_empty() => {
                let rel = Self::sanitize_rel_path(f)?;
                self.sync_root.join(rel)
            }
            _ => self.sync_root.clone(),
        };

        let file_path = target_dir.join(filename);
        if file_path.exists() && file_path.is_file() {
            let rel = file_path
                .strip_prefix(&self.sync_root)
                .map_err(|e| CloudError::Other(e.into()))?;
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            let size = tokio::fs::metadata(&file_path).await.ok().map(|m| m.len());
            Ok(Some(RemoteFileInfo {
                id: rel_str,
                name: filename.to_string(),
                size_bytes: size,
            }))
        } else {
            Ok(None)
        }
    }

    async fn download_by_path(&self, remote_path: &str, local_path: &Path) -> CloudResult<()> {
        self.download(remote_path, local_path).await
    }

    async fn get_manifest(&self) -> CloudResult<Option<CloudCatalogManifest>> {
        let manifest_path = self.sync_root.join("savevault_manifest.json");
        if !manifest_path.exists() {
            return Ok(None);
        }

        let content = tokio::fs::read_to_string(&manifest_path).await?;
        let manifest: CloudCatalogManifest = serde_json::from_str(&content)
            .map_err(|e| CloudError::InvalidManifest(format!("JSON inválido: {}", e)))?;
        Ok(Some(manifest))
    }

    async fn save_manifest(&self, manifest: &CloudCatalogManifest) -> CloudResult<()> {
        if !self.sync_root.exists() {
            tokio::fs::create_dir_all(&self.sync_root).await?;
        }
        let manifest_path = self.sync_root.join("savevault_manifest.json");
        let content = serde_json::to_string_pretty(manifest)
            .map_err(|e| CloudError::InvalidManifest(e.to_string()))?;

        let tmp_path =
            manifest_path.with_extension(format!("tmp_{}", uuid::Uuid::new_v4().as_simple()));
        tokio::fs::write(&tmp_path, content).await?;
        tokio::fs::rename(&tmp_path, &manifest_path).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_local_folder_upload_download_delete() {
        let temp = tempdir().unwrap();
        let sync_root = temp.path().join("OneDrive_Sync");
        let provider = LocalFolderProvider::new(sync_root.clone());

        let source_dir = temp.path().join("local_source");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source_file = source_dir.join("save.dat");
        std::fs::write(&source_file, b"meu savegame de teste").unwrap();

        // 1. Upload
        let remote_id = provider
            .upload(&source_file, "Elden Ring/v1/save.dat")
            .await
            .expect("Upload falhou");
        assert_eq!(remote_id, "Elden Ring/v1/save.dat");

        let dest_on_cloud = sync_root.join("Elden Ring/v1/save.dat");
        assert!(dest_on_cloud.exists());
        assert_eq!(
            std::fs::read(&dest_on_cloud).unwrap(),
            b"meu savegame de teste"
        );

        // 2. Download para outro local
        let restore_target = temp.path().join("restored").join("save.dat");
        provider
            .download(&remote_id, &restore_target)
            .await
            .expect("Download falhou");
        assert_eq!(
            std::fs::read(&restore_target).unwrap(),
            b"meu savegame de teste"
        );

        // 3. Find file by name
        let found = provider
            .find_file_by_name("save.dat", Some("Elden Ring/v1"))
            .await
            .unwrap()
            .expect("Deveria encontrar save.dat");
        assert_eq!(found.id, "Elden Ring/v1/save.dat");

        // 4. Manifest test
        let manifest = CloudCatalogManifest {
            schema_version: 1,
            ..Default::default()
        };
        provider.save_manifest(&manifest).await.unwrap();

        let loaded = provider
            .get_manifest()
            .await
            .unwrap()
            .expect("Deveria carregar manifest");
        assert_eq!(loaded.schema_version, 1);

        // 5. Delete
        provider.delete(&remote_id).await.expect("Delete falhou");
        assert!(!dest_on_cloud.exists());
    }

    #[test]
    fn test_detect_cloud_folders_does_not_panic() {
        let detected = detect_cloud_folders();
        // Não entra em pânico e devolve lista válida
        for d in &detected {
            assert!(!d.name.is_empty());
            assert!(!d.provider_type.is_empty());
        }
    }
}
