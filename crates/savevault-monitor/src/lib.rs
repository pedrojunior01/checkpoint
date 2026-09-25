//! savevault-monitor
//!
//! Pipeline resiliente de monitoramento de arquivos para o SaveVault (Etapa 3).
//!
//! Transforma `SaveLocation`s configurados em uma fonte confiável de eventos de arquivos
//! (`StableFileEvent`) para alimentar com segurança as etapas subsequentes do sistema.
//!
//! Arquitetura:
//! ```text
//! SaveLocation
//!     ↓
//! SaveScanner
//!     ↓
//! SaveWatcherService
//!     ↓
//! FileDebouncer (sem bloqueio de event loop)
//!     ↓
//! StabilityChecker (estabilidade real, detecção de lock / sharing violation e backoff)
//!     ↓
//! StableFileEvent
//! ```

pub mod debouncer;
pub mod events;
pub mod file_access;
pub mod scanner;
pub mod stability;
pub mod watcher;

#[cfg(test)]
pub mod tests;

// Re-exports principais
pub use debouncer::{DebounceConfig, FileDebouncer, PendingPathState};
pub use events::{
    CoalescedFileEvent, RawFileEvent, RawFileEventKind, StableFileEvent, StableFileEventKind,
};
pub use file_access::{FileAccess, FileAccessError, FileMetadata, OsFileAccess};
pub use scanner::{
    split_glob_base_and_pattern, wildcard_match, DiscoveredFile, SaveScanner, ScanError,
};
pub use stability::{StabilityChecker, StabilityConfig, StabilityResult};
pub use watcher::{
    match_path_to_locations, resolve_watch_directories, SaveWatcherConfig, SaveWatcherService,
    WatcherError,
};
