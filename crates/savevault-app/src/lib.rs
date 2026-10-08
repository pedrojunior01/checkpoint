//! savevault-app
//!
//! Camada de aplicação e ponto de entrada do executável SaveVault.

pub mod cloud_recovery;
pub mod conflict;
pub mod desktop;
pub mod metadata;
pub mod notifications;
pub mod orchestrator;
pub mod pipeline;
pub mod reconciler;
pub mod remapping;
pub mod restore;
pub mod sync_worker;
pub mod tray;
pub mod ui;

pub use cloud_recovery::{
    list_cloud_snapshots, pull_snapshots_from_cloud, CloudSnapshotEntry, PullReport,
};
pub use conflict::{
    classify_conflict, resolve_conflict, scan_conflicts, ConflictClassification, ConflictItem,
    ConflictResolutionResult,
};
pub use metadata::{
    build_steam_header_url, is_valid_steam_app_id, is_valid_steam_cover_url, GameMetadataResolver,
};
pub use orchestrator::{discover_and_persist_saves, DiscoverySyncReport};
pub use pipeline::{handle_stable_event, process_file, run_pipeline, run_pipeline_with_provider};
pub use reconciler::{
    run_reconciliation_loop, Reconciler, ReconciliationConfig, ReconciliationError,
    ReconciliationReport,
};
pub use remapping::{
    apply_remapping, list_pending_remappings, reset_remapping, resolve_path_for_game,
    validate_candidate_path, PendingMappingView, ValidationResult,
};
pub use restore::{
    restore_by_id, restore_snapshot, restore_snapshot_with_db, RestoreError, RestoreReport,
    RestoreRequest,
};
pub use sync_worker::{SyncWorker, SyncWorkerConfig, SyncWorkerError};
pub use ui::{start_dev_ui_server, DevUiState};

/// Guarda de instância única para o Windows via Mutex de SO.
/// Mantém o Mutex ativo enquanto a estrutura permanecer em escopo (RAII).
pub struct SingleInstanceGuard {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(not(windows))]
    name: String,
}

impl SingleInstanceGuard {
    /// Tenta adquirir a posse exclusiva do Mutex nomeado.
    /// Retorna `Some(guard)` se for a primeira/única instância, ou `None` se outra já estiver ativa.
    pub fn acquire(name: &str) -> Option<Self> {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
            use windows_sys::Win32::System::Threading::CreateMutexW;

            let wide_name: Vec<u16> = std::ffi::OsStr::new(name)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            let handle = unsafe { CreateMutexW(std::ptr::null(), 1, wide_name.as_ptr()) };
            if handle == 0 || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                if handle != 0 {
                    unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
                }
                return None;
            }
            Some(Self { handle })
        }
        #[cfg(not(windows))]
        {
            use std::sync::Mutex;
            static ACTIVE_INSTANCES: Mutex<Option<std::collections::HashSet<String>>> =
                Mutex::new(None);
            let mut guard = ACTIVE_INSTANCES.lock().unwrap();
            let set = guard.get_or_insert_with(std::collections::HashSet::new);
            if set.contains(name) {
                return None;
            }
            set.insert(name.to_string());
            Some(Self {
                name: name.to_string(),
            })
        }
    }
}

#[cfg(windows)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        if self.handle != 0 {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.handle);
            }
        }
    }
}

#[cfg(not(windows))]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        use std::sync::Mutex;
        static ACTIVE_INSTANCES: Mutex<Option<std::collections::HashSet<String>>> =
            Mutex::new(None);
        if let Ok(mut guard) = ACTIVE_INSTANCES.lock() {
            if let Some(set) = guard.as_mut() {
                set.remove(&self.name);
            }
        }
    }
}
