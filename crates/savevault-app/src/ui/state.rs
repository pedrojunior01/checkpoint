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

use savevault_db::repositories::{FilesRepo, GamesRepo, SaveLocationsRepo, SyncQueueRepo};
use savevault_monitor::scanner::SaveScanner;

use crate::reconciler::{Reconciler, ReconciliationError, ReconciliationReport};
use crate::restore::{restore_snapshot_with_db, RestoreError, RestoreRequest};

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

/// Estado e visão do Assistente de Primeiro Acesso (Setup Wizard).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupStatusView {
    pub setup_completed: bool,
    pub cloud_provider: String,
    pub local_sync_path: Option<String>,
    pub detected_cloud_folders: Vec<savevault_cloud::DetectedCloudFolder>,
    pub games_count: usize,
    pub autostart_enabled: bool,
    pub notifications_enabled: bool,
}

/// Jogo descoberto durante o scan do assistente.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredGameView {
    pub name: String,
    pub platform: String,
    pub save_path: String,
}

/// Estado compartilhado e thread-safe da DEV-UI.
pub struct DevUiState {
    pub db: Arc<Mutex<Connection>>,
    pub db_path: PathBuf,
    pub backup_root: PathBuf,
    pub config_path: Option<PathBuf>,
    pub reconciler: Option<Arc<Reconciler>>,
    pub reconciliation_interval: Duration,
    pub watcher_active: AtomicBool,
    pub reconciler_active: AtomicBool,
    event_counter: AtomicU64,
    recent_events: Mutex<VecDeque<DevUiEvent>>,
    last_error: Mutex<Option<String>>,
    last_reconciliation_report: Mutex<Option<ReconciliationReport>>,
    last_reconcile_time: Mutex<Option<DateTime<Utc>>>,
}

impl DevUiState {
    pub fn new(
        db: Arc<Mutex<Connection>>,
        db_path: PathBuf,
        backup_root: PathBuf,
        reconciler: Option<Arc<Reconciler>>,
        reconciliation_interval: Duration,
    ) -> Self {
        Self {
            db,
            db_path,
            backup_root,
            config_path: None,
            reconciler,
            reconciliation_interval,
            watcher_active: AtomicBool::new(true),
            reconciler_active: AtomicBool::new(true),
            event_counter: AtomicU64::new(1),
            recent_events: Mutex::new(VecDeque::with_capacity(100)),
            last_error: Mutex::new(None),
            last_reconciliation_report: Mutex::new(None),
            last_reconcile_time: Mutex::new(None),
        }
    }

    /// Define o caminho do arquivo de configuração para o estado da UI.
    pub fn with_config_path(mut self, path: PathBuf) -> Self {
        self.config_path = Some(path);
        self
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

            let safe_game_name = savevault_backup::snapshot::sanitize_name(&game_name);
            let physical_p = self
                .backup_root
                .join(&safe_game_name)
                .join("latest")
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

        let res = restore_snapshot_with_db(&self.db, &self.backup_root, &req).await;

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

    /// Resolve o caminho efetivo do arquivo de configuração config.toml.
    pub fn resolve_config_path(&self) -> PathBuf {
        if let Some(ref p) = self.config_path {
            p.clone()
        } else {
            let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
            PathBuf::from(appdata).join("SaveVault").join("config.toml")
        }
    }

    /// Carrega a configuração atual a partir do caminho resolvido.
    pub fn get_config(&self) -> savevault_core::config::Config {
        let config_path = self.resolve_config_path();
        savevault_core::config::Config::load(&config_path).unwrap_or_default()
    }

    /// Salva a configuração fornecida no caminho resolvido.
    pub fn save_config(&self, config: &savevault_core::config::Config) -> Result<()> {
        let config_path = self.resolve_config_path();
        config.save(&config_path)
    }

    /// Obtém o status do Assistente de Primeiro Acesso (Setup Wizard).
    pub async fn get_setup_status(&self) -> Result<SetupStatusView> {
        let config = self.get_config();

        let detected_cloud_folders = savevault_cloud::detect_cloud_folders();
        let local_sync_path = config
            .cloud
            .local_sync_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .or_else(|| {
                detected_cloud_folders
                    .first()
                    .map(|d| d.savevault_folder.to_string_lossy().to_string())
            });

        let games_count = {
            let conn = self.db.lock().await;
            let repo = GamesRepo::new(&conn);
            repo.list_all().map(|g| g.len()).unwrap_or(0)
        };

        let autostart_enabled =
            savevault_core::autostart::is_enabled().unwrap_or(config.general.autostart);

        Ok(SetupStatusView {
            setup_completed: config.general.setup_completed,
            cloud_provider: config.cloud.provider,
            local_sync_path,
            detected_cloud_folders,
            games_count,
            autostart_enabled,
            notifications_enabled: config.notifications.enabled,
        })
    }

    /// Executa o scan inicial de jogos durante o assistente de primeiro acesso.
    pub async fn run_setup_discovery(&self) -> Result<Vec<DiscoveredGameView>> {
        self.record_event(
            DevUiEventLevel::Info,
            "Assistente de Configuração: iniciando detecção de jogos e saves...",
            None,
        )
        .await;

        let config_path = self.resolve_config_path();
        let config_dir = config_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        let result = savevault_discovery::run_all(config_dir).await?;

        let _report = {
            let conn = self.db.lock().await;
            crate::orchestrator::persist_discovery_result(&conn, &result)?
        };

        self.record_event(
            DevUiEventLevel::Success,
            format!(
                "Detecção concluída: {} jogos e {} locais mapeados.",
                result.games.len(),
                result.save_locations.len()
            ),
            None,
        )
        .await;

        let conn = self.db.lock().await;
        let games_repo = GamesRepo::new(&conn);
        let locs_repo = SaveLocationsRepo::new(&conn);
        let games = games_repo.list_all()?;
        let mut views = Vec::new();
        for g in games {
            if let Ok(locs) = locs_repo.list_by_game(g.id) {
                for loc in locs {
                    views.push(DiscoveredGameView {
                        name: g.name.clone(),
                        platform: format!("{:?}", g.platform),
                        save_path: loc.path.display().to_string(),
                    });
                }
            }
        }
        Ok(views)
    }

    /// Conclui o assistente de primeiro acesso e persiste as preferências no config.toml.
    pub async fn complete_setup(
        &self,
        provider: Option<String>,
        autostart: Option<bool>,
        notifications: Option<bool>,
        local_sync_path: Option<String>,
    ) -> Result<SetupStatusView> {
        let config_path = self.resolve_config_path();
        let mut config = savevault_core::config::Config::load(&config_path).unwrap_or_default();

        if let Some(p) = provider {
            config.cloud.provider = p;
        }
        if let Some(ref lsp) = local_sync_path {
            let trimmed = lsp.trim();
            if !trimmed.is_empty() {
                let p = PathBuf::from(trimmed);
                let _ = std::fs::create_dir_all(&p);
                config.cloud.local_sync_path = Some(p);
            }
        }
        if let Some(a) = autostart {
            config.general.autostart = a;
            if a {
                let _ = savevault_core::autostart::enable(None);
            } else {
                let _ = savevault_core::autostart::disable();
            }
        }
        if let Some(n) = notifications {
            config.notifications.enabled = n;
        }
        config.general.setup_completed = true;
        config.save(&config_path)?;

        self.record_event(
            DevUiEventLevel::Success,
            "Assistente de Primeiro Acesso concluído com sucesso!",
            Some(format!(
                "Provedor ativo: {}, Iniciar com Windows: {}, Notificações: {}",
                config.cloud.provider, config.general.autostart, config.notifications.enabled
            )),
        )
        .await;

        self.get_setup_status().await
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
