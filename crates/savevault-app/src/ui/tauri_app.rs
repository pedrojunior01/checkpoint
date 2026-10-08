use std::sync::Arc;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{TrayIconBuilder, TrayIconEvent},
    Manager, State,
};
use uuid::Uuid;

use crate::ui::state::{
    open_path_in_explorer, CloudConfigView, DashboardSummary, DetectedLibraryView, DevUiState,
    DiscoveredGameView, LocationView, OnboardingRequest, ReconciliationView, RestoreApiResponse,
    SafetyBackupGroup, SetupStatusView, SnapshotView,
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
pub async fn trigger_sync(state: State<'_, Arc<DevUiState>>) -> Result<usize, String> {
    state.trigger_sync().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn restore_snapshot(
    state: State<'_, Arc<DevUiState>>,
    snapshot_id: String,
) -> Result<RestoreApiResponse, String> {
    let id = Uuid::parse_str(&snapshot_id).map_err(|e| e.to_string())?;
    let resp = state.trigger_restore(id, None).await;
    if !resp.success {
        return Err(resp
            .error
            .unwrap_or_else(|| "Falha na restauração do snapshot".to_string()));
    }
    Ok(resp)
}

#[tauri::command]
pub async fn get_setup_status(
    state: State<'_, Arc<DevUiState>>,
) -> Result<SetupStatusView, String> {
    state.get_setup_status().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_available_libraries(
    state: State<'_, Arc<DevUiState>>,
) -> Result<Vec<DetectedLibraryView>, String> {
    state.get_available_libraries().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn complete_onboarding(
    state: State<'_, Arc<DevUiState>>,
    req: OnboardingRequest,
) -> Result<SetupStatusView, String> {
    state
        .complete_onboarding(req)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn skip_onboarding(state: State<'_, Arc<DevUiState>>) -> Result<SetupStatusView, String> {
    state.skip_onboarding().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn run_setup_discovery(
    state: State<'_, Arc<DevUiState>>,
    selected_libraries: Option<Vec<String>>,
) -> Result<Vec<DiscoveredGameView>, String> {
    state
        .run_setup_discovery_filtered(selected_libraries)
        .await
        .map_err(|e| e.to_string())
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
    state
        .add_game(&name, &path)
        .await
        .map_err(|e| e.to_string())
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

#[tauri::command]
pub async fn hide_window(
    window: tauri::WebviewWindow,
    state: State<'_, Arc<DevUiState>>,
) -> Result<(), String> {
    let _ = window.hide();
    let config = state.get_config();
    if config.notifications.enabled {
        #[cfg(windows)]
        {
            crate::notifications::notify(
                "checkpoint em segundo plano",
                "O aplicativo foi minimizado para os apps ocultos perto do relógio. Seus saves continuam protegidos.",
            );
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn minimize_window(window: tauri::WebviewWindow) -> Result<(), String> {
    window.minimize().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn toggle_maximize_window(window: tauri::WebviewWindow) -> Result<(), String> {
    if window.is_maximized().unwrap_or(false) {
        window.unmaximize().map_err(|e| e.to_string())
    } else {
        window.maximize().map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn optimize_database(
    state: State<'_, Arc<DevUiState>>,
) -> Result<crate::ui::state::OptimizationResult, String> {
    state.optimize_database().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn export_backup_zip(
    state: State<'_, Arc<DevUiState>>,
) -> Result<crate::ui::state::ExportResult, String> {
    state.export_backup_zip().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_pending_remappings(
    state: State<'_, Arc<DevUiState>>,
) -> Result<Vec<crate::remapping::PendingMappingView>, String> {
    state
        .get_pending_remappings()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn validate_remapping_path(
    state: State<'_, Arc<DevUiState>>,
    candidate_path: String,
) -> Result<crate::remapping::ValidationResult, String> {
    Ok(state.validate_remapping_path(&candidate_path).await)
}

#[tauri::command]
pub async fn apply_remapping(
    state: State<'_, Arc<DevUiState>>,
    location_id: String,
    new_path: String,
) -> Result<(), String> {
    let id = Uuid::parse_str(&location_id).map_err(|e| e.to_string())?;
    state
        .apply_remapping(id, &new_path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reset_remapping(
    state: State<'_, Arc<DevUiState>>,
    location_id: String,
) -> Result<(), String> {
    let id = Uuid::parse_str(&location_id).map_err(|e| e.to_string())?;
    state.reset_remapping(id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn pick_folder(state: State<'_, Arc<DevUiState>>) -> Result<Option<String>, String> {
    state.pick_folder_dialog().await.map_err(|e| e.to_string())
}

// ── App Setup ────────────────────────────────────────────────────────────────

/// Executa a interface gráfica Tauri integrada ao processo residente.
/// Deve ser chamada na thread principal do processo.
pub fn run_tauri_app(state: Arc<DevUiState>, start_minimized: bool) -> anyhow::Result<()> {
    tauri::Builder::default()
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            get_dashboard_status,
            get_locations,
            get_snapshots,
            get_safety_backups,
            get_reconciliation_status,
            trigger_reconcile,
            trigger_sync,
            restore_snapshot,
            get_setup_status,
            get_available_libraries,
            complete_onboarding,
            skip_onboarding,
            run_setup_discovery,
            get_cloud_folders,
            save_setup_cloud,
            complete_setup,
            add_game,
            open_folder,
            pick_folder,
            get_cloud_config,
            update_cloud_config,
            hide_window,
            minimize_window,
            toggle_maximize_window,
            optimize_database,
            export_backup_zip,
            get_pending_remappings,
            validate_remapping_path,
            apply_remapping,
            reset_remapping,
        ])
        .setup(move |app| {
            // Configura System Tray moderno e enriquecido
            let status_item = MenuItemBuilder::with_id("status", "✓ Status: Protegido")
                .enabled(false)
                .build(app)?;
            let show_item = MenuItemBuilder::with_id("show", "🖥️ Abrir SaveVault").build(app)?;
            let reconcile_item =
                MenuItemBuilder::with_id("reconcile", "🔄 Sincronizar / Verificar").build(app)?;
            let games_item = MenuItemBuilder::with_id("games", "🎮 Ver Jogos").build(app)?;
            let config_item = MenuItemBuilder::with_id("config", "⚙️ Configurações").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "❌ Sair").build(app)?;

            let menu = MenuBuilder::new(app)
                .item(&status_item)
                .separator()
                .item(&show_item)
                .item(&reconcile_item)
                .item(&games_item)
                .separator()
                .item(&config_item)
                .item(&quit_item)
                .build()?;

            let tray_icon = app.default_window_icon().cloned();

            // Mostra janela principal e garante que tenha o ícone
            if let Some(win) = app.get_webview_window("main") {
                if let Some(ref ic) = tray_icon {
                    let _ = win.set_icon(ic.clone());
                }
                if start_minimized {
                    let _ = win.hide();
                } else {
                    let _ = win.show();
                    let _ = win.unminimize();
                    let _ = win.set_focus();
                }
            }

            if let Some(ic) = tray_icon {
                TrayIconBuilder::new()
                    .icon(ic)
                    .tooltip("SaveVault — Proteção de Saves")
                    .menu(&menu)
                    .on_menu_event(|app, event| match event.id().as_ref() {
                        "show" => {
                            if let Some(win) = app.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.unminimize();
                                let _ = win.set_focus();
                            }
                        }
                        "games" => {
                            if let Some(win) = app.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.unminimize();
                                let _ = win.set_focus();
                                let _ = win.eval("if (window.showTab) { window.showTab('nav-jogos'); }");
                            }
                        }
                        "config" => {
                            if let Some(win) = app.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.unminimize();
                                let _ = win.set_focus();
                                let _ = win.eval("if (window.showTab) { window.showTab('nav-config'); }");
                            }
                        }
                        "reconcile" => {
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
            // Fechar [X] = ocultar para a bandeja/apps ocultos (processo permanece ativo protegendo saves)
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                let app = window.app_handle();
                let state = app.state::<Arc<DevUiState>>();
                let config = state.get_config();
                if config.notifications.enabled {
                    #[cfg(windows)]
                    {
                        crate::notifications::notify(
                            "checkpoint em segundo plano",
                            "O aplicativo foi minimizado para os apps ocultos perto do relógio. Seus saves continuam protegidos.",
                        );
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .map_err(|e| anyhow::anyhow!("Erro ao executar runtime do Tauri: {}", e))
}
