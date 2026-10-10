//! savevault-app
//!
//! Camada de aplicação e ponto de entrada do executável SaveVault.

pub mod notifications;
pub mod orchestrator;
pub mod pipeline;
pub mod reconciler;
pub mod restore;
pub mod sync_worker;
pub mod ui;

pub use orchestrator::{discover_and_persist_saves, DiscoverySyncReport};
pub use pipeline::{handle_stable_event, process_file, run_pipeline};
pub use sync_worker::{SyncWorker, SyncWorkerConfig};
pub use reconciler::{
    run_reconciliation_loop, Reconciler, ReconciliationConfig, ReconciliationError,
    ReconciliationReport,
};
pub use restore::{
    restore_by_id, restore_snapshot, restore_snapshot_with_db, RestoreError, RestoreReport,
    RestoreRequest,
};
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
                None
            } else {
                set.insert(name.to_string());
                Some(Self { name: name.to_string() })
            }
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
