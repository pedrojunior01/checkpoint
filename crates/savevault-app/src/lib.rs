//! savevault-app
//!
//! Camada de aplicação e ponto de entrada do executável SaveVault.

pub mod orchestrator;
pub mod pipeline;
pub mod reconciler;
pub mod restore;
pub mod ui;

pub use orchestrator::{discover_and_persist_saves, DiscoverySyncReport};
pub use pipeline::{handle_stable_event, process_file, run_pipeline};
pub use reconciler::{
    run_reconciliation_loop, Reconciler, ReconciliationConfig, ReconciliationError,
    ReconciliationReport,
};
pub use restore::{
    restore_by_id, restore_snapshot, restore_snapshot_with_db, RestoreError, RestoreReport,
    RestoreRequest,
};
pub use ui::{start_dev_ui_server, DevUiState};
