use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Plataforma / origem do jogo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Steam,
    Gog,
    Epic,
    Dolphin,
    Pcsx2,
    Ppsspp,
    RetroArch,
    Generic,
}

impl Platform {
    /// Nome da pasta de plataforma utilizado na organização do filesystem do Google Drive:
    /// `<DriveRoot>/SaveVault/<Platform>/<Game>/`
    pub fn display_folder_name(&self) -> &'static str {
        match self {
            Platform::Steam => "Steam",
            Platform::Gog => "GOG",
            Platform::Epic => "Epic",
            Platform::Dolphin => "Dolphin",
            Platform::Pcsx2 => "PCSX2",
            Platform::Ppsspp => "PPSSPP",
            Platform::RetroArch => "RetroArch",
            Platform::Generic => "Other",
        }
    }
}

/// Nível de confiança na detecção do save (usado internamente — não exibido ao usuário como número).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

/// Status de sincronização com a cloud ou pasta do Google Drive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Pending,
    Synced,
    Failed,
    PendingOffline,
    CopiedToDriveFolder,
}

/// Tipo de operação na fila de sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncOperation {
    Upload,
    Delete,
}

// ---------------------------------------------------------------------------
// Structs de domínio
// ---------------------------------------------------------------------------

/// Representa um jogo detectado.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
    pub id: Uuid,
    pub name: String,
    pub platform: Platform,
    /// AppID do Steam, ID do GOG, etc. (opcional).
    pub platform_id: Option<String>,
    pub confidence: Confidence,
    pub created_at: DateTime<Utc>,
}

impl Game {
    pub fn new(
        name: impl Into<String>,
        platform: Platform,
        platform_id: Option<String>,
        confidence: Confidence,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            platform,
            platform_id,
            confidence,
            created_at: Utc::now(),
        }
    }
}

/// Local de save associado a um jogo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveLocation {
    pub id: Uuid,
    pub game_id: Uuid,
    /// Caminho absoluto no filesystem.
    pub path: std::path::PathBuf,
    pub location_type: SaveLocationType,
}

impl SaveLocation {
    pub fn new(
        game_id: Uuid,
        path: impl Into<std::path::PathBuf>,
        location_type: SaveLocationType,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            game_id,
            path: path.into(),
            location_type,
        }
    }

    pub fn is_directory(&self) -> bool {
        self.location_type == SaveLocationType::Directory
    }

    pub fn is_file(&self) -> bool {
        self.location_type == SaveLocationType::File
    }

    pub fn is_registry(&self) -> bool {
        self.location_type == SaveLocationType::Registry
    }

    pub fn is_glob(&self) -> bool {
        self.location_type == SaveLocationType::Glob
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveLocationType {
    File,
    Directory,
    Registry,
    Glob,
}

/// Arquivo de save rastreado.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub id: Uuid,
    pub save_location_id: Uuid,
    pub path: std::path::PathBuf,
    /// Hash BLAKE3 do conteúdo.
    pub hash: String,
    pub size_bytes: u64,
    pub modified_at: DateTime<Utc>,
}

/// Versão/backup de um arquivo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backup {
    pub id: Uuid,
    pub file_id: Uuid,
    pub version: u32,
    /// Caminho lógico no storage em cloud (ex.: "saves/game-uuid/file-uuid/v3").
    pub cloud_path: Option<String>,
    /// Identificador único remoto retornado pelo Google Drive (File ID opaco).
    pub drive_file_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub sync_status: SyncStatus,
}

/// Item na fila de sincronização.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncQueueItem {
    pub id: Uuid,
    pub backup_id: Uuid,
    pub operation: SyncOperation,
    pub status: SyncStatus,
    pub attempts: u32,
    pub created_at: DateTime<Utc>,
}
