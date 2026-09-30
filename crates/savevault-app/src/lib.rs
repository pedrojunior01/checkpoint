//! savevault-app
//!
//! Camada de aplicação e ponto de entrada do executável SaveVault.

pub mod cloud_recovery;
pub mod conflict;
pub mod notifications;
pub mod orchestrator;
pub mod pipeline;
pub mod reconciler;
pub mod restore;
pub mod tray;
pub mod ui;

pub use cloud_recovery::{
    list_cloud_snapshots, pull_snapshots_from_cloud, CloudSnapshotEntry, PullReport,
};
pub use conflict::{
    classify_conflict, resolve_conflict, scan_conflicts, ConflictClassification, ConflictItem,
    ConflictResolutionResult,
};
pub use orchestrator::{discover_and_persist_saves, DiscoverySyncReport};
pub use pipeline::{handle_stable_event, process_file, run_pipeline, run_pipeline_with_provider};
pub use reconciler::{
    run_reconciliation_loop, Reconciler, ReconciliationConfig, ReconciliationError,
    ReconciliationReport,
};
pub use restore::{
    restore_by_id, restore_snapshot, restore_snapshot_with_db, RestoreError, RestoreReport,
    RestoreRequest,
};
pub use ui::{start_dev_ui_server, DevUiState};
