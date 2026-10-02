//! savevault-app
//!
//! Camada de aplicação e ponto de entrada do executável SaveVault.

pub mod cloud_recovery;
pub mod conflict;
pub mod desktop;
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

/// Guarda de instância única para o Windows via Mutex de SO.
/// Mantém o Mutex ativo enquanto a estrutura permanecer em escopo (RAII).
pub struct SingleInstanceGuard {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
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
            let _ = name;
            Some(Self {})
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

/// Se uma instância já estiver em execução, localiza a sua janela principal
/// e a restaura/traz para o primeiro plano.
pub fn focus_existing_instance() {
    #[cfg(windows)]
    {
        unsafe extern "system" fn enum_windows_proc(
            hwnd: windows_sys::Win32::Foundation::HWND,
            _lparam: windows_sys::Win32::Foundation::LPARAM,
        ) -> windows_sys::Win32::Foundation::BOOL {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                GetWindowTextW, GetWindowThreadProcessId, SetForegroundWindow, ShowWindow,
                SW_RESTORE, SW_SHOW,
            };

            let current_pid = std::process::id();
            let mut window_pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut window_pid);
            if window_pid == current_pid || window_pid == 0 {
                return 1;
            }

            let mut buffer = [0u16; 512];
            let len = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
            if len > 0 {
                let title = String::from_utf16_lossy(&buffer[..len as usize]);
                if title.contains("checkpoint") || title.contains("SaveVault") {
                    ShowWindow(hwnd, SW_SHOW);
                    ShowWindow(hwnd, SW_RESTORE);
                    SetForegroundWindow(hwnd);
                    return 0; // Encontrou e focou; interrompe enumeração
                }
            }
            1
        }

        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::EnumWindows(Some(enum_windows_proc), 0);
        }
    }
    #[cfg(not(windows))]
    {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_instance_guard_mutual_exclusion() {
        let mutex_name = format!("Local\\SaveVault_Test_Mutex_{}", uuid::Uuid::new_v4());

        // Primeira aquisição deve ter sucesso
        let guard1 = SingleInstanceGuard::acquire(&mutex_name);
        assert!(guard1.is_some(), "Primeira instância deve adquirir o mutex");

        #[cfg(windows)]
        {
            // Segunda aquisição com mesmo nome de mutex deve falhar (None)
            let guard2 = SingleInstanceGuard::acquire(&mutex_name);
            assert!(
                guard2.is_none(),
                "Segunda instância concorrente deve falhar ao adquirir mutex ativo"
            );
        }

        // Ao liberar o primeiro guard, uma nova aquisição volta a funcionar
        drop(guard1);

        let guard3 = SingleInstanceGuard::acquire(&mutex_name);
        assert!(
            guard3.is_some(),
            "Após liberação da primeira guarda, nova aquisição deve suceder"
        );
    }

    #[test]
    fn test_focus_existing_instance_does_not_panic() {
        // Invoca a busca de janela existente; não deve entrar em pânico nem travar
        focus_existing_instance();
    }
}
