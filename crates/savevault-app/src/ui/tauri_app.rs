//! Integração nativa do SaveVault com o runtime do Tauri 2 (DEV-UI-01).
//!
//! Implementa os comandos IPC tipados, o gerenciamento de eventos, bandeja do sistema (system tray)
//! e o comportamento de minimização/encerramento que preserva os serviços residentes em background.

use std::sync::Arc;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, State};
use uuid::Uuid;

use crate::ui::state::{
    DashboardSummary, DevUiEvent, DevUiState, DiscoverySummaryView, GameCardView, GameDetailView,
    LocationView, ReconciliationView, RestoreApiResponse, SafetyBackupGroup, ScanLocationResult,
    SnapshotView, SyncQueueItemView,
};

// ---------------------------------------------------------------------------
// Comandos IPC do Tauri
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_dashboard_status(
    state: State<'_, Arc<DevUiState>>,
) -> Result<DashboardSummary, String> {
    state.get_dashboard().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_games(state: State<'_, Arc<DevUiState>>) -> Result<Vec<GameCardView>, String> {
    state.get_games().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_game_details(
    state: State<'_, Arc<DevUiState>>,
    game_id: String,
) -> Result<GameDetailView, String> {
    let id = Uuid::parse_str(&game_id).map_err(|e| format!("UUID de jogo inválido: {}", e))?;
    state.get_game_details(id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_save_locations(
    state: State<'_, Arc<DevUiState>>,
    game_id: Option<String>,
) -> Result<Vec<LocationView>, String> {
    let locations = state.get_locations().await.map_err(|e| e.to_string())?;
    if let Some(ref gid_str) = game_id {
        if let Ok(gid) = Uuid::parse_str(gid_str) {
            return Ok(locations.into_iter().filter(|l| l.game_id == gid).collect());
        }
    }
    Ok(locations)
}

#[tauri::command]
pub async fn scan_location(
    state: State<'_, Arc<DevUiState>>,
    location_id: String,
) -> Result<ScanLocationResult, String> {
    let id =
        Uuid::parse_str(&location_id).map_err(|e| format!("UUID de location inválido: {}", e))?;
    state.scan_location(id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_snapshots(
    state: State<'_, Arc<DevUiState>>,
    game_id: Option<String>,
) -> Result<Vec<SnapshotView>, String> {
    let snapshots = state.get_snapshots().await.map_err(|e| e.to_string())?;
    if let Some(ref gid_str) = game_id {
        if let Ok(gid) = Uuid::parse_str(gid_str) {
            let games = state.get_games().await.map_err(|e| e.to_string())?;
            if let Some(target_game) = games.into_iter().find(|g| g.id == gid) {
                return Ok(snapshots
                    .into_iter()
                    .filter(|s| s.game_name == target_game.name)
                    .collect());
            }
        }
    }
    Ok(snapshots)
}

#[tauri::command]
pub async fn restore_snapshot(
    state: State<'_, Arc<DevUiState>>,
    snapshot_id: String,
    expected_hash: Option<String>,
) -> Result<RestoreApiResponse, String> {
    let id =
        Uuid::parse_str(&snapshot_id).map_err(|e| format!("UUID de snapshot inválido: {}", e))?;
    let resp = state.trigger_restore(id, expected_hash).await;
    Ok(resp)
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
pub async fn run_reconciliation(
    state: State<'_, Arc<DevUiState>>,
) -> Result<ReconciliationView, String> {
    state.trigger_reconcile().await.map_err(|e| e.to_string())?;
    state
        .get_reconciliation_status()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_sync_queue(
    state: State<'_, Arc<DevUiState>>,
) -> Result<Vec<SyncQueueItemView>, String> {
    state.get_sync_queue().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_recent_events(
    state: State<'_, Arc<DevUiState>>,
) -> Result<Vec<DevUiEvent>, String> {
    Ok(state.get_events().await)
}

#[tauri::command]
pub async fn trigger_discovery(
    state: State<'_, Arc<DevUiState>>,
) -> Result<DiscoverySummaryView, String> {
    state.trigger_discovery().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_drive_config(
    state: State<'_, Arc<DevUiState>>,
) -> Result<crate::ui::state::DriveConfigView, String> {
    state.get_drive_config().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_drive_folder(
    state: State<'_, Arc<DevUiState>>,
    folder_path: String,
) -> Result<crate::ui::state::DriveConfigView, String> {
    state
        .set_drive_folder(folder_path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn validate_drive_folder(
    state: State<'_, Arc<DevUiState>>,
    folder_path: Option<String>,
) -> Result<crate::ui::state::DriveValidationResult, String> {
    state
        .validate_drive_folder(folder_path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_drive_structure(
    state: State<'_, Arc<DevUiState>>,
) -> Result<crate::ui::state::DriveStructureResult, String> {
    state
        .create_drive_structure()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_drive_folder(state: State<'_, Arc<DevUiState>>) -> Result<String, String> {
    state.open_drive_folder().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sync_all_to_drive(state: State<'_, Arc<DevUiState>>) -> Result<usize, String> {
    state.sync_all_to_drive().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sync_game_to_drive(
    state: State<'_, Arc<DevUiState>>,
    game_id: String,
) -> Result<usize, String> {
    let id = Uuid::parse_str(&game_id).map_err(|e| format!("UUID de jogo inválido: {}", e))?;
    state
        .sync_game_to_drive(id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn select_drive_folder_native() -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(|| {
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Add-Type -AssemblyName System.Windows.Forms; $f = New-Object System.Windows.Forms.FolderBrowserDialog; $f.Description = 'Selecione a pasta do Google Drive para o SaveVault'; $f.ShowNewFolderButton = $true; if ($f.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { [Console]::WriteLine($f.SelectedPath) }",
            ])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if s.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(s))
                }
            }
            Ok(_) => Ok(None),
            Err(e) => Err(format!("Falha ao abrir seletor nativo de pasta: {}", e)),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// Inicialização do App Tauri
// ---------------------------------------------------------------------------

/// Executa a interface gráfica Tauri integrada ao processo residente.
pub fn run_tauri_app(state: Arc<DevUiState>) -> anyhow::Result<()> {
    let builder = tauri::Builder::default()
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            get_dashboard_status,
            get_games,
            get_game_details,
            get_save_locations,
            scan_location,
            get_snapshots,
            restore_snapshot,
            get_safety_backups,
            get_reconciliation_status,
            run_reconciliation,
            get_sync_queue,
            get_recent_events,
            trigger_discovery,
            get_drive_config,
            set_drive_folder,
            validate_drive_folder,
            create_drive_structure,
            open_drive_folder,
            sync_all_to_drive,
            sync_game_to_drive,
            select_drive_folder_native,
        ])
        .setup(|app| {
            // Garante que a janela principal seja exibida e receba foco imediatamente
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
            }

            // Configura o System Tray
            let show_item = MenuItemBuilder::with_id("show", "Abrir Interface").build(app)?;
            let hide_item = MenuItemBuilder::with_id("hide", "Ocultar Interface").build(app)?;
            let recon_item =
                MenuItemBuilder::with_id("reconcile", "Reconciliar Agora").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "Sair do SaveVault").build(app)?;

            let menu = MenuBuilder::new(app)
                .item(&show_item)
                .item(&hide_item)
                .separator()
                .item(&recon_item)
                .separator()
                .item(&quit_item)
                .build()?;

            let icon = app.default_window_icon().cloned();
            if let Some(ic) = icon {
                let _tray = TrayIconBuilder::new()
                    .icon(ic)
                    .tooltip("SaveVault — Gerenciador de Saves")
                    .menu(&menu)
                    .on_menu_event(|app, event| match event.id().as_ref() {
                        "show" => {
                            if let Some(win) = app.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.unminimize();
                                let _ = win.set_focus();
                                let _ = win.emit("ui_state_change", true);
                            }
                        }
                        "hide" => {
                            if let Some(win) = app.get_webview_window("main") {
                                let _ = win.hide();
                                let _ = win.emit("ui_state_change", false);
                            }
                        }
                        "reconcile" => {
                            let state_ref: State<'_, Arc<DevUiState>> = app.state();
                            let s = state_ref.inner().clone();
                            tauri::async_runtime::spawn(async move {
                                let _ = s.trigger_reconcile().await;
                            });
                        }
                        "quit" => {
                            let state_ref: State<'_, Arc<DevUiState>> = app.state();
                            let s = state_ref.inner().clone();
                            let app_handle = app.clone();
                            tauri::async_runtime::spawn(async move {
                                tracing::info!("Encerramento solicitado pelo System Tray. Aguardando finalização graciosa dos serviços...");
                                let clean_exit = s.wait_graceful_shutdown(std::time::Duration::from_secs(15)).await;
                                if !clean_exit {
                                    tracing::error!("Timeout de 15s excedido durante encerramento no tray. Forçando exit.");
                                }
                                app_handle.exit(0);
                            });
                        }
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click { .. } = event {
                            let app = tray.app_handle();
                            if let Some(win) = app.get_webview_window("main") {
                                if win.is_visible().unwrap_or(false) {
                                    let _ = win.hide();
                                    let _ = win.emit("ui_state_change", false);
                                } else {
                                    let _ = win.show();
                                    let _ = win.unminimize();
                                    let _ = win.set_focus();
                                    let _ = win.emit("ui_state_change", true);
                                }
                            }
                        }
                    })
                    .build(app)?;
            }

            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                // Impede o encerramento do processo! O backend continua residente.
                api.prevent_close();
                let _ = window.hide();
                let _ = window.emit("ui_state_change", false);
            }
            tauri::WindowEvent::Resized(..) if window.is_minimized().unwrap_or(false) => {
                let _ = window.emit("ui_state_change", false);
            }
            _ => {}
        });

    builder
        .run(tauri::generate_context!())
        .map_err(|e| anyhow::anyhow!("Erro ao executar runtime do Tauri: {}", e))?;

    Ok(())
}
