//! savevault-app
//!
//! Camada de aplicação e ponto de entrada do executável SaveVault.

pub mod orchestrator;
pub mod pipeline;
pub mod reconciler;

pub use orchestrator::{discover_and_persist_saves, DiscoverySyncReport};
pub use pipeline::{handle_stable_event, process_file, run_pipeline};
pub use reconciler::{
    run_reconciliation_loop, Reconciler, ReconciliationConfig, ReconciliationError,
    ReconciliationReport,
};
