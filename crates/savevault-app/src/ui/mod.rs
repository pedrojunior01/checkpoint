//! Módulo da interface visual nativa em Tauri 2 para testes e operação do SaveVault (DEV-UI-01).

pub mod state;
pub mod tauri_app;

pub use state::{
    DashboardSummary, DevUiEvent, DevUiEventLevel, DevUiState, DiscoveredFileView,
    DiscoverySummaryView, GameCardView, GameDetailView, LocationView, ReconciliationView,
    RestoreApiResponse, SafetyBackupGroup, SafetyBackupItem, ScanLocationResult, SnapshotView,
    SyncQueueItemView,
};
pub use tauri_app::run_tauri_app;
