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
    Dolphin,
    Pcsx2,
    Ppsspp,
    RetroArch,
    Generic,
}

/// Nível de confiança na detecção do save (usado internamente — não exibido ao usuário como número).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

/// Status de sincronização com a cloud.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Pending,
    Synced,
    Failed,
    PendingOffline,
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

/// Status do mapeamento de um `SaveLocation` na máquina local (AUD-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MappingStatus {
    /// Caminho verificado e válido na máquina atual sem necessidade de remapeamento.
    #[default]
    Resolved,
    /// Caminho foi resolvido automaticamente (ex.: username traduzido ou via discovery).
    AutoResolved,
    /// Caminho foi explicitamente remapeado pelo usuário.
    UserMapped,
    /// Caminho precisa de intervenção do usuário (não encontrado e sem resolução automática segura).
    NeedsUserMapping,
    /// Caminho inválido (violação de segurança, traversal, raiz protegida de sistema).
    Invalid,
}

/// Origem do mapeamento associado a um `SaveLocation` (AUD-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MappingSource {
    /// Caminho original da máquina de origem / manifest da nuvem.
    #[default]
    Original,
    /// Resolução automática de username ou diretório padrão do sistema.
    Auto,
    /// Detector de Discovery (ex.: Ludusavi).
    Discovery,
    /// Mapeamento manual explícito do usuário.
    User,
}

/// Local de save associado a um jogo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveLocation {
    pub id: Uuid,
    pub game_id: Uuid,
    /// Caminho absoluto no filesystem local.
    pub path: std::path::PathBuf,
    pub location_type: SaveLocationType,
    /// Caminho original capturado na máquina de origem (se diferente do local ou recuperado da nuvem).
    #[serde(default)]
    pub original_path: Option<std::path::PathBuf>,
    /// Status do mapeamento para esta máquina.
    #[serde(default)]
    pub mapping_status: MappingStatus,
    /// Origem do mapeamento atual.
    #[serde(default)]
    pub mapping_source: MappingSource,
    /// Detalhe ou motivo da necessidade de remapeamento (se houver).
    #[serde(default)]
    pub resolution_reason: Option<String>,
}

impl Default for SaveLocation {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            game_id: Uuid::nil(),
            path: std::path::PathBuf::new(),
            location_type: SaveLocationType::Directory,
            original_path: None,
            mapping_status: MappingStatus::Resolved,
            mapping_source: MappingSource::Original,
            resolution_reason: None,
        }
    }
}

impl SaveLocation {
    pub fn new(
        game_id: Uuid,
        path: impl Into<std::path::PathBuf>,
        location_type: SaveLocationType,
    ) -> Self {
        let p = path.into();
        Self {
            id: Uuid::new_v4(),
            game_id,
            path: p.clone(),
            location_type,
            original_path: Some(p),
            mapping_status: MappingStatus::Resolved,
            mapping_source: MappingSource::Original,
            resolution_reason: None,
        }
    }

    pub fn with_id(
        id: Uuid,
        game_id: Uuid,
        path: impl Into<std::path::PathBuf>,
        location_type: SaveLocationType,
    ) -> Self {
        let p = path.into();
        Self {
            id,
            game_id,
            path: p.clone(),
            location_type,
            original_path: Some(p),
            mapping_status: MappingStatus::Resolved,
            mapping_source: MappingSource::Original,
            resolution_reason: None,
        }
    }

    pub fn with_mapping(
        mut self,
        original_path: impl Into<std::path::PathBuf>,
        status: MappingStatus,
        source: MappingSource,
        reason: Option<String>,
    ) -> Self {
        self.original_path = Some(original_path.into());
        self.mapping_status = status;
        self.mapping_source = source;
        self.resolution_reason = reason;
        self
    }

    pub fn needs_remapping(&self) -> bool {
        matches!(
            self.mapping_status,
            MappingStatus::NeedsUserMapping | MappingStatus::Invalid
        )
    }

    pub fn is_valid_or_resolved(&self) -> bool {
        matches!(
            self.mapping_status,
            MappingStatus::Resolved | MappingStatus::AutoResolved | MappingStatus::UserMapped
        )
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

/// Origem dos metadados visuais de um jogo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MetadataSource {
    #[default]
    None,
    Steam,
    Ludusavi,
    Manual,
}

impl std::fmt::Display for MetadataSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Steam => write!(f, "steam"),
            Self::Ludusavi => write!(f, "ludusavi"),
            Self::Manual => write!(f, "manual"),
        }
    }
}

impl std::str::FromStr for MetadataSource {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "steam" => Self::Steam,
            "ludusavi" => Self::Ludusavi,
            "manual" => Self::Manual,
            _ => Self::None,
        })
    }
}

/// Metadados visuais e de catálogo associados a um jogo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameMetadata {
    pub game_id: Uuid,
    pub steam_app_id: Option<String>,
    pub cover_url: Option<String>,
    pub source: MetadataSource,
    pub updated_at: DateTime<Utc>,
}

impl GameMetadata {
    pub fn new(
        game_id: Uuid,
        steam_app_id: Option<String>,
        cover_url: Option<String>,
        source: MetadataSource,
    ) -> Self {
        Self {
            game_id,
            steam_app_id,
            cover_url,
            source,
            updated_at: Utc::now(),
        }
    }

    pub fn empty(game_id: Uuid) -> Self {
        Self {
            game_id,
            steam_app_id: None,
            cover_url: None,
            source: MetadataSource::None,
            updated_at: Utc::now(),
        }
    }
}
