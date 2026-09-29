//! Estado e Controladores da Interface de Desenvolvimento (DEV-UI-01).
//!
//! Desacopla a camada visual das operações do backend, expondo uma API limpa
//! e testável sem dependência de rendering para consumo pela interface gráfica e testes.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use savevault_core::models::Platform;
use savevault_db::repositories::{FilesRepo, GamesRepo, SaveLocationsRepo, SyncQueueRepo};
use savevault_monitor::scanner::SaveScanner;

use crate::reconciler::{Reconciler, ReconciliationError, ReconciliationReport};
use crate::restore::{RestoreError, RestoreRequest};

/// Nível de severidade de um evento registrado na DEV-UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DevUiEventLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// Registro de evento para o log visual da DEV-UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevUiEvent {
    pub id: u64,
    pub timestamp: String,
    pub level: DevUiEventLevel,
    pub message: String,
    pub details: Option<String>,
}

/// Resumo geral do sistema para o Dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSummary {
    pub app_status: String,
    pub db_status: String,
    pub db_path: String,
    pub backup_root: String,
    pub save_locations_count: usize,
    pub monitored_files_count: usize,
    pub snapshots_count: usize,
    pub sync_pending_count: usize,
    pub watcher_status: String,
    pub reconciler_status: String,
    pub reconciliation_interval_secs: u64,
    pub last_reconciliation_time: Option<String>,
    pub next_reconciliation_time: Option<String>,
    pub last_event: Option<DevUiEvent>,
    pub last_error: Option<String>,
}

/// Card resumido de um Jogo para a tela principal da DEV-UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameCardView {
    pub id: Uuid,
    pub name: String,
    pub platform: String,
    pub platform_id: Option<String>,
    pub steam_app_id: Option<String>,
    pub gog_id: Option<String>,
    pub save_locations_count: usize,
    pub files_count: usize,
    pub snapshots_count: usize,
    pub drive_snapshots_count: usize,
    pub last_backup_local: Option<String>,
    pub last_copied_to_drive: Option<String>,
    pub monitoring_status: String,
    pub last_activity: Option<String>,
    pub last_error: Option<String>,
    pub sync_status: String,
}

/// Detalhes completos de um jogo para exibição e gerenciamento na UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameDetailView {
    pub game: GameCardView,
    pub locations: Vec<LocationView>,
    pub snapshots: Vec<SnapshotView>,
    pub recent_activity: Vec<DevUiEvent>,
    pub drive_snapshots_count: usize,
    pub last_backup_local: Option<String>,
    pub last_copied_to_drive: Option<String>,
}

/// Configuração e estado do Google Drive para a interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveConfigView {
    pub configured: bool,
    pub path: Option<String>,
    pub resolved_base_dir: Option<String>,
    pub exists: bool,
    pub is_available: bool,
    pub status_message: String,
    pub total_snapshots_in_drive: usize,
    pub structure: Vec<DrivePlatformTree>,
}

/// Estrutura em árvore de uma plataforma no Google Drive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrivePlatformTree {
    pub platform: String,
    pub games: Vec<DriveGameTree>,
}

/// Estrutura de um jogo no Google Drive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveGameTree {
    pub name: String,
    pub folder_name: String,
    pub snapshots_count: usize,
    pub last_snapshot_date: Option<String>,
}

/// Resultado da validação de uma pasta do Drive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveValidationResult {
    pub path: String,
    pub is_valid: bool,
    pub exists: bool,
    pub is_writable: bool,
    pub message: String,
}

/// Resultado da criação da estrutura de diretórios no Drive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveStructureResult {
    pub base_path: String,
    pub platforms_created: Vec<String>,
    pub games_created: Vec<String>,
    pub message: String,
}

/// Item da fila de sincronização com metadados para a DEV-UI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncQueueItemView {
    pub id: Uuid,
    pub backup_id: Uuid,
    pub operation: String,
    pub status: String,
    pub attempts: u32,
    pub file_name: String,
    pub game_name: String,
    pub created_at: String,
    pub last_error: Option<String>,
}

/// Resumo do resultado da execução do Discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoverySummaryView {
    pub games_found: usize,
    pub locations_found: usize,
    pub new_games: usize,
    pub new_locations: usize,
}

/// Visão de um SaveLocation cadastrado.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocationView {
    pub id: Uuid,
    pub game_id: Uuid,
    pub game_name: String,
    pub platform: String,
    pub path: String,
    pub location_type: String,
    pub exists_on_disk: bool,
    pub monitored_files_count: usize,
    pub last_check: String,
}

/// Arquivo físico descoberto durante varredura manual.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredFileView {
    pub path: String,
    pub filename: String,
    pub size_bytes: u64,
    pub size_formatted: String,
    pub modified_at: String,
}

/// Resultado do scan sob demanda de um SaveLocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanLocationResult {
    pub location_id: Uuid,
    pub files_found: Vec<DiscoveredFileView>,
    pub scanned_at: String,
}

/// Visão estruturada de um Snapshot para a UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotView {
    pub id: Uuid,
    pub file_id: Uuid,
    pub game_name: String,
    pub file_path: String,
    pub file_name: String,
    pub version: u32,
    pub size_bytes: u64,
    pub size_formatted: String,
    pub hash: String,
    pub short_hash: String,
    pub created_at: String,
    pub physical_path: String,
    pub physical_exists: bool,
    pub sync_status: String,
}

/// Informações de um Safety Backup individual.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyBackupItem {
    pub filename: String,
    pub path: String,
    pub size_bytes: u64,
    pub size_formatted: String,
    pub modified_at: String,
}

/// Grupo de Safety Backups associados a um arquivo original.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyBackupGroup {
    pub original_file_id: Uuid,
    pub original_path: String,
    pub original_name: String,
    pub game_name: String,
    pub backups: Vec<SafetyBackupItem>,
}

/// Visão detalhada do status de Reconciliação.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconciliationView {
    pub status: String,
    pub interval_secs: u64,
    pub is_running: bool,
    pub last_run: Option<String>,
    pub next_run: Option<String>,
    pub locations_scanned: usize,
    pub files_seen: usize,
    pub files_added: usize,
    pub files_changed: usize,
    pub files_removed: usize,
    pub errors: usize,
    pub duration_ms: u64,
}

/// Resposta de uma operação de restauração para a UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreApiResponse {
    pub success: bool,
    pub snapshot_id: Option<Uuid>,
    pub version: Option<u32>,
    pub dest_path: Option<String>,
    pub safety_backup_path: Option<String>,
    pub restored_hash: Option<String>,
    pub bytes_restored: Option<u64>,
    pub error: Option<String>,
    pub details: Option<String>,
}

/// RAII Guard que rastreia uma operação crítica em andamento (ex: snapshot, restauração).
/// Ao sofrer drop, decrementa atomicamente o contador `in_flight_operations`.
#[derive(Debug)]
pub struct InFlightOpGuard {
    counter: Arc<std::sync::atomic::AtomicUsize>,
}

impl Drop for InFlightOpGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Estado compartilhado e thread-safe da DEV-UI.
pub struct DevUiState {
    pub db: Arc<Mutex<Connection>>,
    pub db_path: PathBuf,
    pub config_dir: PathBuf,
    pub backup_root: PathBuf,
    pub reconciler: Option<Arc<Reconciler>>,
    pub reconciliation_interval: Duration,
    pub watcher_active: AtomicBool,
    pub reconciler_active: AtomicBool,
    pub in_flight_operations: Arc<std::sync::atomic::AtomicUsize>,
    event_counter: AtomicU64,
    recent_events: Mutex<VecDeque<DevUiEvent>>,
    last_error: Mutex<Option<String>>,
    last_reconciliation_report: Mutex<Option<ReconciliationReport>>,
    last_reconcile_time: Mutex<Option<DateTime<Utc>>>,
    shutdown_tx: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
}

impl DevUiState {
    pub fn new(
        db: Arc<Mutex<Connection>>,
        db_path: PathBuf,
        backup_root: PathBuf,
        reconciler: Option<Arc<Reconciler>>,
        reconciliation_interval: Duration,
    ) -> Self {
        let config_dir = db_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        Self {
            db,
            db_path,
            config_dir,
            backup_root,
            reconciler,
            reconciliation_interval,
            watcher_active: AtomicBool::new(true),
            reconciler_active: AtomicBool::new(true),
            in_flight_operations: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            event_counter: AtomicU64::new(1),
            recent_events: Mutex::new(VecDeque::with_capacity(100)),
            last_error: Mutex::new(None),
            last_reconciliation_report: Mutex::new(None),
            last_reconcile_time: Mutex::new(None),
            shutdown_tx: Mutex::new(None),
        }
    }

    /// Registra uma operação crítica em andamento.
    /// Retorna `None` se o aplicativo já estiver em processo de shutdown (impedindo novos trabalhos).
    pub fn register_in_flight_operation(&self) -> Option<InFlightOpGuard> {
        if self.is_shutting_down() {
            return None;
        }
        self.in_flight_operations.fetch_add(1, Ordering::SeqCst);
        if self.is_shutting_down() {
            self.in_flight_operations.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(InFlightOpGuard {
            counter: self.in_flight_operations.clone(),
        })
    }

    /// Retorna a quantidade de operações críticas atualmente em execução.
    pub fn in_flight_count(&self) -> usize {
        self.in_flight_operations.load(Ordering::SeqCst)
    }

    /// Indica se o processo de shutdown já foi disparado.
    pub fn is_shutting_down(&self) -> bool {
        !self.watcher_active.load(Ordering::SeqCst)
            || !self.reconciler_active.load(Ordering::SeqCst)
    }

    /// Registra o transmissor de shutdown para encerramento gracioso dos serviços de background.
    pub async fn set_shutdown_sender(&self, tx: tokio::sync::watch::Sender<bool>) {
        let mut guard = self.shutdown_tx.lock().await;
        *guard = Some(tx);
    }

    /// Aciona o desligamento seguro de todos os serviços de background (watcher, pipeline, reconciliação).
    pub async fn trigger_graceful_shutdown(&self) {
        self.watcher_active.store(false, Ordering::SeqCst);
        self.reconciler_active.store(false, Ordering::SeqCst);
        let guard = self.shutdown_tx.lock().await;
        if let Some(ref tx) = *guard {
            let _ = tx.send(true);
        }
    }

    /// Solicita shutdown e aguarda ativamente a finalização de todas as operações críticas e reconciliações
    /// até o timeout informado.
    ///
    /// Justificativa de timeout recomendado em produção (15s):
    /// No Windows NTFS, saves de jogos típicos variam entre dezenas de KB a 200MB.
    /// Em SSD/HD comum, hashing BLAKE3 e cópia/rename atômico levam entre 0.5s e 3s.
    /// Um limite de 15 segundos concede margem ampla (>5x) para a conclusão segura de qualquer
    /// snapshot ou restore em trânsito sem congelar a UI indefinidamente em falhas de disco.
    pub async fn wait_graceful_shutdown(&self, timeout: Duration) -> bool {
        tracing::info!(
            "Disparando shutdown gracioso com barreira (timeout: {:?})...",
            timeout
        );
        self.trigger_graceful_shutdown().await;

        let start = std::time::Instant::now();
        let poll_interval = Duration::from_millis(25);

        while start.elapsed() < timeout {
            let is_recon = if let Some(ref r) = self.reconciler {
                r.is_reconciling()
            } else {
                false
            };
            let in_flight = self.in_flight_count();

            if !is_recon && in_flight == 0 {
                tracing::info!(
                    "Shutdown concluído graciosamente em {:?} sem operações pendentes.",
                    start.elapsed()
                );
                return true;
            }

            tokio::time::sleep(poll_interval).await;
        }

        let is_recon = if let Some(ref r) = self.reconciler {
            r.is_reconciling()
        } else {
            false
        };
        let in_flight = self.in_flight_count();
        tracing::warn!(
            "Timeout de shutdown atingido ({:?})! Pendências ativas: reconciliando={}, in_flight={}",
            timeout,
            is_recon,
            in_flight
        );
        false
    }

    /// Registra um evento no log em memória da DEV-UI.
    pub async fn record_event(
        &self,
        level: DevUiEventLevel,
        message: impl Into<String>,
        details: Option<String>,
    ) {
        let id = self.event_counter.fetch_add(1, Ordering::SeqCst);
        let event = DevUiEvent {
            id,
            timestamp: Utc::now().format("%H:%M:%S").to_string(),
            level,
            message: message.into(),
            details,
        };

        let mut events = self.recent_events.lock().await;
        if events.len() >= 100 {
            events.pop_front();
        }
        events.push_back(event);
    }

    /// Registra o último erro conhecido.
    pub async fn record_error(&self, err: &str) {
        let mut last_err = self.last_error.lock().await;
        *last_err = Some(err.to_string());
        self.record_event(DevUiEventLevel::Error, err, None).await;
    }

    /// Retorna os eventos recentes armazenados em memória.
    pub async fn get_events(&self) -> Vec<DevUiEvent> {
        let events = self.recent_events.lock().await;
        events.iter().cloned().rev().collect()
    }

    /// Retorna o resumo consolidado para o Dashboard.
    pub async fn get_dashboard(&self) -> Result<DashboardSummary> {
        let (locations_count, monitored_files_count, snapshots_count, sync_pending_count) = {
            let conn = self.db.lock().await;
            let loc_repo = SaveLocationsRepo::new(&conn);
            let files_repo = FilesRepo::new(&conn);
            let queue_repo = SyncQueueRepo::new(&conn);

            let locations = loc_repo.list_all().unwrap_or_default();
            let locations_count = locations.len();

            let mut monitored_files_count = 0usize;
            for loc in &locations {
                let files = files_repo.list_by_location(loc.id).unwrap_or_default();
                monitored_files_count += files.len();
            }

            let snapshots_count: usize = conn
                .query_row("SELECT COUNT(*) FROM backups", [], |r| r.get(0))
                .unwrap_or(0);

            let sync_pending_count = queue_repo.list_pending().unwrap_or_default().len();

            (
                locations_count,
                monitored_files_count,
                snapshots_count,
                sync_pending_count,
            )
        };

        let last_recon = *self.last_reconcile_time.lock().await;
        let last_recon_str = last_recon.map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string());

        let next_recon_str = last_recon.map(|t| {
            let next =
                t + chrono::Duration::from_std(self.reconciliation_interval).unwrap_or_default();
            next.format("%H:%M:%S UTC").to_string()
        });

        let watcher_status = if self.watcher_active.load(Ordering::SeqCst) {
            "Running".to_string()
        } else {
            "Idle".to_string()
        };

        let is_recon = if let Some(ref r) = self.reconciler {
            r.is_reconciling()
        } else {
            false
        };

        let reconciler_status = if is_recon {
            "Reconciling".to_string()
        } else if self.reconciler_active.load(Ordering::SeqCst) {
            "Running".to_string()
        } else {
            "Idle".to_string()
        };

        let last_event = {
            let events = self.recent_events.lock().await;
            events.back().cloned()
        };

        let last_error = self.last_error.lock().await.clone();

        let app_status = if last_error.is_some() {
            "Warning".to_string()
        } else {
            "Online".to_string()
        };

        Ok(DashboardSummary {
            app_status,
            db_status: "Connected".to_string(),
            db_path: self.db_path.to_string_lossy().to_string(),
            backup_root: self.backup_root.to_string_lossy().to_string(),
            save_locations_count: locations_count,
            monitored_files_count,
            snapshots_count,
            sync_pending_count,
            watcher_status,
            reconciler_status,
            reconciliation_interval_secs: self.reconciliation_interval.as_secs(),
            last_reconciliation_time: last_recon_str,
            next_reconciliation_time: next_recon_str,
            last_event,
            last_error,
        })
    }

    /// Retorna a lista de todos os Jogos mapeados com métricas e status para a DEV-UI.
    pub async fn get_games(&self) -> Result<Vec<GameCardView>> {
        let conn = self.db.lock().await;
        let games_repo = GamesRepo::new(&conn);
        let loc_repo = SaveLocationsRepo::new(&conn);
        let files_repo = FilesRepo::new(&conn);

        let games = games_repo.list_all()?;
        let mut views = Vec::new();

        for game in games {
            let locations = loc_repo.list_by_game(game.id).unwrap_or_default();
            let locations_count = locations.len();

            let mut files_count = 0usize;
            let mut has_existing_loc = false;
            for loc in &locations {
                if loc.path.exists() {
                    has_existing_loc = true;
                }
                let files = files_repo.list_by_location(loc.id).unwrap_or_default();
                files_count += files.len();
            }

            // Total de snapshots do jogo
            let snapshots_count: usize = conn
                .query_row(
                    "SELECT COUNT(b.id)
                     FROM backups b
                     JOIN files f ON b.file_id = f.id
                     JOIN save_locations sl ON f.save_location_id = sl.id
                     WHERE sl.game_id = ?1",
                    rusqlite::params![game.id.to_string()],
                    |r| r.get(0),
                )
                .unwrap_or(0);

            // Última atividade do jogo
            let last_activity: Option<String> = conn
                .query_row(
                    "SELECT b.created_at
                     FROM backups b
                     JOIN files f ON b.file_id = f.id
                     JOIN save_locations sl ON f.save_location_id = sl.id
                     WHERE sl.game_id = ?1
                     ORDER BY b.created_at DESC
                     LIMIT 1",
                    rusqlite::params![game.id.to_string()],
                    |r| r.get(0),
                )
                .ok()
                .or_else(|| Some(game.created_at.format("%Y-%m-%d %H:%M:%S").to_string()));

            // Status de sync e Drive do jogo
            let drive_snapshots_count: usize = conn
                .query_row(
                    "SELECT COUNT(b.id)
                     FROM backups b
                     JOIN files f ON b.file_id = f.id
                     JOIN save_locations sl ON f.save_location_id = sl.id
                     WHERE sl.game_id = ?1 AND b.sync_status = 'copied_to_drive_folder'",
                    rusqlite::params![game.id.to_string()],
                    |r| r.get(0),
                )
                .unwrap_or(0);

            let last_copied_to_drive: Option<String> = conn
                .query_row(
                    "SELECT b.created_at
                     FROM backups b
                     JOIN files f ON b.file_id = f.id
                     JOIN save_locations sl ON f.save_location_id = sl.id
                     WHERE sl.game_id = ?1 AND b.sync_status = 'copied_to_drive_folder'
                     ORDER BY b.created_at DESC
                     LIMIT 1",
                    rusqlite::params![game.id.to_string()],
                    |r| r.get(0),
                )
                .ok();

            let last_backup_local = last_activity.clone();

            let pending_sync: usize = conn
                .query_row(
                    "SELECT COUNT(sq.id)
                     FROM sync_queue sq
                     JOIN backups b ON sq.backup_id = b.id
                     JOIN files f ON b.file_id = f.id
                     JOIN save_locations sl ON f.save_location_id = sl.id
                     WHERE sl.game_id = ?1 AND sq.status = 'pending'",
                    rusqlite::params![game.id.to_string()],
                    |r| r.get(0),
                )
                .unwrap_or(0);

            let sync_status = if pending_sync > 0 {
                format!("Pendente ({})", pending_sync)
            } else if drive_snapshots_count > 0 {
                "Copiado para o Drive".to_string()
            } else if snapshots_count > 0 {
                "Apenas local".to_string()
            } else {
                "—".to_string()
            };

            let monitoring_status = if locations_count == 0 {
                "Sem locais".to_string()
            } else if has_existing_loc {
                "Monitorando".to_string()
            } else {
                "Aguardando".to_string()
            };

            let steam_app_id = if game.platform == Platform::Steam {
                game.platform_id.clone()
            } else {
                None
            };

            let gog_id = if game.platform == Platform::Gog {
                game.platform_id.clone()
            } else {
                None
            };

            views.push(GameCardView {
                id: game.id,
                name: game.name,
                platform: format!("{:?}", game.platform),
                platform_id: game.platform_id,
                steam_app_id,
                gog_id,
                save_locations_count: locations_count,
                files_count,
                snapshots_count,
                drive_snapshots_count,
                last_backup_local,
                last_copied_to_drive,
                monitoring_status,
                last_activity,
                last_error: None,
                sync_status,
            });
        }

        Ok(views)
    }

    /// Retorna detalhes completos de um jogo, incluindo save locations, snapshots e atividade recente.
    pub async fn get_game_details(&self, game_id: Uuid) -> Result<GameDetailView> {
        let games = self.get_games().await?;
        let game = games
            .into_iter()
            .find(|g| g.id == game_id)
            .ok_or_else(|| anyhow::anyhow!("Jogo '{}' não encontrado", game_id))?;

        let all_locations = self.get_locations().await?;
        let locations = all_locations
            .into_iter()
            .filter(|l| l.game_id == game_id)
            .collect();

        let all_snapshots = self.get_snapshots().await?;
        let snapshots = all_snapshots
            .into_iter()
            .filter(|s| s.game_name == game.name)
            .collect();

        let all_events = self.get_events().await;
        let recent_activity = all_events
            .into_iter()
            .filter(|e| {
                e.message.contains(&game.name)
                    || e.details.as_deref().unwrap_or("").contains(&game.name)
            })
            .take(15)
            .collect();

        Ok(GameDetailView {
            drive_snapshots_count: game.drive_snapshots_count,
            last_backup_local: game.last_backup_local.clone(),
            last_copied_to_drive: game.last_copied_to_drive.clone(),
            game,
            locations,
            snapshots,
            recent_activity,
        })
    }

    /// Retorna itens da fila de sincronização com metadados para a DEV-UI.
    pub async fn get_sync_queue(&self) -> Result<Vec<SyncQueueItemView>> {
        let conn = self.db.lock().await;
        let mut stmt = conn.prepare(
            "SELECT sq.id, sq.backup_id, sq.operation, sq.status, sq.attempts, sq.created_at,
                    f.path, g.name
             FROM sync_queue sq
             JOIN backups b ON sq.backup_id = b.id
             JOIN files f ON b.file_id = f.id
             JOIN save_locations sl ON f.save_location_id = sl.id
             JOIN games g ON sl.game_id = g.id
             ORDER BY sq.created_at DESC
             LIMIT 100",
        )?;

        let rows = stmt.query_map([], |row| {
            let id_str: String = row.get(0)?;
            let b_id: String = row.get(1)?;
            let op: String = row.get(2)?;
            let status: String = row.get(3)?;
            let attempts: u32 = row.get(4)?;
            let created_at: String = row.get(5)?;
            let file_path: String = row.get(6)?;
            let game_name: String = row.get(7)?;

            let file_name = std::path::Path::new(&file_path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or(file_path);

            Ok(SyncQueueItemView {
                id: Uuid::parse_str(&id_str).unwrap_or_default(),
                backup_id: Uuid::parse_str(&b_id).unwrap_or_default(),
                operation: op,
                status,
                attempts,
                file_name,
                game_name,
                created_at,
                last_error: None,
            })
        })?;

        let mut views = Vec::new();
        for r in rows {
            views.push(r?);
        }
        Ok(views)
    }

    /// Dispara a descoberta de jogos e save locations sob demanda e persiste no banco.
    pub async fn trigger_discovery(&self) -> Result<DiscoverySummaryView> {
        let result = savevault_discovery::run_all(&self.config_dir).await?;
        let mut new_games = 0usize;
        let mut new_locations = 0usize;

        {
            let conn = self.db.lock().await;
            let games_repo = GamesRepo::new(&conn);
            let locations_repo = SaveLocationsRepo::new(&conn);

            let mut real_ids = std::collections::HashMap::new();
            for game in &result.games {
                let mut real_id = game.id;
                if let Some(ref platform_id) = game.platform_id {
                    if let Ok(Some(existing)) =
                        games_repo.find_by_platform_id(&game.platform, platform_id)
                    {
                        real_id = existing.id;
                    } else if let Ok(Some(existing)) =
                        games_repo.find_by_name_and_platform(&game.name, &game.platform)
                    {
                        real_id = existing.id;
                    } else if games_repo.insert(game).is_ok() {
                        new_games += 1;
                    }
                } else if let Ok(Some(existing)) =
                    games_repo.find_by_name_and_platform(&game.name, &game.platform)
                {
                    real_id = existing.id;
                } else if games_repo.insert(game).is_ok() {
                    new_games += 1;
                }
                real_ids.insert(game.id, real_id);
            }

            for loc in &result.save_locations {
                if let Some(&real_id) = real_ids.get(&loc.game_id) {
                    let mut loc_to_insert = loc.clone();
                    loc_to_insert.game_id = real_id;
                    if let Ok(saved) = locations_repo.upsert(&loc_to_insert) {
                        if saved.id == loc_to_insert.id {
                            new_locations += 1;
                        }
                    }
                }
            }
        }

        self.record_event(
            DevUiEventLevel::Success,
            format!(
                "Descoberta executada: {} jogos encontrados ({} novos), {} locais detectados ({} novos)",
                result.games.len(),
                new_games,
                result.save_locations.len(),
                new_locations
            ),
            None,
        ).await;

        Ok(DiscoverySummaryView {
            games_found: result.games.len(),
            locations_found: result.save_locations.len(),
            new_games,
            new_locations,
        })
    }

    /// Retorna a lista de SaveLocations para a aba de Localizações.
    pub async fn get_locations(&self) -> Result<Vec<LocationView>> {
        let conn = self.db.lock().await;
        let loc_repo = SaveLocationsRepo::new(&conn);
        let games_repo = GamesRepo::new(&conn);
        let files_repo = FilesRepo::new(&conn);

        let locations = loc_repo.list_all().unwrap_or_default();
        let mut views = Vec::new();

        for loc in locations {
            let game = games_repo.find_by_id(loc.game_id).unwrap_or(None);
            let game_name = game
                .as_ref()
                .map(|g| g.name.clone())
                .unwrap_or_else(|| "Desconhecido".into());
            let platform = game
                .as_ref()
                .map(|g| format!("{:?}", g.platform))
                .unwrap_or_else(|| "Generic".into());

            let exists_on_disk = loc.path.exists();
            let files = files_repo.list_by_location(loc.id).unwrap_or_default();

            views.push(LocationView {
                id: loc.id,
                game_id: loc.game_id,
                game_name,
                platform,
                path: loc.path.to_string_lossy().to_string(),
                location_type: format!("{:?}", loc.location_type),
                exists_on_disk,
                monitored_files_count: files.len(),
                last_check: Utc::now().format("%H:%M:%S").to_string(),
            });
        }

        Ok(views)
    }

    /// Executa varredura sob demanda de um `SaveLocation` utilizando o `SaveScanner` existente.
    pub async fn scan_location(&self, location_id: Uuid) -> Result<ScanLocationResult> {
        let loc = {
            let conn = self.db.lock().await;
            let loc_repo = SaveLocationsRepo::new(&conn);
            loc_repo
                .find_by_id(location_id)?
                .ok_or_else(|| anyhow::anyhow!("SaveLocation '{}' não encontrado", location_id))?
        };

        let scanner = SaveScanner::default();
        let discovered = scanner.scan(&loc)?;

        let mut files_found = Vec::new();
        for f in discovered {
            let filename = f
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let size = f.size_bytes;
            let modified: DateTime<Utc> = f.modified_at.into();

            files_found.push(DiscoveredFileView {
                path: f.path.to_string_lossy().to_string(),
                filename,
                size_bytes: size,
                size_formatted: format_bytes(size),
                modified_at: modified.format("%Y-%m-%d %H:%M:%S").to_string(),
            });
        }

        self.record_event(
            DevUiEventLevel::Info,
            format!(
                "Scan manual concluído para location {}: {} arquivo(s) encontrado(s)",
                location_id,
                files_found.len()
            ),
            None,
        )
        .await;

        Ok(ScanLocationResult {
            location_id,
            files_found,
            scanned_at: Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        })
    }

    /// Retorna a lista de todos os Snapshots cadastrados no cofre.
    pub async fn get_snapshots(&self) -> Result<Vec<SnapshotView>> {
        let conn = self.db.lock().await;
        let mut stmt = conn.prepare(
            "SELECT b.id, b.file_id, b.version, b.created_at, b.sync_status,
                    f.path, f.hash, f.size_bytes, g.name
             FROM backups b
             JOIN files f ON b.file_id = f.id
             JOIN save_locations sl ON f.save_location_id = sl.id
             JOIN games g ON sl.game_id = g.id
             ORDER BY b.created_at DESC",
        )?;

        let rows = stmt.query_map([], |row| {
            let b_id: String = row.get(0)?;
            let f_id: String = row.get(1)?;
            let version: i64 = row.get(2)?;
            let created_at: String = row.get(3)?;
            let sync_status: String = row.get(4)?;
            let file_path: String = row.get(5)?;
            let hash: String = row.get(6)?;
            let size_bytes: i64 = row.get(7)?;
            let game_name: String = row.get(8)?;

            Ok((
                b_id,
                f_id,
                version,
                created_at,
                sync_status,
                file_path,
                hash,
                size_bytes,
                game_name,
            ))
        })?;

        let mut views = Vec::new();
        for r in rows {
            let (
                b_id,
                f_id,
                version,
                created_at,
                sync_status,
                file_path,
                hash,
                size_bytes,
                game_name,
            ) = r?;
            let snapshot_id = Uuid::parse_str(&b_id).unwrap_or_default();
            let file_id = Uuid::parse_str(&f_id).unwrap_or_default();

            let file_p = PathBuf::from(&file_path);
            let file_name = file_p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| file_path.clone());

            let physical_p = self
                .backup_root
                .join(file_id.to_string())
                .join(format!("v{}", version))
                .join(&file_name);
            let physical_exists = physical_p.exists();

            let short_hash = if hash.len() > 12 {
                format!("{}...", &hash[..12])
            } else {
                hash.clone()
            };

            views.push(SnapshotView {
                id: snapshot_id,
                file_id,
                game_name,
                file_path: file_path.clone(),
                file_name,
                version: version as u32,
                size_bytes: size_bytes as u64,
                size_formatted: format_bytes(size_bytes as u64),
                hash,
                short_hash,
                created_at,
                physical_path: physical_p.to_string_lossy().to_string(),
                physical_exists,
                sync_status,
            });
        }

        Ok(views)
    }

    /// Descobre e agrupa Safety Backups existentes no disco para cada arquivo monitorado.
    pub async fn get_safety_backups(&self) -> Result<Vec<SafetyBackupGroup>> {
        let conn = self.db.lock().await;
        let mut stmt = conn.prepare(
            "SELECT f.id, f.path, g.name
             FROM files f
             JOIN save_locations sl ON f.save_location_id = sl.id
             JOIN games g ON sl.game_id = g.id",
        )?;

        let rows = stmt.query_map([], |row| {
            let f_id: String = row.get(0)?;
            let path: String = row.get(1)?;
            let game_name: String = row.get(2)?;
            Ok((f_id, path, game_name))
        })?;

        let mut groups = Vec::new();

        for r in rows {
            let (f_id, path_str, game_name) = r?;
            let file_id = Uuid::parse_str(&f_id).unwrap_or_default();
            let orig_path = PathBuf::from(&path_str);
            let orig_name = orig_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            let mut backups = Vec::new();
            if let Some(parent) = orig_path.parent() {
                if parent.exists() {
                    if let Ok(entries) = std::fs::read_dir(parent) {
                        for entry in entries.flatten() {
                            let fname = entry.file_name().to_string_lossy().to_string();
                            if fname.starts_with(&orig_name) && fname.contains(".savevault_safety_")
                            {
                                let meta = entry.metadata().ok();
                                let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                                let modified = meta
                                    .and_then(|m| m.modified().ok())
                                    .map(|st| {
                                        let dt: DateTime<Utc> = st.into();
                                        dt.format("%Y-%m-%d %H:%M:%S").to_string()
                                    })
                                    .unwrap_or_else(|| "Desconhecido".into());

                                backups.push(SafetyBackupItem {
                                    filename: fname,
                                    path: entry.path().to_string_lossy().to_string(),
                                    size_bytes: size,
                                    size_formatted: format_bytes(size),
                                    modified_at: modified,
                                });
                            }
                        }
                    }
                }
            }

            if !backups.is_empty() {
                groups.push(SafetyBackupGroup {
                    original_file_id: file_id,
                    original_path: path_str,
                    original_name: orig_name,
                    game_name,
                    backups,
                });
            }
        }

        Ok(groups)
    }

    /// Retorna as estatísticas e status atual do Reconciliador.
    pub async fn get_reconciliation_status(&self) -> Result<ReconciliationView> {
        let is_running = if let Some(ref r) = self.reconciler {
            r.is_reconciling()
        } else {
            false
        };

        let last_time = *self.last_reconcile_time.lock().await;
        let last_run = last_time.map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string());
        let next_run = last_time.map(|t| {
            let next =
                t + chrono::Duration::from_std(self.reconciliation_interval).unwrap_or_default();
            next.format("%H:%M:%S UTC").to_string()
        });

        let rep = self
            .last_reconciliation_report
            .lock()
            .await
            .clone()
            .unwrap_or_default();

        let status = if is_running {
            "Running".to_string()
        } else {
            "Idle".to_string()
        };

        Ok(ReconciliationView {
            status,
            interval_secs: self.reconciliation_interval.as_secs(),
            is_running,
            last_run,
            next_run,
            locations_scanned: rep.locations_scanned,
            files_seen: rep.files_seen,
            files_added: rep.files_added,
            files_changed: rep.files_changed,
            files_removed: rep.files_removed,
            errors: rep.errors,
            duration_ms: rep.duration.as_millis() as u64,
        })
    }

    /// Aciona um ciclo de reconciliação imediatamente utilizando o `Reconciler` real.
    pub async fn trigger_reconcile(&self) -> Result<ReconciliationReport, ReconciliationError> {
        let reconciler = self.reconciler.as_ref().ok_or_else(|| {
            ReconciliationError::Pipeline(anyhow::anyhow!("Reconciliador não inicializado"))
        })?;

        self.record_event(
            DevUiEventLevel::Info,
            "Ciclo de reconciliação sob demanda iniciado pela DEV-UI".to_string(),
            None,
        )
        .await;

        let res = reconciler.reconcile().await;
        match res {
            Ok(report) => {
                {
                    let mut last_rep = self.last_reconciliation_report.lock().await;
                    *last_rep = Some(report.clone());
                    let mut last_time = self.last_reconcile_time.lock().await;
                    *last_time = Some(Utc::now());
                }

                self.record_event(
                    DevUiEventLevel::Success,
                    format!(
                        "Reconciliação concluída: {} locais, {} arquivos vistos (+{} novos, ~{} modificados, -{} removidos, {} erros) em {:?}",
                        report.locations_scanned, report.files_seen, report.files_added,
                        report.files_changed, report.files_removed, report.errors, report.duration
                    ),
                    None,
                ).await;

                Ok(report)
            }
            Err(e) => {
                let err_msg = format!("Falha na reconciliação: {}", e);
                self.record_error(&err_msg).await;
                Err(e)
            }
        }
    }

    /// Executa a restauração de um snapshot com safety backup através da API real.
    pub async fn trigger_restore(
        &self,
        snapshot_id: Uuid,
        expected_hash: Option<String>,
    ) -> RestoreApiResponse {
        let _op_guard = match self.register_in_flight_operation() {
            Some(g) => g,
            None => {
                return RestoreApiResponse {
                    success: false,
                    snapshot_id: Some(snapshot_id),
                    version: None,
                    dest_path: None,
                    safety_backup_path: None,
                    restored_hash: None,
                    bytes_restored: None,
                    error: Some("Operação recusada: sistema em processo de encerramento.".into()),
                    details: Some("O aplicativo está sendo encerrado e não aceita novas operações de restauração.".into()),
                };
            }
        };

        let req = match expected_hash {
            Some(h) => RestoreRequest::with_expected_hash(snapshot_id, h),
            None => RestoreRequest::new(snapshot_id),
        };

        self.record_event(
            DevUiEventLevel::Info,
            format!(
                "Iniciando restauração do snapshot {} via DEV-UI",
                snapshot_id
            ),
            None,
        )
        .await;

        let config_path = self.config_dir.join("config.toml");
        let config = savevault_core::config::Config::load(&config_path).unwrap_or_default();
        let drive_dir = config.paths.drive_dir.as_deref();

        let res = crate::restore::restore_snapshot_with_db_and_drive(
            &self.db,
            &self.backup_root,
            drive_dir,
            &req,
        )
        .await;

        match res {
            Ok(report) => {
                let safety_str = report
                    .safety_backup_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string());
                let msg = format!(
                    "Restauração concluída com sucesso: versão {} de {} restaurada em {}",
                    report.version,
                    report.file_id,
                    report.dest_path.display()
                );

                self.record_event(
                    DevUiEventLevel::Success,
                    msg,
                    Some(format!(
                        "Hash: {}, Bytes: {}",
                        report.restored_hash, report.bytes_restored
                    )),
                )
                .await;

                RestoreApiResponse {
                    success: true,
                    snapshot_id: Some(report.snapshot_id),
                    version: Some(report.version),
                    dest_path: Some(report.dest_path.to_string_lossy().to_string()),
                    safety_backup_path: safety_str,
                    restored_hash: Some(report.restored_hash),
                    bytes_restored: Some(report.bytes_restored),
                    error: None,
                    details: None,
                }
            }
            Err(err) => {
                let (short_err, details) = match &err {
                    RestoreError::SnapshotNotFound(id) => (
                        format!("Snapshot '{}' não encontrado no banco de dados.", id),
                        "O identificador UUID informado não existe na tabela backups.".to_string(),
                    ),
                    RestoreError::PhysicalSnapshotMissing(path) => (
                        format!("Arquivo físico do snapshot ausente em '{}'.", path.display()),
                        "O arquivo de backup foi apagado ou movido do cofre físico.".to_string(),
                    ),
                    RestoreError::IntegrityCheckFailed(id, exp, act) => (
                        format!("Integridade violada para o snapshot '{}'!", id),
                        format!("O hash BLAKE3 real ('{}') diverge do esperado ('{}'). Restauração abortada para segurança.", act, exp),
                    ),
                    RestoreError::PathSecurityViolation(reason) => (
                        "Violação de segurança de caminho detectada!".to_string(),
                        format!("A tentativa de restauração tentou escapar do SaveLocation autorizado: {}", reason),
                    ),
                    RestoreError::SafetyBackupFailed(path, e) => (
                        format!("Falha ao criar Safety Backup em '{}'.", path.display()),
                        format!("Não foi possível duplicar o arquivo atual antes da substituição: {}. O save ativo não foi alterado.", e),
                    ),
                    RestoreError::AtomicWriteFailed(path, e) => (
                        format!("Falha na escrita atômica em '{}'.", path.display()),
                        format!("Erro durante staging ou rename: {}. O arquivo original permaneceu intacto.", e),
                    ),
                    other => (
                        "Falha durante a operação de restauração.".to_string(),
                        other.to_string(),
                    ),
                };

                self.record_event(
                    DevUiEventLevel::Error,
                    format!("Restauração falhou: {}", short_err),
                    Some(details.clone()),
                )
                .await;

                RestoreApiResponse {
                    success: false,
                    snapshot_id: Some(snapshot_id),
                    version: None,
                    dest_path: None,
                    safety_backup_path: None,
                    restored_hash: None,
                    bytes_restored: None,
                    error: Some(short_err),
                    details: Some(format!("{}\nO arquivo original foi preservado.", details)),
                }
            }
        }
    }

    /// Retorna as informações de configuração e árvore de pastas do Google Drive para a DEV-UI.
    pub async fn get_drive_config(&self) -> Result<DriveConfigView> {
        let config_path = self.config_dir.join("config.toml");
        let config = savevault_core::config::Config::load(&config_path).unwrap_or_default();

        let drive_path = config.paths.drive_dir.clone();
        if let Some(ref path) = drive_path {
            use savevault_backup::storage::FilesystemBackupStorage;
            let storage = FilesystemBackupStorage::new(path);
            let status = storage.validate_folder();

            let exists = path.exists();
            let is_available = status.is_available();
            let status_message = match &status {
                savevault_backup::storage::DriveFolderStatus::Available { .. } => {
                    "Pasta disponível".to_string()
                }
                savevault_backup::storage::DriveFolderStatus::NotFound { .. } => {
                    "Pasta não encontrada no disco".to_string()
                }
                savevault_backup::storage::DriveFolderStatus::AccessDenied { reason, .. } => {
                    format!("Acesso negado: {}", reason)
                }
                savevault_backup::storage::DriveFolderStatus::Unavailable { reason, .. } => {
                    format!("Indisponível: {}", reason)
                }
            };

            let mut total_snapshots = 0usize;
            let mut platform_trees = Vec::new();

            if is_available {
                let base_dir = storage.resolve_base_dir();
                if base_dir.exists() {
                    let platforms = ["Steam", "GOG", "Epic", "Other"];
                    for plat in &platforms {
                        let plat_dir = base_dir.join(plat);
                        if plat_dir.exists() {
                            let mut games = Vec::new();
                            if let Ok(entries) = std::fs::read_dir(&plat_dir) {
                                for entry in entries.flatten() {
                                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                                        let folder_name =
                                            entry.file_name().to_string_lossy().to_string();
                                        let meta_dir = entry.path().join("metadata");
                                        let mut snap_count = 0usize;
                                        let mut latest_date: Option<String> = None;

                                        if meta_dir.exists() {
                                            if let Ok(meta_entries) = std::fs::read_dir(&meta_dir) {
                                                for m in meta_entries.flatten() {
                                                    let fname =
                                                        m.file_name().to_string_lossy().to_string();
                                                    if fname.starts_with('v')
                                                        && fname.ends_with(".json")
                                                        && !fname.contains(".tmp.")
                                                    {
                                                        snap_count += 1;
                                                        if let Ok(c) =
                                                            std::fs::read_to_string(m.path())
                                                        {
                                                            if let Ok(meta) = serde_json::from_str::<savevault_backup::storage::StoredSnapshotMetadata>(&c) {
                                                                let date_str = meta.copied_at.format("%Y-%m-%d %H:%M").to_string();
                                                                if latest_date.is_none() || latest_date.as_ref().map(|d| d < &date_str).unwrap_or(false) {
                                                                    latest_date = Some(date_str);
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                        total_snapshots += snap_count;
                                        games.push(DriveGameTree {
                                            name: folder_name.clone(),
                                            folder_name,
                                            snapshots_count: snap_count,
                                            last_snapshot_date: latest_date,
                                        });
                                    }
                                }
                            }

                            if !games.is_empty() {
                                platform_trees.push(DrivePlatformTree {
                                    platform: plat.to_string(),
                                    games,
                                });
                            }
                        }
                    }
                }
            }

            let base_dir = storage.resolve_base_dir();
            let resolved_base_dir = Some(base_dir.to_string_lossy().to_string());

            Ok(DriveConfigView {
                configured: true,
                path: Some(path.to_string_lossy().to_string()),
                resolved_base_dir,
                exists,
                is_available,
                status_message,
                total_snapshots_in_drive: total_snapshots,
                structure: platform_trees,
            })
        } else {
            Ok(DriveConfigView {
                configured: false,
                path: None,
                resolved_base_dir: None,
                exists: false,
                is_available: false,
                status_message: "Nenhuma pasta configurada".to_string(),
                total_snapshots_in_drive: 0,
                structure: Vec::new(),
            })
        }
    }

    /// Salva e valida uma nova pasta do Google Drive na configuração persistente.
    pub async fn set_drive_folder(&self, path_str: String) -> Result<DriveConfigView> {
        let trimmed = path_str.trim();
        if trimmed.is_empty() {
            anyhow::bail!("O caminho da pasta não pode ser vazio.");
        }

        let normalized = savevault_core::path::normalize_windows_path_str(trimmed)
            .map_err(|e| anyhow::anyhow!("Caminho inválido: {}", e))?;
        let pbuf = PathBuf::from(&normalized);

        let config_path = self.config_dir.join("config.toml");
        let mut config = savevault_core::config::Config::load(&config_path).unwrap_or_default();
        config.paths.drive_dir = Some(pbuf.clone());
        config.save(&config_path)?;

        if let Some(ref r) = self.reconciler {
            r.set_drive_dir(Some(pbuf.clone()));
        }

        self.record_event(
            DevUiEventLevel::Info,
            format!("Pasta do Google Drive configurada: {}", normalized),
            None,
        )
        .await;

        self.get_drive_config().await
    }

    /// Valida uma pasta do Google Drive (permissões, existência, escrita).
    pub async fn validate_drive_folder(
        &self,
        path_str: Option<String>,
    ) -> Result<DriveValidationResult> {
        let path = match path_str {
            Some(ref s) if !s.trim().is_empty() => PathBuf::from(s.trim()),
            _ => {
                let config_path = self.config_dir.join("config.toml");
                let config = savevault_core::config::Config::load(&config_path).unwrap_or_default();
                match config.paths.drive_dir {
                    Some(p) => p,
                    None => {
                        return Ok(DriveValidationResult {
                            path: String::new(),
                            is_valid: false,
                            exists: false,
                            is_writable: false,
                            message: "Nenhuma pasta configurada".into(),
                        });
                    }
                }
            }
        };

        let exists = path.exists();
        let mut is_writable = false;
        let message;

        if exists {
            if path.is_dir() {
                let test_file = path.join(format!(".savevault_write_test_{}.tmp", Uuid::new_v4()));
                match std::fs::write(&test_file, b"ok") {
                    Ok(_) => {
                        let _ = std::fs::remove_file(&test_file);
                        is_writable = true;
                        message = "Pasta acessível e com permissão de escrita".to_string();
                    }
                    Err(e) => {
                        message = format!("Sem permissão de escrita na pasta: {}", e);
                    }
                }
            } else {
                message = "O caminho especificado não é um diretório".to_string();
            }
        } else {
            message = "A pasta não existe no disco".to_string();
        }

        Ok(DriveValidationResult {
            path: path.to_string_lossy().to_string(),
            is_valid: exists && is_writable,
            exists,
            is_writable,
            message,
        })
    }

    /// Cria a estrutura inicial de diretórios do SaveVault no Google Drive para todos os jogos cadastrados.
    pub async fn create_drive_structure(&self) -> Result<DriveStructureResult> {
        let config_path = self.config_dir.join("config.toml");
        let config = savevault_core::config::Config::load(&config_path).unwrap_or_default();
        let drive_dir = config.paths.drive_dir.ok_or_else(|| {
            anyhow::anyhow!("Nenhuma pasta do Google Drive configurada. Configure a pasta antes de criar a estrutura.")
        })?;

        use savevault_backup::storage::{BackupStorage, FilesystemBackupStorage};
        let storage = FilesystemBackupStorage::new(&drive_dir);
        let status = storage.validate_folder();
        if !status.is_available() {
            anyhow::bail!("A pasta do Google Drive não está acessível: {:?}", status);
        }

        let base_dir = storage.resolve_base_dir();
        std::fs::create_dir_all(&base_dir)?;

        let (platforms_vec, games_created) = {
            let conn = self.db.lock().await;
            let games_repo = GamesRepo::new(&conn);
            let games = games_repo.list_all()?;

            let mut platforms_set = std::collections::HashSet::new();
            let mut games_created = Vec::new();

            for game in &games {
                let plat_name = game.platform.display_folder_name();
                platforms_set.insert(plat_name.to_string());
                if let Ok(game_paths) =
                    storage.ensure_game_structure(&game.platform, &game.name, Some(game.id))
                {
                    let folder_name = game_paths
                        .game_dir
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("");
                    games_created.push(format!("{}/{}", plat_name, folder_name));
                }
            }

            let platforms_vec: Vec<String> = platforms_set.into_iter().collect();
            (platforms_vec, games_created)
        };

        self.record_event(
            DevUiEventLevel::Success,
            format!(
                "Estrutura do SaveVault criada no Drive com sucesso: {} plataforma(s), {} jogo(s)",
                platforms_vec.len(),
                games_created.len()
            ),
            None,
        )
        .await;

        Ok(DriveStructureResult {
            base_path: base_dir.to_string_lossy().to_string(),
            platforms_created: platforms_vec,
            games_created,
            message: "Estrutura do SaveVault criada com sucesso na pasta do Drive.".into(),
        })
    }

    /// Abre o Windows Explorer diretamente na pasta do Google Drive.
    pub async fn open_drive_folder(&self) -> Result<String> {
        let config_path = self.config_dir.join("config.toml");
        let config = savevault_core::config::Config::load(&config_path).unwrap_or_default();
        let drive_dir = config
            .paths
            .drive_dir
            .ok_or_else(|| anyhow::anyhow!("Nenhuma pasta do Google Drive configurada."))?;

        if !drive_dir.exists() {
            anyhow::bail!(
                "A pasta configurada não existe no disco: {}",
                drive_dir.display()
            );
        }

        let storage = savevault_backup::storage::FilesystemBackupStorage::new(&drive_dir);
        let target = storage.resolve_base_dir();
        let open_target = if target.exists() { target } else { drive_dir };

        std::process::Command::new("explorer")
            .arg(&open_target)
            .spawn()
            .map_err(|e| anyhow::anyhow!("Falha ao abrir Windows Explorer: {}", e))?;

        Ok(format!(
            "Pasta aberta no Windows Explorer: {}",
            open_target.display()
        ))
    }

    /// Sincroniza manualmente todos os snapshots pendentes para a pasta do Drive.
    pub async fn sync_all_to_drive(&self) -> Result<usize> {
        let config_path = self.config_dir.join("config.toml");
        let config = savevault_core::config::Config::load(&config_path).unwrap_or_default();
        let drive_dir = config
            .paths
            .drive_dir
            .ok_or_else(|| anyhow::anyhow!("Nenhuma pasta do Google Drive configurada."))?;

        let count = {
            let conn = self.db.lock().await;
            crate::pipeline::sync_all_pending_to_drive_folder(&conn, &self.backup_root, &drive_dir)?
        };

        self.record_event(
            DevUiEventLevel::Success,
            format!(
                "Sincronização manual com o Drive: {} snapshot(s) copiado(s)",
                count
            ),
            None,
        )
        .await;
        Ok(count)
    }

    /// Sincroniza manualmente todos os snapshots de um jogo específico para a pasta do Drive.
    pub async fn sync_game_to_drive(&self, game_id: Uuid) -> Result<usize> {
        let config_path = self.config_dir.join("config.toml");
        let config = savevault_core::config::Config::load(&config_path).unwrap_or_default();
        let drive_dir = config
            .paths
            .drive_dir
            .ok_or_else(|| anyhow::anyhow!("Nenhuma pasta do Google Drive configurada."))?;

        let count = {
            let conn = self.db.lock().await;
            let mut count = 0usize;
            let mut stmt = conn.prepare(
                "SELECT b.id
                 FROM backups b
                 JOIN files f ON b.file_id = f.id
                 JOIN save_locations sl ON f.save_location_id = sl.id
                 WHERE sl.game_id = ?1 AND b.sync_status != 'copied_to_drive_folder'
                 ORDER BY b.version ASC",
            )?;
            let backup_ids: Vec<Uuid> = stmt
                .query_map(rusqlite::params![game_id.to_string()], |r| {
                    let id_str: String = r.get(0)?;
                    Ok(Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::new_v4()))
                })?
                .filter_map(|r| r.ok())
                .collect();
            drop(stmt);

            for bid in backup_ids {
                if let Ok(true) = crate::pipeline::sync_backup_to_drive_folder(
                    &conn,
                    bid,
                    &self.backup_root,
                    &drive_dir,
                ) {
                    count += 1;
                }
            }
            count
        };

        self.record_event(
            DevUiEventLevel::Success,
            format!(
                "Sincronização do jogo com o Drive: {} snapshot(s) copiado(s)",
                count
            ),
            None,
        )
        .await;

        Ok(count)
    }
}

/// Formata bytes de forma legível.
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
