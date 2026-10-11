use std::path::PathBuf;
use std::sync::Arc;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{TrayIconBuilder, TrayIconEvent},
    Manager, State, WebviewWindow,
};
use uuid::Uuid;

use savevault_core::config::Config;
use crate::ui::state::{
    CloudConfigView, DashboardSummary, DevUiState, ExportResult, LocationView, OptimizationResult,
    ReconciliationView, RestoreApiResponse, SafetyBackupGroup, SnapshotView,
};

// ── Utilitário de Limpeza de Memória (Working Set Trim) ───────────────────────

/// Solicita ao Gerenciador de Memória do Windows o descarte imediato de
/// páginas de memória ociosas do processo, devolvendo RAM para o sistema.
#[cfg(windows)]
pub fn trim_process_memory() {
    use std::collections::HashSet;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentProcessId, OpenProcess, SetProcessWorkingSetSize,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
    };

    unsafe {
        // 1. Esvazia o Working Set do processo principal Rust
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);

        let my_pid = GetCurrentProcessId();
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE || snapshot == 0 {
            return;
        }

        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        let mut child_pids = HashSet::new();
        let mut all_processes = Vec::new();

        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                all_processes.push((entry.th32ProcessID, entry.th32ParentProcessID));
                if entry.th32ParentProcessID == my_pid {
                    child_pids.insert(entry.th32ProcessID);
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);

        // Expande para processos netos e bisnetos do WebView2 (renderer, gpu, storage, network)
        let mut added = true;
        while added {
            added = false;
            for &(pid, ppid) in &all_processes {
                if child_pids.contains(&ppid) && child_pids.insert(pid) {
                    added = true;
                }
            }
        }

        // 2. Esvazia o Working Set de CADA subprocesso do WebView2 no Windows
        for child_pid in child_pids {
            let handle = OpenProcess(
                PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                child_pid,
            );
            if handle != 0 && handle != INVALID_HANDLE_VALUE {
                SetProcessWorkingSetSize(handle, usize::MAX, usize::MAX);
                CloseHandle(handle);
            }
        }
    }
}

#[cfg(not(windows))]
pub fn trim_process_memory() {}

// ── Window Control Commands ──────────────────────────────────────────────────

#[tauri::command]
pub fn window_minimize(window: WebviewWindow) -> Result<(), String> {
    window.minimize().map_err(|e| e.to_string())?;
    trim_process_memory();
    Ok(())
}

#[tauri::command]
pub fn window_toggle_maximize(window: WebviewWindow) -> Result<(), String> {
    if window.is_maximized().unwrap_or(false) {
        window.unmaximize().map_err(|e| e.to_string())?;
    } else {
        window.maximize().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn window_close(window: WebviewWindow) -> Result<(), String> {
    let _ = window.hide();
    trim_process_memory();
    Ok(())
}

// ── Configuration & Explorer Commands ────────────────────────────────────────

pub use crate::ui::state::ConfigDto;

fn config_path() -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    let checkpoint_cfg = PathBuf::from(&appdata).join("checkpoint").join("config.toml");
    if checkpoint_cfg.exists() {
        return checkpoint_cfg;
    }
    let legacy_cfg = PathBuf::from(&appdata).join("SaveVault").join("config.toml");
    if legacy_cfg.exists() {
        return legacy_cfg;
    }
    checkpoint_cfg
}

#[tauri::command]
pub fn get_config() -> Result<ConfigDto, String> {
    let path = config_path();
    let cfg = Config::load(&path).map_err(|e| e.to_string())?;
    let autostart = savevault_core::autostart::is_enabled().unwrap_or(false);
    let cloud_sync_path = cfg
        .cloud
        .local_sync_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());
    Ok(ConfigDto {
        max_versions: cfg.general.max_versions,
        debounce_ms: cfg.general.debounce_ms,
        reconciliation_interval_secs: cfg.general.reconciliation_interval_secs,
        log_level: cfg.general.log_level,
        backup_dir: cfg.paths.backup_dir.to_string_lossy().to_string(),
        database_path: cfg.paths.database_path.to_string_lossy().to_string(),
        autostart,
        cloud_sync_path,
    })
}

#[tauri::command]
pub async fn save_config(
    state: State<'_, Arc<DevUiState>>,
    new_cfg: ConfigDto,
) -> Result<String, String> {
    state.update_config(new_cfg).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_path_in_explorer(path: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        let p = std::path::Path::new(&path);
        let target = if p.is_file() {
            p.parent().unwrap_or(p)
        } else {
            p
        };
        if !target.exists() {
            let _ = std::fs::create_dir_all(target);
        }
        std::process::Command::new("explorer.exe")
            .arg(target)
            .spawn()
            .map_err(|e| format!("Não foi possível abrir o Explorer: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
pub fn open_folder(path: String) -> Result<(), String> {
    open_path_in_explorer(path)
}

// ── Data & Action Commands ───────────────────────────────────────────────────

#[tauri::command]
pub async fn get_dashboard_status(
    state: State<'_, Arc<DevUiState>>,
) -> Result<DashboardSummary, String> {
    state.get_dashboard().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_locations(state: State<'_, Arc<DevUiState>>) -> Result<Vec<LocationView>, String> {
    state.get_locations().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_snapshots(
    state: State<'_, Arc<DevUiState>>,
) -> Result<Vec<SnapshotView>, String> {
    state.get_snapshots().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_safety_backups(
    state: State<'_, Arc<DevUiState>>,
) -> Result<Vec<SafetyBackupGroup>, String> {
    state.get_safety_backups().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_reconciliation_status(
    state: State<'_, Arc<DevUiState>>,
) -> Result<ReconciliationView, String> {
    state
        .get_reconciliation_status()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn trigger_reconcile(state: State<'_, Arc<DevUiState>>) -> Result<String, String> {
    state
        .trigger_reconcile()
        .await
        .map(|r| format!("{} arquivo(s) modificado(s)", r.files_changed))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn restore_snapshot(
    state: State<'_, Arc<DevUiState>>,
    snapshot_id: String,
) -> Result<RestoreApiResponse, String> {
    let id = Uuid::parse_str(&snapshot_id).map_err(|e| e.to_string())?;
    Ok(state.trigger_restore(id, None).await)
}

#[tauri::command]
pub async fn add_game(
    state: State<'_, Arc<DevUiState>>,
    name: String,
    path: String,
) -> Result<LocationView, String> {
    state.add_game(&name, &path).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn optimize_database(
    state: State<'_, Arc<DevUiState>>,
) -> Result<OptimizationResult, String> {
    state.optimize_database().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn export_backup_zip(
    state: State<'_, Arc<DevUiState>>,
) -> Result<ExportResult, String> {
    state.export_backup_zip().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_cloud_config(
    state: State<'_, Arc<DevUiState>>,
) -> Result<CloudConfigView, String> {
    state.get_cloud_config().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_cloud_config(
    state: State<'_, Arc<DevUiState>>,
    provider: Option<String>,
    local_sync_path: Option<String>,
    enabled: Option<bool>,
) -> Result<CloudConfigView, String> {
    state
        .update_cloud_config(provider, local_sync_path, enabled)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn resolve_game_cover(
    state: State<'_, Arc<DevUiState>>,
    app_id: String,
) -> Result<Option<String>, String> {
    Ok(state.resolve_steam_cover(&app_id).await)
}

// ── App Setup ────────────────────────────────────────────────────────────────

/// Executa a interface gráfica Tauri integrada ao processo residente.
/// Deve ser chamada na thread principal do processo.
pub fn run_tauri_app(state: Arc<DevUiState>) -> anyhow::Result<()> {
    // Otimizações do Chromium para eliminar subprocessos pesados de GPU e desativar módulos não utilizados
    #[cfg(windows)]
    std::env::set_var(
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "--disable-gpu --disable-gpu-compositing --disable-gpu-rasterization --in-process-gpu --disable-software-rasterizer --renderer-process-limit=1 --js-flags=\"--max-old-space-size=32 --optimize-for-size --lite-mode\" --disable-features=Translate,OptimizationHints,MediaRouter,CalculateNativeWinOcclusion,InterestFeedContentSuggestions,AudioServiceOutOfProcess --enable-features=NetworkServiceInProcess --disable-extensions --disable-component-update --disable-background-networking --disable-speech-api --disable-sync --disable-breakpad --disable-default-apps --disable-domain-reliability --no-pings --mute-audio --disable-dev-shm-usage"
    );

    tauri::Builder::default()
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            window_minimize,
            window_toggle_maximize,
            window_close,
            get_config,
            save_config,
            open_path_in_explorer,
            open_folder,
            get_dashboard_status,
            get_locations,
            get_snapshots,
            get_safety_backups,
            get_reconciliation_status,
            trigger_reconcile,
            restore_snapshot,
            add_game,
            optimize_database,
            export_backup_zip,
            get_cloud_config,
            update_cloud_config,
            resolve_game_cover,
        ])
        .setup(|app| {
            // Mostra janela e dá foco
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
            }

            // Configura System Tray
            let show_item = MenuItemBuilder::with_id("show", "Abrir checkpoint").build(app)?;
            let hide_item = MenuItemBuilder::with_id("hide", "Ocultar").build(app)?;
            let reconcile_item =
                MenuItemBuilder::with_id("reconcile", "Reconciliar Agora").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "Sair do checkpoint").build(app)?;

            let menu = MenuBuilder::new(app)
                .item(&show_item)
                .item(&hide_item)
                .separator()
                .item(&reconcile_item)
                .separator()
                .item(&quit_item)
                .build()?;

            if let Some(ic) = app.default_window_icon().cloned() {
                TrayIconBuilder::new()
                    .icon(ic)
                    .tooltip("checkpoint — Proteção de Saves")
                    .menu(&menu)
                    .on_menu_event(|app, event| match event.id().as_ref() {
                        "show" => {
                            if let Some(win) = app.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.unminimize();
                                let _ = win.set_focus();
                            }
                        }
                        "hide" => {
                            if let Some(win) = app.get_webview_window("main") {
                                let _ = win.hide();
                                trim_process_memory();
                            }
                        }
                        "reconcile" => {
                            let state = app.state::<Arc<DevUiState>>();
                            let state_clone = Arc::clone(&state);
                            tauri::async_runtime::spawn(async move {
                                let _ = state_clone.trigger_reconcile().await;
                                trim_process_memory();
                            });
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click { .. } = event {
                            let app = tray.app_handle();
                            if let Some(win) = app.get_webview_window("main") {
                                if win.is_visible().unwrap_or(false) {
                                    let _ = win.hide();
                                    trim_process_memory();
                                } else {
                                    let _ = win.show();
                                    let _ = win.unminimize();
                                    let _ = win.set_focus();
                                }
                            }
                        }
                    })
                    .build(app)?;
            }

            // Limpeza inicial retardada (após renderização do WebView2) e ciclo periódico em background
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                trim_process_memory();

                let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
                loop {
                    interval.tick().await;
                    trim_process_memory();
                }
            });

            trim_process_memory();

            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = window.hide();
                    trim_process_memory();
                }
                tauri::WindowEvent::Focused(false) => {
                    trim_process_memory();
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .map_err(|e| anyhow::anyhow!("Erro ao executar runtime do Tauri: {}", e))
}
