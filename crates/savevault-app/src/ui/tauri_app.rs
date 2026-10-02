use std::sync::Arc;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{TrayIconBuilder, TrayIconEvent},
    Manager, State,
};
use uuid::Uuid;

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

// ── Commands ─────────────────────────────────────────────────────────────────

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
    // Configura flags de otimização do Chromium / WebView2 antes da criação da janela
    // Desativa módulos pesados desnecessários (tradutor, media router, extensões, etc.)
    #[cfg(windows)]
    std::env::set_var(
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "--disable-features=Translate,OptimizationHints,MediaRouter,CalculateNativeWinOcclusion,InterestFeedContentSuggestions --disable-extensions --disable-component-update --renderer-process-limit=1 --disable-background-networking --disable-speech-api --disable-sync --disable-breakpad --disable-default-apps --disable-domain-reliability --no-pings --mute-audio"
    );

    tauri::Builder::default()
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
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

            // Realiza uma limpeza inicial de memória após carregar
            trim_process_memory();

            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                // Fechar = ocultar para bandeja e liberar memória
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
