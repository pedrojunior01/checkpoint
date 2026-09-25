//! Modelos de eventos do sistema de monitoramento de arquivos.

use std::path::PathBuf;
use std::time::SystemTime;
use uuid::Uuid;

/// Tipo de evento bruto recebido do watcher do sistema operacional.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawFileEventKind {
    Created,
    Modified,
    Removed,
    Renamed,
}

/// Evento bruto pré-debounce emitido pelo `FileWatcher`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFileEvent {
    pub path: PathBuf,
    pub save_location_id: Uuid,
    pub kind: RawFileEventKind,
}

/// Evento consolidado após período de debounce e coalescing por caminho.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoalescedFileEvent {
    pub path: PathBuf,
    pub save_location_id: Uuid,
    pub kind: RawFileEventKind,
    pub generation: u64,
}

/// Tipo de evento após validação e estabilização de arquivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StableFileEventKind {
    Created,
    Modified,
    Removed,
}

/// Evento final emitido pelo pipeline de monitoramento:
/// Garante que o arquivo foi detectado, coalescido, estabilizado e está pronto
/// para processamento seguro (ex.: criação de snapshot na etapa futura).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StableFileEvent {
    pub path: PathBuf,
    pub save_location_id: Uuid,
    pub kind: StableFileEventKind,
    pub size_bytes: u64,
    pub observed_at: SystemTime,
}
