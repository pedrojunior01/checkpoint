//! Backend de Armazenamento no Filesystem do Google Drive para computador.
//!
//! Trata a pasta local sincronizada pelo Google Drive para computador como um
//! filesystem Windows normal, organizando os snapshots por plataforma e jogo:
//!
//! ```text
//! <DriveRoot>/SaveVault/
//! ├── Steam/
//! │   ├── Game A/
//! │   │   ├── snapshots/
//! │   │   │   └── v1/
//! │   │   │       └── slot1.sav
//! │   │   └── metadata/
//! │   │       ├── game.json
//! │   │       └── v1_slot1.sav.json
//! │   └── Game B/
//! ├── GOG/
//! ├── Epic/
//! └── Other/
//! ```
//!
//! Todas as escritas são atômicas:
//! arquivo temporário no mesmo diretório/volume -> sync_all -> validação de tamanho e hash BLAKE3 -> rename atômico.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;
use walkdir::WalkDir;

use savevault_core::models::Platform;
use savevault_core::path::{disambiguated_game_folder_name, sanitize_game_name};

/// Status de acessibilidade do diretório raiz configurado do Drive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriveFolderStatus {
    Available { path: PathBuf },
    NotFound { path: PathBuf },
    AccessDenied { path: PathBuf, reason: String },
    Unavailable { path: PathBuf, reason: String },
}

impl DriveFolderStatus {
    pub fn is_available(&self) -> bool {
        matches!(self, DriveFolderStatus::Available { .. })
    }

    pub fn path(&self) -> &Path {
        match self {
            DriveFolderStatus::Available { path }
            | DriveFolderStatus::NotFound { path }
            | DriveFolderStatus::AccessDenied { path, .. }
            | DriveFolderStatus::Unavailable { path, .. } => path,
        }
    }
}

/// Status de disponibilidade física de um arquivo no filesystem do Drive.
///
/// Diferencia arquivos não encontrados de arquivos "online only" (Cloud Files / VFS)
/// ou com acesso negado pelo sistema operacional.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileAvailability {
    Available,
    NotFound,
    Unavailable(String),
    AccessDenied(String),
}

/// Caminhos resolvidos de diretório para um jogo no Drive filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDrivePaths {
    pub game_dir: PathBuf,
    pub snapshots_dir: PathBuf,
    pub metadata_dir: PathBuf,
    pub folder_name: String,
}

/// Parâmetros para copiar um snapshot local para o filesystem do Drive.
#[derive(Debug, Clone)]
pub struct BackupSnapshotParams {
    pub local_snapshot_path: PathBuf,
    pub platform: Platform,
    pub game_name: String,
    pub game_id: Option<Uuid>,
    pub file_id: Option<Uuid>,
    pub backup_id: Option<Uuid>,
    pub version: u32,
    pub file_name: String,
    pub expected_hash: Option<String>,
    pub expected_size: Option<u64>,
}

/// Metadados persistidos no Drive junto ao snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredSnapshotMetadata {
    pub backup_id: Option<Uuid>,
    pub file_id: Option<Uuid>,
    pub game_id: Option<Uuid>,
    pub game_name: String,
    pub platform: String,
    pub version: u32,
    pub file_name: String,
    pub size_bytes: u64,
    pub blake3_hash: String,
    pub copied_at: DateTime<Utc>,
}

/// Metadados básicos de identidade de um jogo no diretório de metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredGameIdentity {
    pub id: Option<Uuid>,
    pub name: String,
    pub platform: String,
    pub created_at: DateTime<Utc>,
}

/// Parâmetros para extração/staging de um snapshot do Drive para uso pelo restore.
#[derive(Debug, Clone)]
pub struct FetchSnapshotParams {
    pub platform: Platform,
    pub game_name: String,
    pub game_id: Option<Uuid>,
    pub version: u32,
    pub file_name: String,
    pub staging_dir: PathBuf,
    pub expected_hash: Option<String>,
}

/// Erros originados durante operações no backend de storage em filesystem.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Pasta do Google Drive indisponível: {0:?}")]
    DriveFolderUnavailable(DriveFolderStatus),

    #[error("Arquivo de snapshot '{path}' não acessível no Drive: {status:?}")]
    SnapshotAvailabilityError {
        path: PathBuf,
        status: FileAvailability,
    },

    #[error("Snapshot de versão {version} para '{file_name}' não encontrado no Drive")]
    SnapshotNotFound { version: u32, file_name: String },

    #[error("Falha de integridade: hash BLAKE3 esperado '{expected}', calculado '{actual}'")]
    IntegrityMismatch { expected: String, actual: String },

    #[error("Falha de tamanho: tamanho esperado {expected} bytes, gravado {actual} bytes")]
    SizeMismatch { expected: u64, actual: u64 },

    #[error("Erro de I/O no filesystem: {0}")]
    Io(#[from] std::io::Error),

    #[error("Erro de serialização/deserialização JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

/// Trait abstrata para provedores de armazenamento de backups (ex.: filesystem do Google Drive).
pub trait BackupStorage: Send + Sync {
    /// Valida se a pasta raiz de armazenamento está acessível.
    fn validate_folder(&self) -> DriveFolderStatus;

    /// Garante que a estrutura de diretórios para uma plataforma e jogo exista.
    fn ensure_game_structure(
        &self,
        platform: &Platform,
        game_name: &str,
        game_id: Option<Uuid>,
    ) -> Result<GameDrivePaths, StorageError>;

    /// Copia um snapshot local para o filesystem com escrita atômica e verificação de integridade.
    fn backup_snapshot(
        &self,
        params: BackupSnapshotParams,
    ) -> Result<StoredSnapshotMetadata, StorageError>;

    /// Lista os snapshots disponíveis no Drive para determinado jogo.
    fn list_snapshots(
        &self,
        platform: &Platform,
        game_name: &str,
        game_id: Option<Uuid>,
    ) -> Result<Vec<StoredSnapshotMetadata>, StorageError>;

    /// Copia e valida um snapshot do Drive para uma pasta local de staging temporário para restauração segura.
    fn fetch_snapshot_to_staging(
        &self,
        params: FetchSnapshotParams,
    ) -> Result<PathBuf, StorageError>;
}

/// Implementação do backend de storage baseada no filesystem Windows sincronizado pelo Google Drive.
#[derive(Debug, Clone)]
pub struct FilesystemBackupStorage {
    /// Diretório raiz configurado pelo usuário (ex.: `G:\Meu Drive\SaveVault` ou `C:\...\Google Drive`).
    drive_root: PathBuf,
}

impl FilesystemBackupStorage {
    /// Cria uma nova instância de storage para a pasta raiz configurada.
    pub fn new(drive_root: impl Into<PathBuf>) -> Self {
        Self {
            drive_root: drive_root.into(),
        }
    }

    /// Retorna a pasta raiz original configurada.
    pub fn drive_root(&self) -> &Path {
        &self.drive_root
    }

    /// Resolve a pasta base do SaveVault:
    /// Se a raiz configurada já terminar com `SaveVault` (case-insensitive), utiliza diretamente a raiz.
    /// Caso contrário, anexa `SaveVault`.
    pub fn resolve_base_dir(&self) -> PathBuf {
        let is_savevault = self
            .drive_root
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.eq_ignore_ascii_case("SaveVault"))
            .unwrap_or(false);

        if is_savevault {
            self.drive_root.clone()
        } else {
            self.drive_root.join("SaveVault")
        }
    }

    /// Verifica a disponibilidade física da pasta raiz configurada.
    pub fn validate_folder(&self) -> DriveFolderStatus {
        if !self.drive_root.exists() {
            return DriveFolderStatus::NotFound {
                path: self.drive_root.clone(),
            };
        }

        match fs::read_dir(&self.drive_root) {
            Ok(_) => DriveFolderStatus::Available {
                path: self.drive_root.clone(),
            },
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                DriveFolderStatus::AccessDenied {
                    path: self.drive_root.clone(),
                    reason: e.to_string(),
                }
            }
            Err(e) => DriveFolderStatus::Unavailable {
                path: self.drive_root.clone(),
                reason: e.to_string(),
            },
        }
    }

    /// Verifica o estado físico de um arquivo individual no filesystem (identificando arquivos online-only).
    pub fn check_file_availability(&self, path: &Path) -> FileAvailability {
        let meta = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return FileAvailability::NotFound
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                return FileAvailability::AccessDenied(e.to_string())
            }
            Err(e) => return FileAvailability::Unavailable(e.to_string()),
        };

        if meta.is_dir() {
            return FileAvailability::Unavailable("Caminho aponta para um diretório".into());
        }

        // No Windows, verifica atributo FILE_ATTRIBUTE_OFFLINE (0x1000)
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            let attrs = meta.file_attributes();
            if attrs & 0x1000 != 0 {
                return FileAvailability::Unavailable(
                    "Arquivo está em modo 'Online Only' e não foi baixado pelo Google Drive"
                        .to_string(),
                );
            }
        }

        // Tenta abrir o arquivo e ler 1 byte para confirmar disponibilidade imediata no filesystem
        match File::open(path) {
            Ok(mut file) => {
                if meta.len() > 0 {
                    let mut probe = [0u8; 1];
                    match file.read(&mut probe) {
                        Ok(1) => FileAvailability::Available,
                        Ok(_) => FileAvailability::Unavailable(
                            "Arquivo retornou 0 bytes inesperadamente".into(),
                        ),
                        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                            FileAvailability::AccessDenied(e.to_string())
                        }
                        Err(e) => FileAvailability::Unavailable(format!(
                            "Arquivo online-only ou inacessível no momento: {}",
                            e
                        )),
                    }
                } else {
                    FileAvailability::Available
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => FileAvailability::NotFound,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                FileAvailability::AccessDenied(e.to_string())
            }
            Err(e) => {
                FileAvailability::Unavailable(format!("Falha ao abrir arquivo no Drive: {}", e))
            }
        }
    }

    /// Resolve os caminhos do diretório do jogo com prevenção de colisões e sanitização segura.
    pub fn get_game_paths(
        &self,
        platform: &Platform,
        game_name: &str,
        game_id: Option<Uuid>,
    ) -> GameDrivePaths {
        let base_dir = self.resolve_base_dir();
        let platform_dir = base_dir.join(platform.display_folder_name());

        let clean_name = sanitize_game_name(game_name, game_id);
        let candidate_dir = platform_dir.join(&clean_name);

        // Se candidate_dir existe, verifica se o game.json pertence a outro game_id (colisão de nomes)
        let folder_name = if candidate_dir.exists() {
            let meta_game_file = candidate_dir.join("metadata").join("game.json");
            if meta_game_file.exists() {
                if let Ok(content) = fs::read_to_string(&meta_game_file) {
                    if let Ok(identity) = serde_json::from_str::<StoredGameIdentity>(&content) {
                        if let (Some(cur_id), Some(stored_id)) = (game_id, identity.id) {
                            if cur_id != stored_id {
                                // Colisão detectada com outro jogo: desambigua usando sufixo estável determinístico
                                disambiguated_game_folder_name(game_name, cur_id)
                            } else {
                                clean_name
                            }
                        } else {
                            clean_name
                        }
                    } else {
                        clean_name
                    }
                } else {
                    clean_name
                }
            } else {
                clean_name
            }
        } else {
            clean_name
        };

        let game_dir = platform_dir.join(&folder_name);
        let snapshots_dir = game_dir.join("snapshots");
        let metadata_dir = game_dir.join("metadata");

        GameDrivePaths {
            game_dir,
            snapshots_dir,
            metadata_dir,
            folder_name,
        }
    }

    /// Limpa arquivos temporários órfãos (`.*.tmp.*`) deixados na pasta do SaveVault.
    pub fn cleanup_stale_drive_staging(&self) -> Result<usize, StorageError> {
        let base_dir = self.resolve_base_dir();
        if !base_dir.exists() {
            return Ok(0);
        }

        let mut removed = 0usize;
        for entry in WalkDir::new(&base_dir).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                let name = entry.file_name().to_string_lossy();
                if name.starts_with('.') && name.contains(".tmp.") {
                    if let Ok(()) = fs::remove_file(entry.path()) {
                        tracing::debug!(
                            "Arquivo temporário órfão removido do Drive: {}",
                            entry.path().display()
                        );
                        removed += 1;
                    }
                }
            }
        }

        Ok(removed)
    }
}

impl BackupStorage for FilesystemBackupStorage {
    fn validate_folder(&self) -> DriveFolderStatus {
        FilesystemBackupStorage::validate_folder(self)
    }

    fn ensure_game_structure(
        &self,
        platform: &Platform,
        game_name: &str,
        game_id: Option<Uuid>,
    ) -> Result<GameDrivePaths, StorageError> {
        let status = self.validate_folder();
        if !status.is_available() {
            return Err(StorageError::DriveFolderUnavailable(status));
        }

        let paths = self.get_game_paths(platform, game_name, game_id);

        fs::create_dir_all(&paths.snapshots_dir)?;
        fs::create_dir_all(&paths.metadata_dir)?;

        let game_meta_file = paths.metadata_dir.join("game.json");
        if !game_meta_file.exists() {
            let identity = StoredGameIdentity {
                id: game_id,
                name: game_name.to_string(),
                platform: platform.display_folder_name().to_string(),
                created_at: Utc::now(),
            };
            let json = serde_json::to_string_pretty(&identity)?;

            let tmp_path = paths
                .metadata_dir
                .join(format!(".game.json.tmp.{}", Uuid::new_v4()));
            {
                let mut f = File::create(&tmp_path)?;
                f.write_all(json.as_bytes())?;
                f.sync_all()?;
            }
            fs::rename(&tmp_path, &game_meta_file)?;
        }

        Ok(paths)
    }

    fn backup_snapshot(
        &self,
        params: BackupSnapshotParams,
    ) -> Result<StoredSnapshotMetadata, StorageError> {
        // Valida acessibilidade da pasta raiz
        let status = self.validate_folder();
        if !status.is_available() {
            return Err(StorageError::DriveFolderUnavailable(status));
        }

        // Garante estrutura
        let paths =
            self.ensure_game_structure(&params.platform, &params.game_name, params.game_id)?;

        // Diretório de versão no Drive: <GameDir>/snapshots/v<version>/
        let version_dir = paths.snapshots_dir.join(format!("v{}", params.version));
        fs::create_dir_all(&version_dir)?;

        let target_file = version_dir.join(&params.file_name);

        // Arquivo temporário no mesmo diretório/volume para rename atômico garantido
        let tmp_file_name = format!(".{}.tmp.{}", params.file_name, Uuid::new_v4());
        let tmp_file_path = version_dir.join(&tmp_file_name);

        let mut reader = File::open(&params.local_snapshot_path)?;
        let mut writer = File::create(&tmp_file_path)?;

        let mut hasher = blake3::Hasher::new();
        let mut buffer = [0u8; 64 * 1024];
        let mut bytes_written = 0u64;

        loop {
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            writer.write_all(&buffer[..n])?;
            hasher.update(&buffer[..n]);
            bytes_written += n as u64;
        }

        // Força flush no disco
        writer.sync_all()?;
        drop(writer);

        let actual_hash = hasher.finalize().to_hex().to_string();

        // Verificação estrita de tamanho
        if let Some(expected_size) = params.expected_size {
            if bytes_written != expected_size {
                let _ = fs::remove_file(&tmp_file_path);
                return Err(StorageError::SizeMismatch {
                    expected: expected_size,
                    actual: bytes_written,
                });
            }
        }

        // Verificação estrita de integridade de hash
        if let Some(ref expected_hash) = params.expected_hash {
            if !actual_hash.eq_ignore_ascii_case(expected_hash) {
                let _ = fs::remove_file(&tmp_file_path);
                return Err(StorageError::IntegrityMismatch {
                    expected: expected_hash.clone(),
                    actual: actual_hash,
                });
            }
        }

        // Promoção atômica
        fs::rename(&tmp_file_path, &target_file)?;

        // Metadados do snapshot
        let metadata = StoredSnapshotMetadata {
            backup_id: params.backup_id,
            file_id: params.file_id,
            game_id: params.game_id,
            game_name: params.game_name.clone(),
            platform: params.platform.display_folder_name().to_string(),
            version: params.version,
            file_name: params.file_name.clone(),
            size_bytes: bytes_written,
            blake3_hash: actual_hash,
            copied_at: Utc::now(),
        };

        // Escrita atômica dos metadados JSON
        let meta_target = paths
            .metadata_dir
            .join(format!("v{}_{}.json", params.version, params.file_name));
        let meta_tmp = paths.metadata_dir.join(format!(
            ".v{}_{}.json.tmp.{}",
            params.version,
            params.file_name,
            Uuid::new_v4()
        ));

        let meta_json = serde_json::to_string_pretty(&metadata)?;
        {
            let mut mf = File::create(&meta_tmp)?;
            mf.write_all(meta_json.as_bytes())?;
            mf.sync_all()?;
        }
        fs::rename(&meta_tmp, &meta_target)?;

        tracing::info!(
            "Snapshot v{} copiado com sucesso para o Drive: {}",
            params.version,
            target_file.display()
        );

        Ok(metadata)
    }

    fn list_snapshots(
        &self,
        platform: &Platform,
        game_name: &str,
        game_id: Option<Uuid>,
    ) -> Result<Vec<StoredSnapshotMetadata>, StorageError> {
        let paths = self.get_game_paths(platform, game_name, game_id);
        if !paths.metadata_dir.exists() {
            return Ok(Vec::new());
        }

        let mut snapshots = Vec::new();
        for entry in fs::read_dir(&paths.metadata_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('v') && name.ends_with(".json") && !name.contains(".tmp.") {
                if let Ok(content) = fs::read_to_string(entry.path()) {
                    if let Ok(meta) = serde_json::from_str::<StoredSnapshotMetadata>(&content) {
                        snapshots.push(meta);
                    }
                }
            }
        }

        // Ordena pela versão decrescente (mais recente primeiro)
        snapshots.sort_by_key(|a| std::cmp::Reverse(a.version));
        Ok(snapshots)
    }

    fn fetch_snapshot_to_staging(
        &self,
        params: FetchSnapshotParams,
    ) -> Result<PathBuf, StorageError> {
        let paths = self.get_game_paths(&params.platform, &params.game_name, params.game_id);
        let drive_snapshot_file = paths
            .snapshots_dir
            .join(format!("v{}", params.version))
            .join(&params.file_name);

        let availability = self.check_file_availability(&drive_snapshot_file);
        if availability != FileAvailability::Available {
            return Err(StorageError::SnapshotAvailabilityError {
                path: drive_snapshot_file,
                status: availability,
            });
        }

        fs::create_dir_all(&params.staging_dir)?;

        let tmp_staged_name = format!(".staging_drive_{}_{}.tmp", params.version, Uuid::new_v4());
        let tmp_staged_path = params.staging_dir.join(tmp_staged_name);

        let final_staged_name = format!("drive_v{}_{}", params.version, params.file_name);
        let final_staged_path = params.staging_dir.join(final_staged_name);

        let mut reader = File::open(&drive_snapshot_file)?;
        let mut writer = File::create(&tmp_staged_path)?;

        let mut hasher = blake3::Hasher::new();
        let mut buffer = [0u8; 64 * 1024];

        loop {
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            writer.write_all(&buffer[..n])?;
            hasher.update(&buffer[..n]);
        }

        writer.sync_all()?;
        drop(writer);

        let actual_hash = hasher.finalize().to_hex().to_string();
        if let Some(ref expected) = params.expected_hash {
            if !actual_hash.eq_ignore_ascii_case(expected) {
                let _ = fs::remove_file(&tmp_staged_path);
                return Err(StorageError::IntegrityMismatch {
                    expected: expected.clone(),
                    actual: actual_hash,
                });
            }
        }

        fs::rename(&tmp_staged_path, &final_staged_path)?;
        Ok(final_staged_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_drive_folder_validation_and_resolution() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("My Google Drive");

        let storage = FilesystemBackupStorage::new(&root);
        assert_eq!(
            storage.validate_folder(),
            DriveFolderStatus::NotFound { path: root.clone() }
        );

        fs::create_dir_all(&root).unwrap();
        assert!(storage.validate_folder().is_available());

        // Se raiz não é SaveVault, anexa SaveVault
        assert_eq!(storage.resolve_base_dir(), root.join("SaveVault"));

        // Se raiz já termina com SaveVault, usa ela mesma
        let sv_root = temp.path().join("SaveVault");
        let storage_sv = FilesystemBackupStorage::new(&sv_root);
        assert_eq!(storage_sv.resolve_base_dir(), sv_root);
    }

    #[test]
    fn test_backup_and_fetch_with_integrity() {
        let temp = tempdir().unwrap();
        let drive_root = temp.path().join("DriveRoot");
        fs::create_dir_all(&drive_root).unwrap();

        let storage = FilesystemBackupStorage::new(&drive_root);

        // Cria snapshot local para backup
        let local_dir = temp.path().join("local_snapshots");
        fs::create_dir_all(&local_dir).unwrap();
        let local_file = local_dir.join("slot1.sav");
        let sample_data = b"SAVEGAME_DATA_PAYLOAD_123456789";
        fs::write(&local_file, sample_data).unwrap();

        let expected_hash = blake3::hash(sample_data).to_hex().to_string();

        let params = BackupSnapshotParams {
            local_snapshot_path: local_file.clone(),
            platform: Platform::Steam,
            game_name: "Cyberpunk 2077".into(),
            game_id: Some(Uuid::new_v4()),
            file_id: Some(Uuid::new_v4()),
            backup_id: Some(Uuid::new_v4()),
            version: 1,
            file_name: "slot1.sav".into(),
            expected_hash: Some(expected_hash.clone()),
            expected_size: Some(sample_data.len() as u64),
        };

        let meta = storage.backup_snapshot(params.clone()).unwrap();
        assert_eq!(meta.version, 1);
        assert_eq!(meta.size_bytes, sample_data.len() as u64);
        assert_eq!(meta.blake3_hash, expected_hash);

        // Verifica que arquivos aparecem em DriveRoot/SaveVault/Steam/Cyberpunk 2077/
        let expected_drive_file = drive_root
            .join("SaveVault")
            .join("Steam")
            .join("Cyberpunk 2077")
            .join("snapshots")
            .join("v1")
            .join("slot1.sav");
        assert!(expected_drive_file.exists());
        assert_eq!(fs::read(&expected_drive_file).unwrap(), sample_data);

        // Lista snapshots no Drive
        let list = storage
            .list_snapshots(&Platform::Steam, "Cyberpunk 2077", params.game_id)
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].version, 1);

        // Fetch to local staging
        let staging_dir = temp.path().join("staging");
        let fetch_params = FetchSnapshotParams {
            platform: Platform::Steam,
            game_name: "Cyberpunk 2077".into(),
            game_id: params.game_id,
            version: 1,
            file_name: "slot1.sav".into(),
            staging_dir: staging_dir.clone(),
            expected_hash: Some(expected_hash),
        };
        let staged = storage.fetch_snapshot_to_staging(fetch_params).unwrap();
        assert!(staged.exists());
        assert_eq!(fs::read(&staged).unwrap(), sample_data);

        // Limpeza de staging órfãos
        let orphan = drive_root.join("SaveVault").join(".orphan.tmp.123");
        fs::write(&orphan, b"junk").unwrap();
        let cleaned = storage.cleanup_stale_drive_staging().unwrap();
        assert_eq!(cleaned, 1);
        assert!(!orphan.exists());
    }

    #[test]
    fn test_corrupt_upload_is_rejected_and_tmp_removed() {
        let temp = tempdir().unwrap();
        let drive_root = temp.path().join("DriveRoot");
        fs::create_dir_all(&drive_root).unwrap();

        let storage = FilesystemBackupStorage::new(&drive_root);

        let local_dir = temp.path().join("local");
        fs::create_dir_all(&local_dir).unwrap();
        let local_file = local_dir.join("save.dat");
        fs::write(&local_file, b"valid_content").unwrap();

        let params = BackupSnapshotParams {
            local_snapshot_path: local_file,
            platform: Platform::Gog,
            game_name: "The Witcher 3".into(),
            game_id: Some(Uuid::new_v4()),
            file_id: Some(Uuid::new_v4()),
            backup_id: Some(Uuid::new_v4()),
            version: 1,
            file_name: "save.dat".into(),
            expected_hash: Some("incorrect_hash_value".into()),
            expected_size: Some(13),
        };

        let res = storage.backup_snapshot(params);
        assert!(res.is_err());
        match res {
            Err(StorageError::IntegrityMismatch { .. }) => {}
            other => panic!("Esperado IntegrityMismatch, recebido: {:?}", other),
        }

        // Verifica que nenhum arquivo .tmp ou final ficou órfão no diretório de destino
        let version_dir = drive_root
            .join("SaveVault")
            .join("GOG")
            .join("The Witcher 3")
            .join("snapshots")
            .join("v1");
        if version_dir.exists() {
            let files: Vec<_> = fs::read_dir(version_dir).unwrap().collect();
            assert_eq!(
                files.len(),
                0,
                "Nenhum arquivo deve sobrar após falha de hash"
            );
        }
    }
}
