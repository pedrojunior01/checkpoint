//! Motor de Reconciliação Periódica do SaveVault (MON-01).
//!
//! Complementar ao watcher de filesystem: varre periodicamente os locais de save
//! configurados utilizando o `SaveScanner`, detectando arquivos novos, modificados
//! e removidos, recuperando alterações que possam ter sido perdidas por indisponibilidade
//! temporária, buffer overflow no debouncer ou modificações enquanto o app estava fechado.

use anyhow::Result;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::watch;

use savevault_core::models::{SaveLocation, SaveLocationType};
use savevault_db::repositories::{FilesRepo, SaveLocationsRepo};
use savevault_monitor::events::{StableFileEvent, StableFileEventKind};
use savevault_monitor::scanner::{SaveScanner, ScanError};

use crate::pipeline::process_file;

/// Configuração do ciclo de reconciliação periódica.
#[derive(Debug, Clone)]
pub struct ReconciliationConfig {
    /// Intervalo entre varreduras completas em background.
    pub interval: Duration,
    /// Profundidade máxima de travessia do scanner.
    pub max_depth: usize,
}

impl Default for ReconciliationConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30),
            max_depth: 10,
        }
    }
}

impl ReconciliationConfig {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            max_depth: 10,
        }
    }

    pub fn fast_for_tests() -> Self {
        Self {
            interval: Duration::from_millis(50),
            max_depth: 10,
        }
    }
}

/// Relatório estatístico de uma execução de reconciliação.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReconciliationReport {
    pub locations_scanned: usize,
    pub files_seen: usize,
    pub files_added: usize,
    pub files_changed: usize,
    pub files_removed: usize,
    pub errors: usize,
    pub duration: Duration,
}

/// Erros possíveis durante a execução da reconciliação.
#[derive(Debug, Error)]
pub enum ReconciliationError {
    #[error("Reconciliação já em andamento. Execução pulada para evitar sobreposição.")]
    AlreadyRunning,

    #[error("Erro de persistência SQLite: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Erro de escaneamento de filesystem: {0}")]
    Scan(#[from] ScanError),

    #[error("Erro no processamento do pipeline: {0}")]
    Pipeline(#[from] anyhow::Error),
}

/// RAII Guard para garantir que o flag atômico de execução seja sempre liberado.
struct ReconcileGuard(Arc<AtomicBool>);

impl Drop for ReconcileGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Reconciliador periódico de saves.
pub struct Reconciler {
    pub db: Arc<tokio::sync::Mutex<Connection>>,
    pub backup_root: PathBuf,
    pub max_versions: u32,
    pub scanner: SaveScanner,
    pub drive_dir: std::sync::RwLock<Option<PathBuf>>,
    is_reconciling: Arc<AtomicBool>,
}

impl Reconciler {
    pub fn new(
        db: Arc<tokio::sync::Mutex<Connection>>,
        backup_root: PathBuf,
        max_versions: u32,
    ) -> Self {
        Self {
            db,
            backup_root,
            max_versions,
            scanner: SaveScanner::default(),
            drive_dir: std::sync::RwLock::new(None),
            is_reconciling: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_scanner(
        db: Arc<tokio::sync::Mutex<Connection>>,
        backup_root: PathBuf,
        max_versions: u32,
        scanner: SaveScanner,
    ) -> Self {
        Self {
            db,
            backup_root,
            max_versions,
            scanner,
            drive_dir: std::sync::RwLock::new(None),
            is_reconciling: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set_drive_dir(&self, drive_dir: Option<PathBuf>) {
        if let Ok(mut lock) = self.drive_dir.write() {
            *lock = drive_dir;
        }
    }

    pub fn with_drive_dir(self, drive_dir: Option<PathBuf>) -> Self {
        self.set_drive_dir(drive_dir);
        self
    }

    /// Retorna se há uma reconciliação atualmente em execução.
    pub fn is_reconciling(&self) -> bool {
        self.is_reconciling.load(Ordering::SeqCst)
    }

    /// Executa reconciliação de todas as `SaveLocation`s cadastradas no banco de dados.
    pub async fn reconcile(&self) -> Result<ReconciliationReport, ReconciliationError> {
        let locations = {
            let conn = self.db.lock().await;
            let loc_repo = SaveLocationsRepo::new(&conn);
            loc_repo.list_all()?
        };
        self.reconcile_locations(&locations).await
    }

    /// Executa a reconciliação para a lista informada de `SaveLocation`s.
    pub async fn reconcile_locations(
        &self,
        locations: &[SaveLocation],
    ) -> Result<ReconciliationReport, ReconciliationError> {
        // Garantia de não-sobreposição: impede duas reconciliações simultâneas
        if self
            .is_reconciling
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            tracing::warn!(
                "Reconciliação já em andamento. Execução pulada para evitar sobreposição."
            );
            return Err(ReconciliationError::AlreadyRunning);
        }

        let _guard = ReconcileGuard(Arc::clone(&self.is_reconciling));
        let start = Instant::now();
        let mut report = ReconciliationReport::default();

        tracing::info!(
            "Iniciando ciclo de reconciliação para {} local(is) de save...",
            locations.len()
        );

        for location in locations {
            // 1. Verificação prévia de existência e permissão do caminho base (para File e Directory)
            if location.location_type != SaveLocationType::Glob {
                match std::fs::metadata(&location.path) {
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        tracing::debug!(
                            "Local de save '{}' ainda não existe no disco (aguardando criação pelo jogo).",
                            location.path.display()
                        );
                        continue;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                        tracing::warn!(
                            "Permissão negada (PermissionDenied) ao acessar '{}': {}",
                            location.path.display(),
                            e
                        );
                        report.errors += 1;
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Erro de I/O ao inspecionar '{}': {}",
                            location.path.display(),
                            e
                        );
                        report.errors += 1;
                        continue;
                    }
                }
            }

            // 2. Executa o scanner existente para descobrir arquivos físicos
            let discovered = match self.scanner.scan(location) {
                Ok(files) => files,
                Err(e) => {
                    tracing::warn!(
                        "Falha ao escanear localização '{}': {}",
                        location.path.display(),
                        e
                    );
                    report.errors += 1;
                    continue;
                }
            };

            report.locations_scanned += 1;
            report.files_seen += discovered.len();

            // 3. Consulta arquivos previamente persistidos para esta SaveLocation
            let existing_entries = {
                let conn = self.db.lock().await;
                let files_repo = FilesRepo::new(&conn);
                match files_repo.list_by_location(location.id) {
                    Ok(entries) => entries,
                    Err(e) => {
                        tracing::error!(
                            "Erro ao consultar arquivos do banco para location {}: {}",
                            location.id,
                            e
                        );
                        report.errors += 1;
                        continue;
                    }
                }
            };

            // Mapeia por caminho normalizado (case-folded para Windows)
            let mut existing_map = std::collections::HashMap::new();
            for entry in existing_entries {
                let norm = savevault_core::path::normalize_windows_path(&entry.path)
                    .unwrap_or_else(|_| entry.path.clone());
                let key = norm.to_string_lossy().to_lowercase();
                existing_map.insert(key, entry);
            }

            let mut seen_in_fs = std::collections::HashSet::new();

            // 4. Itera sobre os arquivos descobertos no filesystem
            for disc in discovered {
                let norm_path = savevault_core::path::normalize_windows_path(&disc.path)
                    .unwrap_or_else(|_| disc.path.clone());
                let lookup_key = norm_path.to_string_lossy().to_lowercase();
                seen_in_fs.insert(lookup_key.clone());

                let existing = existing_map.get(&lookup_key);

                let needs_processing = match existing {
                    None => {
                        // Arquivo novo: não existe no DB
                        true
                    }
                    Some(entry) => {
                        // Checagem barata de metadados: tamanho e modified_at
                        let disc_modified: chrono::DateTime<chrono::Utc> = disc.modified_at.into();
                        let size_differs = entry.size_bytes != disc.size_bytes;
                        let time_differs =
                            (entry.modified_at - disc_modified).num_seconds().abs() > 0;

                        size_differs || time_differs
                    }
                };

                if needs_processing {
                    let is_new = existing.is_none();
                    let event = StableFileEvent {
                        path: disc.path.clone(),
                        save_location_id: location.id,
                        kind: if is_new {
                            StableFileEventKind::Created
                        } else {
                            StableFileEventKind::Modified
                        },
                        size_bytes: disc.size_bytes,
                        observed_at: disc.modified_at,
                    };

                    match process_file(&self.db, &event, &self.backup_root, self.max_versions).await
                    {
                        Ok(Some((backup, _item))) => {
                            if is_new {
                                report.files_added += 1;
                            } else {
                                report.files_changed += 1;
                            }
                            tracing::info!(
                                "Reconciliação: snapshot v{} criado para {} (novo: {})",
                                backup.version,
                                disc.path.display(),
                                is_new
                            );
                        }
                        Ok(None) => {
                            // Arquivo inalterado (mesmo hash apurado pelo pipeline)
                            tracing::debug!(
                                "Reconciliação: conteúdo inalterado para {}",
                                disc.path.display()
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                "Reconciliação: falha ao processar {}: {}",
                                disc.path.display(),
                                e
                            );
                            report.errors += 1;
                        }
                    }
                }
            }

            // 5. Arquivos Removidos: existem no banco mas não foram vistos no filesystem
            // Importante: NÃO apagamos registros, FileEntry ou snapshots. Preservamos o histórico.
            for (lookup_key, entry) in existing_map {
                if !seen_in_fs.contains(&lookup_key) {
                    report.files_removed += 1;
                    tracing::info!(
                        "Reconciliação: arquivo ausente no filesystem detectado: {} (id: {}) - histórico preservado.",
                        entry.path.display(),
                        entry.id
                    );
                }
            }
        }

        // Reconciliação opcional com a pasta configurada do Google Drive
        let maybe_drive = { self.drive_dir.read().ok().and_then(|lock| lock.clone()) };
        if let Some(ref ddir) = maybe_drive {
            let conn = self.db.lock().await;
            if let Ok(copied) =
                crate::pipeline::sync_all_pending_to_drive_folder(&conn, &self.backup_root, ddir)
            {
                if copied > 0 {
                    tracing::info!("Reconciliação: {} snapshot(s) pendente(s) sincronizado(s) com a pasta do Drive.", copied);
                }
            }
        }

        report.duration = start.elapsed();

        tracing::info!(
            "Ciclo de reconciliação concluído em {:?}: {} local(is), {} arquivo(s) visto(s), {} adicionado(s), {} modificado(s), {} removido(s), {} erro(s).",
            report.duration,
            report.locations_scanned,
            report.files_seen,
            report.files_added,
            report.files_changed,
            report.files_removed,
            report.errors
        );

        Ok(report)
    }
}

/// Loop assíncrono Tokio para agendamento periódico da reconciliação.
pub async fn run_reconciliation_loop(
    reconciler: Arc<Reconciler>,
    config: ReconciliationConfig,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<()> {
    let mut timer = tokio::time::interval(config.interval);
    // Evita acúmulo de ticks durante reconciliações longas
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    tracing::info!(
        "Serviço de reconciliação periódica ativo com intervalo de {:?}.",
        config.interval
    );

    loop {
        tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    tracing::info!("Shutdown recebido no serviço de reconciliação periódica.");
                    break;
                }
            }
            _ = timer.tick() => {
                match reconciler.reconcile().await {
                    Ok(report) => {
                        tracing::debug!(
                            "Reconciliação executada: {} vistos, {} novos, {} alterados, {} erros.",
                            report.files_seen,
                            report.files_added,
                            report.files_changed,
                            report.errors
                        );
                    }
                    Err(ReconciliationError::AlreadyRunning) => {
                        tracing::warn!("Reconciliação pulada pois a execução anterior ainda está ativa.");
                    }
                    Err(e) => {
                        tracing::error!("Erro durante ciclo de reconciliação: {}", e);
                    }
                }
            }
        }
    }

    tracing::info!("Serviço de reconciliação periódica encerrado com sucesso.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use savevault_core::models::{Confidence, Game, Platform, SaveLocation, SaveLocationType};
    use savevault_db::repositories::{
        BackupsRepo, FilesRepo, GamesRepo, SaveLocationsRepo, SyncQueueRepo,
    };
    use savevault_db::Database;
    use std::sync::atomic::Ordering;
    use tempfile::tempdir;

    // -----------------------------------------------------------------------
    // Teste 1: Arquivo novo recuperado
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_1_reconciliation_recovers_new_file() {
        let dir = tempdir().unwrap();
        let saves_dir = dir.path().join("saves");
        std::fs::create_dir_all(&saves_dir).unwrap();
        let save_file = saves_dir.join("player.sav");
        std::fs::write(&save_file, b"PLAYER_DATA_V1").unwrap();

        let db = Database::open_in_memory().unwrap();
        let game = Game::new("HeroGame", Platform::Steam, None, Confidence::High);
        GamesRepo::new(&db.conn).insert(&game).unwrap();
        let loc = SaveLocation::new(game.id, saves_dir.clone(), SaveLocationType::Directory);
        SaveLocationsRepo::new(&db.conn).insert(&loc).unwrap();

        let db_conn = Arc::new(tokio::sync::Mutex::new(db.conn));
        let backup_root = dir.path().join("backups");
        let reconciler = Reconciler::new(db_conn.clone(), backup_root.clone(), 5);

        let report = reconciler.reconcile_locations(&[loc]).await.unwrap();
        assert_eq!(report.files_seen, 1);
        assert_eq!(report.files_added, 1);
        assert_eq!(report.files_changed, 0);

        let conn = db_conn.lock().await;
        let files_repo = FilesRepo::new(&conn);
        let entry = files_repo.find_by_path(&save_file).unwrap();
        assert!(entry.is_some());
        let entry = entry.unwrap();

        let backups_repo = BackupsRepo::new(&conn);
        let backups = backups_repo.list_by_file(entry.id).unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(backups[0].version, 1);
    }

    // -----------------------------------------------------------------------
    // Teste 2: Arquivo modificado detectado
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_2_reconciliation_recovers_modified_file() {
        let dir = tempdir().unwrap();
        let saves_dir = dir.path().join("saves");
        std::fs::create_dir_all(&saves_dir).unwrap();
        let save_file = saves_dir.join("player.sav");
        std::fs::write(&save_file, b"PLAYER_DATA_V1").unwrap();

        let db = Database::open_in_memory().unwrap();
        let game = Game::new("HeroGame", Platform::Steam, None, Confidence::High);
        GamesRepo::new(&db.conn).insert(&game).unwrap();
        let loc = SaveLocation::new(game.id, saves_dir.clone(), SaveLocationType::Directory);
        SaveLocationsRepo::new(&db.conn).insert(&loc).unwrap();

        let db_conn = Arc::new(tokio::sync::Mutex::new(db.conn));
        let backup_root = dir.path().join("backups");
        let reconciler = Reconciler::new(db_conn.clone(), backup_root.clone(), 5);

        // 1. Estado inicial
        reconciler
            .reconcile_locations(std::slice::from_ref(&loc))
            .await
            .unwrap();

        // 2. Modifica arquivo
        tokio::time::sleep(Duration::from_millis(20)).await;
        std::fs::write(&save_file, b"PLAYER_DATA_V2_ADVANCED_LEVEL").unwrap();

        // 3. Reconciliação detecta a alteração
        let report = reconciler.reconcile_locations(&[loc]).await.unwrap();
        assert_eq!(report.files_seen, 1);
        assert_eq!(report.files_added, 0);
        assert_eq!(report.files_changed, 1);

        let conn = db_conn.lock().await;
        let files_repo = FilesRepo::new(&conn);
        let entry = files_repo.find_by_path(&save_file).unwrap().unwrap();
        let backups_repo = BackupsRepo::new(&conn);
        let backups = backups_repo.list_by_file(entry.id).unwrap();
        assert_eq!(backups.len(), 2);
        assert_eq!(backups[0].version, 2);
        assert_eq!(backups[1].version, 1);
    }

    // -----------------------------------------------------------------------
    // Teste 3: Evento perdido recuperado pelo rescan
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_3_reconciliation_recovers_lost_event() {
        let dir = tempdir().unwrap();
        let saves_dir = dir.path().join("saves");
        std::fs::create_dir_all(&saves_dir).unwrap();
        let save_file = saves_dir.join("save.dat");
        std::fs::write(&save_file, b"ORIGINAL_STATE").unwrap();

        let db = Database::open_in_memory().unwrap();
        let game = Game::new("Game", Platform::Generic, None, Confidence::High);
        GamesRepo::new(&db.conn).insert(&game).unwrap();
        let loc = SaveLocation::new(game.id, saves_dir.clone(), SaveLocationType::Directory);
        SaveLocationsRepo::new(&db.conn).insert(&loc).unwrap();

        let db_conn = Arc::new(tokio::sync::Mutex::new(db.conn));
        let backup_root = dir.path().join("backups");
        let reconciler = Reconciler::new(db_conn.clone(), backup_root.clone(), 5);

        // Inicializa
        reconciler
            .reconcile_locations(std::slice::from_ref(&loc))
            .await
            .unwrap();

        // Simula evento perdido: arquivo alterado sem watcher
        tokio::time::sleep(Duration::from_millis(20)).await;
        std::fs::write(&save_file, b"MUTATED_WITHOUT_EVENT").unwrap();

        let report = reconciler.reconcile_locations(&[loc]).await.unwrap();
        assert_eq!(report.files_changed, 1);

        let conn = db_conn.lock().await;
        let files_repo = FilesRepo::new(&conn);
        let entry = files_repo.find_by_path(&save_file).unwrap().unwrap();
        let backups_repo = BackupsRepo::new(&conn);
        let backups = backups_repo.list_by_file(entry.id).unwrap();
        assert_eq!(backups.len(), 2);
        assert_eq!(backups[0].version, 2);
    }

    // -----------------------------------------------------------------------
    // Teste 4: Idempotência de execuções sucessivas
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_4_reconciliation_idempotency() {
        let dir = tempdir().unwrap();
        let saves_dir = dir.path().join("saves");
        std::fs::create_dir_all(&saves_dir).unwrap();
        let save_file = saves_dir.join("static.sav");
        std::fs::write(&save_file, b"STABLE_PAYLOAD").unwrap();

        let db = Database::open_in_memory().unwrap();
        let game = Game::new("StaticGame", Platform::Generic, None, Confidence::High);
        GamesRepo::new(&db.conn).insert(&game).unwrap();
        let loc = SaveLocation::new(game.id, saves_dir.clone(), SaveLocationType::Directory);
        SaveLocationsRepo::new(&db.conn).insert(&loc).unwrap();

        let db_conn = Arc::new(tokio::sync::Mutex::new(db.conn));
        let backup_root = dir.path().join("backups");
        let reconciler = Reconciler::new(db_conn.clone(), backup_root.clone(), 5);

        // Execução 1: descobre e adiciona
        let r1 = reconciler
            .reconcile_locations(std::slice::from_ref(&loc))
            .await
            .unwrap();
        assert_eq!(r1.files_added, 1);

        // Execução 2: sem alterações
        let r2 = reconciler
            .reconcile_locations(std::slice::from_ref(&loc))
            .await
            .unwrap();
        assert_eq!(r2.files_added, 0);
        assert_eq!(r2.files_changed, 0);

        // Execução 3: sem alterações
        let r3 = reconciler
            .reconcile_locations(std::slice::from_ref(&loc))
            .await
            .unwrap();
        assert_eq!(r3.files_added, 0);
        assert_eq!(r3.files_changed, 0);

        let conn = db_conn.lock().await;
        let files_repo = FilesRepo::new(&conn);
        let entry = files_repo.find_by_path(&save_file).unwrap().unwrap();
        let backups_repo = BackupsRepo::new(&conn);
        let backups = backups_repo.list_by_file(entry.id).unwrap();
        assert_eq!(backups.len(), 1, "Não deve criar versões duplicadas");

        let queue_repo = SyncQueueRepo::new(&conn);
        let queue = queue_repo.list_pending().unwrap();
        assert_eq!(queue.len(), 1, "Não deve criar itens duplicados na fila");
    }

    // -----------------------------------------------------------------------
    // Teste 5: SaveLocation inacessível não derruba as demais
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_5_reconciliation_inaccessible_location_handled_cleanly() {
        let dir = tempdir().unwrap();
        let saves_a = dir.path().join("saves_a");
        let saves_c = dir.path().join("saves_c");
        std::fs::create_dir_all(&saves_a).unwrap();
        std::fs::create_dir_all(&saves_c).unwrap();
        std::fs::write(saves_a.join("a.sav"), b"DATA_A").unwrap();
        std::fs::write(saves_c.join("c.sav"), b"DATA_C").unwrap();

        let db = Database::open_in_memory().unwrap();
        let game = Game::new("MultiLocGame", Platform::Generic, None, Confidence::High);
        GamesRepo::new(&db.conn).insert(&game).unwrap();

        let loc_a = SaveLocation::new(game.id, saves_a, SaveLocationType::Directory);
        SaveLocationsRepo::new(&db.conn).insert(&loc_a).unwrap();
        // Location B com tentativa de path traversal em glob
        let loc_b = SaveLocation::new(
            game.id,
            PathBuf::from(r"C:\..\traversal_attempt\*.dat"),
            SaveLocationType::Glob,
        );
        let loc_c = SaveLocation::new(game.id, saves_c, SaveLocationType::Directory);
        SaveLocationsRepo::new(&db.conn).insert(&loc_c).unwrap();

        let db_conn = Arc::new(tokio::sync::Mutex::new(db.conn));
        let backup_root = dir.path().join("backups");
        let reconciler = Reconciler::new(db_conn, backup_root, 5);

        let report = reconciler
            .reconcile_locations(&[loc_a, loc_b, loc_c])
            .await
            .unwrap();

        assert_eq!(
            report.locations_scanned, 2,
            "Loc A e Loc C devem ter sido escaneadas"
        );
        assert_eq!(report.errors, 1, "Loc B deve registrar exatamente 1 erro");
        assert_eq!(
            report.files_added, 2,
            "Arquivos de Loc A e Loc C devem ter sido salvos"
        );
    }

    // -----------------------------------------------------------------------
    // Teste 6: Shutdown limpo do loop de reconciliação
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_6_reconciliation_clean_shutdown() {
        let dir = tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let db_conn = Arc::new(tokio::sync::Mutex::new(db.conn));
        let reconciler = Arc::new(Reconciler::new(db_conn, dir.path().join("backups"), 5));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let config = ReconciliationConfig::fast_for_tests();
        let handle = tokio::spawn(run_reconciliation_loop(reconciler, config, shutdown_rx));

        tokio::time::sleep(Duration::from_millis(20)).await;
        shutdown_tx.send(true).unwrap();

        let res = tokio::time::timeout(Duration::from_secs(1), handle).await;
        assert!(
            res.is_ok(),
            "Shutdown deve terminar graciosamente sem travar"
        );
        assert!(res.unwrap().unwrap().is_ok());
    }

    // -----------------------------------------------------------------------
    // Teste 7: Reconciliações não sobrepostas
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_7_reconciliations_never_overlap() {
        let dir = tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let db_conn = Arc::new(tokio::sync::Mutex::new(db.conn));
        let reconciler = Arc::new(Reconciler::new(db_conn, dir.path().join("backups"), 5));

        // Força flag como já executando
        reconciler.is_reconciling.store(true, Ordering::SeqCst);

        let res = reconciler.reconcile_locations(&[]).await;
        assert!(matches!(res, Err(ReconciliationError::AlreadyRunning)));

        // Libera flag
        reconciler.is_reconciling.store(false, Ordering::SeqCst);
        let res2 = reconciler.reconcile_locations(&[]).await;
        assert!(res2.is_ok());
    }

    // -----------------------------------------------------------------------
    // Teste 8: Diretório criado posteriormente pelo jogo
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_8_reconciliation_discovers_later_created_subdirectory() {
        let dir = tempdir().unwrap();
        let saves_root = dir.path().join("gamesaves");
        std::fs::create_dir_all(&saves_root).unwrap();

        let db = Database::open_in_memory().unwrap();
        let game = Game::new("DynamicGame", Platform::Steam, None, Confidence::High);
        GamesRepo::new(&db.conn).insert(&game).unwrap();
        let loc = SaveLocation::new(game.id, saves_root.clone(), SaveLocationType::Directory);
        SaveLocationsRepo::new(&db.conn).insert(&loc).unwrap();

        let db_conn = Arc::new(tokio::sync::Mutex::new(db.conn));
        let backup_root = dir.path().join("backups");
        let reconciler = Reconciler::new(db_conn.clone(), backup_root, 5);

        // Startup: diretório vazio
        let r1 = reconciler
            .reconcile_locations(std::slice::from_ref(&loc))
            .await
            .unwrap();
        assert_eq!(r1.files_seen, 0);

        // Jogo cria subpasta de perfil mais tarde
        let subprofile = saves_root.join("Profile_10");
        std::fs::create_dir_all(&subprofile).unwrap();
        let new_save = subprofile.join("save_slot.dat");
        std::fs::write(&new_save, b"SLOT_10_DATA").unwrap();

        // Próximo ciclo descobre o arquivo na subpasta
        let r2 = reconciler.reconcile_locations(&[loc]).await.unwrap();
        assert_eq!(r2.files_seen, 1);
        assert_eq!(r2.files_added, 1);

        let conn = db_conn.lock().await;
        let files_repo = FilesRepo::new(&conn);
        assert!(files_repo.find_by_path(&new_save).unwrap().is_some());
    }
}
