//! Abstração de Save Discovery.
//!
//! Define a trait `SaveDetector` e os tipos de resultado que representam
//! como uma fonte externa descobre onde um jogo armazena seus saves.

use savevault_core::models::{Game, SaveLocation};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SaveDiscoveryError {
    #[error("Erro de I/O ao acessar fonte de saves: {0}")]
    Io(#[from] std::io::Error),

    #[error("Erro ao interpretar dados do manifest: {0}")]
    ParseError(String),

    #[error("Variável de ambiente necessária não encontrada: {0}")]
    MissingVariable(String),

    #[error("Erro interno no detector {detector}: {message}")]
    Internal {
        detector: &'static str,
        message: String,
    },
}

/// Resultado explícito de uma tentativa de descoberta de locais de save.
///
/// Diferencia claramente:
/// 1. Jogo não reconhecido pela fonte (`UnrecognizedGame`);
/// 2. Conflito entre ID da plataforma e título no manifest (`ConflictingGameIdentity`);
/// 3. Jogo reconhecido, mas caminhos ainda não existem fisicamente no disco (`KnownNotFoundOnDisk`);
/// 4. Jogo reconhecido com locais de save encontrados fisicamente (`Found`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveDiscoveryResult {
    /// O jogo não foi reconhecido por esta fonte (ex.: ausente no manifest).
    UnrecognizedGame,

    /// O jogo possui platform_id conhecido (ex.: Steam AppID), mas ele não foi encontrado no manifest,
    /// e a entrada localizada por fallback de nome possui um Store ID conflitante para essa mesma plataforma.
    ConflictingGameIdentity {
        game_name: String,
        platform: savevault_core::models::Platform,
        expected_platform_id: String,
        manifest_platform_id: String,
    },

    /// O jogo é conhecido pela fonte, mas nenhum caminho foi encontrado fisicamente no disco.
    /// Retorna os locais esperados para permitir inspeção ou monitoramento prévio.
    KnownNotFoundOnDisk {
        expected_locations: Vec<SaveLocation>,
    },

    /// O jogo é conhecido e possui locais de save identificados fisicamente no disco.
    Found(Vec<SaveLocation>),
}

impl SaveDiscoveryResult {
    /// Retorna uma fatia dos locais de save identificados (ou esperados).
    pub fn locations(&self) -> &[SaveLocation] {
        match self {
            Self::UnrecognizedGame | Self::ConflictingGameIdentity { .. } => &[],
            Self::KnownNotFoundOnDisk { expected_locations } => expected_locations,
            Self::Found(locs) => locs,
        }
    }

    /// Consome o resultado e retorna todos os locais de save descobertos
    /// (sejam encontrados no disco ou esperados pelo manifest).
    pub fn into_locations(self) -> Vec<SaveLocation> {
        match self {
            Self::UnrecognizedGame | Self::ConflictingGameIdentity { .. } => vec![],
            Self::KnownNotFoundOnDisk { expected_locations } => expected_locations,
            Self::Found(locs) => locs,
        }
    }

    /// Retorna `true` se a fonte possui conhecimento sobre este jogo (mesmo que com conflito de ID).
    pub fn is_recognized(&self) -> bool {
        !matches!(self, Self::UnrecognizedGame)
    }

    /// Retorna `true` se foi detectado um conflito seguro de identidade entre Store ID e nome.
    pub fn is_conflict(&self) -> bool {
        matches!(self, Self::ConflictingGameIdentity { .. })
    }

    /// Retorna `true` se os saves foram encontrados fisicamente no disco.
    pub fn exists_on_disk(&self) -> bool {
        matches!(self, Self::Found(_))
    }
}

/// Trait principal para fontes de descoberta de locais de save.
///
/// Implementações convertem dados de fontes externas (como o Manifest do Ludusavi,
/// perfis de emuladores ou regras manuais) para a entidade canônica `SaveLocation`.
pub trait SaveDetector: Send + Sync {
    /// Nome identificador do detector para logs e diagnósticos.
    fn name(&self) -> &'static str;

    /// Descobre locais de save para o jogo fornecido.
    fn detect_save_locations(&self, game: &Game) -> Result<SaveDiscoveryResult, SaveDiscoveryError>;
}
