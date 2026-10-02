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
    DashboardSummary, DevUiState, LocationView, ReconciliationView, RestoreApiResponse,
    SafetyBackupGroup, SnapshotView,
};

// ── Utilitário de Limpeza de Memória (Working Set Trim) ───────────────────────

/// Solicita ao Gerenciador de Memória do Windows o descarte imediato de
/// páginas de memória ociosas do processo, devolvendo RAM para o sistema.
#[cfg(windows)]
pub fn trim_process_memory() {
    extern "system" {
        fn SetProcessWorkingSetSize(
            h_process: *mut std::ffi::c_void,
            min: usize,
            max: usize,
        ) -> i32;
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
    }
    unsafe {
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
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

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ConfigDto {
    pub max_versions: u32,
    pub debounce_ms: u64,
    pub reconciliation_interval_secs: u64,
    pub log_level: String,
    pub backup_dir: String,
    pub database_path: String,
}

fn config_path() -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(appdata).join("SaveVault").join("config.toml")
}

#[tauri::command]
pub fn get_config() -> Result<ConfigDto, String> {
    let path = config_path();
    let cfg = Config::load(&path).map_err(|e| e.to_string())?;
    Ok(ConfigDto {
        max_versions: cfg.general.max_versions,
        debounce_ms: cfg.general.debounce_ms,
        reconciliation_interval_secs: cfg.general.reconciliation_interval_secs,
        log_level: cfg.general.log_level,
        backup_dir: cfg.paths.backup_dir.to_string_lossy().to_string(),
        database_path: cfg.paths.database_path.to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub fn save_config(new_cfg: ConfigDto) -> Result<String, String> {
    let path = config_path();
    let mut cfg = Config::load(&path).unwrap_or_default();
    
    cfg.general.max_versions = new_cfg.max_versions.max(1);
    cfg.general.debounce_ms = new_cfg.debounce_ms.max(100);
    cfg.general.reconciliation_interval_secs = new_cfg.reconciliation_interval_secs.max(5);
    cfg.general.log_level = new_cfg.log_level;
    cfg.paths.backup_dir = PathBuf::from(new_cfg.backup_dir);
    
    cfg.save(&path).map_err(|e| format!("Erro ao salvar arquivo de configuração: {}", e))?;
    Ok("Configurações salvas com sucesso!".into())
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
        std::process::Command::new("explorer.exe")
            .arg(target)
            .spawn()
            .map_err(|e| format!("Não foi possível abrir o Explorer: {}", e))?;
    }
    Ok(())
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

// ── App Setup ────────────────────────────────────────────────────────────────

/// Executa a interface gráfica Tauri integrada ao processo residente.
/// Deve ser chamada na thread principal do processo.
pub fn run_tauri_app(state: Arc<DevUiState>) -> anyhow::Result<()> {
    // Otimizações do Chromium para eliminar subprocessos pesados de GPU e desativar módulos não utilizados
    #[cfg(windows)]
    std::env::set_var(
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "--disable-gpu --disable-gpu-compositing --disable-gpu-rasterization --disable-features=Translate,OptimizationHints,MediaRouter,CalculateNativeWinOcclusion,InterestFeedContentSuggestions --disable-extensions --disable-component-update --renderer-process-limit=1 --disable-background-networking --disable-speech-api --disable-sync --disable-breakpad --disable-default-apps --disable-domain-reliability --no-pings --mute-audio"
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
            get_dashboard_status,
            get_locations,
            get_snapshots,
            get_safety_backups,
            get_reconciliation_status,
            trigger_reconcile,
            restore_snapshot,
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

            // Limpeza inicial de memória após carregar
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
