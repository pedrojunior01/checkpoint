use std::sync::Arc;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{TrayIconBuilder, TrayIconEvent},
    Manager, State,
};
use uuid::Uuid;

use crate::ui::state::{
    open_path_in_explorer, CloudConfigView, DashboardSummary, DevUiState, DiscoveredGameView,
    LocationView, ReconciliationView, RestoreApiResponse, SafetyBackupGroup, SetupStatusView,
    SnapshotView,
};

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
pub async fn get_snapshots(state: State<'_, Arc<DevUiState>>) -> Result<Vec<SnapshotView>, String> {
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
pub async fn get_setup_status(
    state: State<'_, Arc<DevUiState>>,
) -> Result<SetupStatusView, String> {
    state.get_setup_status().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn run_setup_discovery(
    state: State<'_, Arc<DevUiState>>,
) -> Result<Vec<DiscoveredGameView>, String> {
    state.run_setup_discovery().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_cloud_folders() -> Result<Vec<savevault_cloud::DetectedCloudFolder>, String> {
    Ok(savevault_cloud::detect_cloud_folders())
}

#[tauri::command]
pub async fn save_setup_cloud(
    state: State<'_, Arc<DevUiState>>,
    provider: Option<String>,
    local_sync_path: Option<String>,
) -> Result<SetupStatusView, String> {
    let mut config = state.get_config();
    if let Some(ref lsp) = local_sync_path {
        let trimmed = lsp.trim();
        if !trimmed.is_empty() {
            let p = std::path::PathBuf::from(trimmed);
            let _ = std::fs::create_dir_all(&p);
            config.cloud.local_sync_path = Some(p);
        }
    }
    if let Some(p) = provider {
        config.cloud.provider = p;
    }
    let _ = state.save_config(&config);
    state.get_setup_status().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn complete_setup(
    state: State<'_, Arc<DevUiState>>,
    provider: Option<String>,
    autostart: Option<bool>,
    notifications: Option<bool>,
    local_sync_path: Option<String>,
) -> Result<SetupStatusView, String> {
    state
        .complete_setup(provider, autostart, notifications, local_sync_path)
        .await
        .map_err(|e| e.to_string())
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
pub async fn open_folder(path: String) -> Result<(), String> {
    open_path_in_explorer(&path).map_err(|e| e.to_string())
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

// ── App Setup ────────────────────────────────────────────────────────────────

/// Executa a interface gráfica Tauri integrada ao processo residente.
/// Deve ser chamada na thread principal do processo.
pub fn run_tauri_app(state: Arc<DevUiState>) -> anyhow::Result<()> {
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
            get_setup_status,
            run_setup_discovery,
            get_cloud_folders,
            save_setup_cloud,
            complete_setup,
            add_game,
            open_folder,
            get_cloud_config,
            update_cloud_config,
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
                            }
                        }
                        "reconcile" => {
                            // Dispara reconciliação via canal Tauri async
                            let state = app.state::<Arc<DevUiState>>();
                            let state_clone = Arc::clone(&state);
                            tauri::async_runtime::spawn(async move {
                                let _ = state_clone.trigger_reconcile().await;
                            });
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        // Duplo clique: toggle visibilidade
                        if let TrayIconEvent::Click { .. } = event {
                            let app = tray.app_handle();
                            if let Some(win) = app.get_webview_window("main") {
                                if win.is_visible().unwrap_or(false) {
                                    let _ = win.hide();
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

            Ok(())
        })
        .on_window_event(|window, event| {
            // Fechar = ocultar para bandeja (processo permanece ativo)
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .map_err(|e| anyhow::anyhow!("Erro ao executar runtime do Tauri: {}", e))
}
