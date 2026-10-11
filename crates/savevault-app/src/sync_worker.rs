//! Worker de Sincronização em Segundo Plano do SaveVault (Desktop & Daemon).
//!
//! Responsável por consumir itens da `sync_queue`, acionar uploads assíncronos
//! para o provedor de nuvem configurado (`LocalFolderProvider`, etc.) sem reter o banco,
//! reagir a eventos de novos snapshots em tempo de execução via canal de notificação
//! e executar reconciliações periódicas da fila para recuperar eventuais falhas transitórias.

use anyhow::Result;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{watch, Mutex, Notify, RwLock};

use savevault_sync::{resolve_active_cloud_provider_with_path, SyncEngine, SyncEngineConfig};

/// Erros emitidos pelo ciclo de vida do `SyncWorker`.
#[derive(Debug, Error)]
pub enum SyncWorkerError {
    #[error("SyncWorker já está em execução nesta instância.")]
    AlreadyRunning,

    #[error("Erro durante o processamento da fila de sincronização: {0}")]
    Engine(#[from] anyhow::Error),
}

/// Parâmetros de configuração do `SyncWorker`.
#[derive(Debug, Clone)]
pub struct SyncWorkerConfig {
    /// Provedor ativo ("local_folder", "none", etc.)
    pub provider: String,
    /// Caminho da pasta de sincronização remota (usado especialmente por `LocalFolderProvider`).
    pub local_sync_path: Option<PathBuf>,
    /// Intervalo de reconciliação periódica da fila de sync.
    pub reconciliation_interval: Duration,
    /// Número máximo de tentativas de upload antes de marcar como permanente.
    pub max_attempts: u32,
    /// Backoff base em milissegundos para retries.
    pub base_backoff_ms: u64,
}

impl Default for SyncWorkerConfig {
    fn default() -> Self {
        Self {
            provider: "local_folder".to_string(),
            local_sync_path: None,
            reconciliation_interval: Duration::from_secs(60),
            max_attempts: 5,
            base_backoff_ms: 2000,
        }
    }
}

impl SyncWorkerConfig {
    /// Configuração acelerada para testes determinísticos sem espera prolongada.
    pub fn fast_for_tests() -> Self {
        Self {
            provider: "local_folder".to_string(),
            local_sync_path: None,
            reconciliation_interval: Duration::from_millis(50),
            max_attempts: 3,
            base_backoff_ms: 10,
        }
    }
}

/// Worker de sincronização em segundo plano.
pub struct SyncWorker {
    pub db: Arc<Mutex<Connection>>,
    pub backup_root: PathBuf,
    config: Arc<RwLock<SyncWorkerConfig>>,
    custom_engine: Option<Arc<SyncEngine>>,
    is_running: Arc<AtomicBool>,
    processing_lock: Arc<Mutex<()>>,
    notify: Arc<Notify>,
}

impl SyncWorker {
    /// Cria uma nova instância de `SyncWorker` conectada ao banco e à pasta de backups.
    pub fn new(db: Arc<Mutex<Connection>>, backup_root: PathBuf, config: SyncWorkerConfig) -> Self {
        Self {
            db,
            backup_root,
            config: Arc::new(RwLock::new(config)),
            custom_engine: None,
            is_running: Arc::new(AtomicBool::new(false)),
            processing_lock: Arc::new(Mutex::new(())),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Cria um `SyncWorker` com um `SyncEngine` customizado (ideal para testes com mocks/falhas controladas).
    pub fn with_engine(
        db: Arc<Mutex<Connection>>,
        engine: Arc<SyncEngine>,
        reconciliation_interval: Duration,
    ) -> Self {
        let backup_root = engine.backup_root().to_path_buf();
        let config = SyncWorkerConfig {
            provider: "custom".to_string(),
            local_sync_path: None,
            reconciliation_interval,
            max_attempts: 3,
            base_backoff_ms: 10,
        };
        Self {
            db,
            backup_root,
            config: Arc::new(RwLock::new(config)),
            custom_engine: Some(engine),
            is_running: Arc::new(AtomicBool::new(false)),
            processing_lock: Arc::new(Mutex::new(())),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Retorna `true` se o loop do worker estiver em execução.
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    /// Notifica o worker sobre a chegada de um novo snapshot na fila, acordando-o imediatamente.
    pub fn notify_new_item(&self) {
        self.notify.notify_one();
    }

    /// Atualiza dinamicamente as configurações do worker (ex.: alteração de provedor ou pasta).
    pub async fn update_config(&self, new_config: SyncWorkerConfig) {
        let mut cfg = self.config.write().await;
        *cfg = new_config;
    }

    /// Processa um lote de itens pendentes na `sync_queue` de forma atômica e sem travar o banco.
    pub async fn process_batch(&self) -> Result<usize> {
        // Evita execuções concorrentes sobrepostas dentro do mesmo worker
        let _guard = match self.processing_lock.try_lock() {
            Ok(g) => g,
            Err(_) => {
                tracing::debug!(
                    "SyncWorker já está processando um lote. Pulando chamada concorrente."
                );
                return Ok(0);
            }
        };

        let count = if let Some(ref engine) = self.custom_engine {
            engine.process_queue_unlocked(&self.db).await?
        } else {
            let (provider_name, local_path, max_attempts, base_backoff_ms) = {
                let cfg = self.config.read().await;
                (
                    cfg.provider.clone(),
                    cfg.local_sync_path.clone(),
                    cfg.max_attempts,
                    cfg.base_backoff_ms,
                )
            };

            if provider_name.to_lowercase() == "none"
                || provider_name.to_lowercase() == "disabled"
                || provider_name.to_lowercase() == "offline"
            {
                return Ok(0);
            }

            let provider = resolve_active_cloud_provider_with_path(
                &provider_name,
                &self.backup_root,
                local_path.as_deref(),
            );

            let engine = SyncEngine::with_config(
                provider,
                self.backup_root.clone(),
                SyncEngineConfig {
                    max_attempts,
                    base_backoff_ms,
                },
            );

            engine.process_queue_unlocked(&self.db).await?
        };

        if count > 0 {
            crate::notifications::notify(
                "checkpoint — Sincronização Concluída",
                format!("{} save(s) sincronizado(s) com sucesso na nuvem.", count),
            );
        }

        Ok(count)
    }

    /// Inicia o ciclo de vida do `SyncWorker`.
    ///
    /// Garante que apenas uma rotina do worker seja executada simultaneamente.
    /// No startup, realiza drenagem imediata de itens pendentes.
    /// Posteriormente, aguarda notificações de novos itens ou o tick periódico de reconciliação.
    pub async fn start(
        &self,
        mut shutdown_rx: watch::Receiver<bool>,
    ) -> Result<(), SyncWorkerError> {
        if self
            .is_running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(SyncWorkerError::AlreadyRunning);
        }

        struct RunningGuard(Arc<AtomicBool>);
        impl Drop for RunningGuard {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _running_guard = RunningGuard(self.is_running.clone());

        tracing::info!("SyncWorker iniciado com sucesso.");

        // Drenagem imediata de itens pendentes no startup
        if let Err(e) = self.process_batch().await {
            tracing::warn!(
                "Erro durante o processamento inicial da fila de sync: {}",
                e
            );
        }

        let interval_duration = {
            let cfg = self.config.read().await;
            cfg.reconciliation_interval
        };

        let mut interval = tokio::time::interval(interval_duration);
        // Descarta o primeiro tick imediato pois já executamos process_batch() acima
        interval.tick().await;

        loop {
            tokio::select! {
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        tracing::info!("SyncWorker recebeu sinal de shutdown gracioso.");
                        break;
                    }
                }
                _ = self.notify.notified() => {
                    tracing::debug!("SyncWorker acionado por evento/notificação de novo item.");
                    if let Err(e) = self.process_batch().await {
                        tracing::warn!("Erro ao processar item notificado no SyncWorker: {}", e);
                    }
                }
                _ = interval.tick() => {
                    tracing::debug!("SyncWorker executando reconciliação periódica da fila de sync.");
                    if let Err(e) = self.process_batch().await {
                        tracing::warn!("Erro durante reconciliação periódica no SyncWorker: {}", e);
                    }
                }
            }
        }

        // Drenagem final no encerramento gracioso
        let _ = self.process_batch().await;
        tracing::info!("SyncWorker encerrado.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use savevault_core::models::{
        Backup, Confidence, FileEntry, Game, Platform, SaveLocation, SaveLocationType,
        SyncOperation, SyncQueueItem, SyncStatus,
    };
    use savevault_db::repositories::{
        BackupsRepo, FilesRepo, GamesRepo, SaveLocationsRepo, SyncQueueRepo,
    };
    use savevault_db::Database;
    use tempfile::tempdir;
    use uuid::Uuid;

    fn setup_test_db() -> Connection {
        let db = Database::open_in_memory().unwrap();
        db.conn
    }

    #[tokio::test]
    async fn test_sync_worker_startup_processes_existing_pending_items() {
        let conn = setup_test_db();
        let tmp = tempdir().unwrap();
        let backup_root = tmp.path().join("backups");
        let remote_root = tmp.path().join("remote");
        std::fs::create_dir_all(&backup_root).unwrap();
        std::fs::create_dir_all(&remote_root).unwrap();

        let file_id = Uuid::new_v4();
        let backup_id = Uuid::new_v4();

        // Insere jogo, local, arquivo e backup
        let games_repo = GamesRepo::new(&conn);
        let game = Game::new("Hollow Knight", Platform::Steam, None, Confidence::High);
        games_repo.insert(&game).unwrap();

        let locs_repo = SaveLocationsRepo::new(&conn);
        let save_loc = SaveLocation::new(
            game.id,
            tmp.path().join("saves"),
            SaveLocationType::Directory,
        );
        let loc_id = save_loc.id;
        locs_repo.insert(&save_loc).unwrap();

        let files_repo = FilesRepo::new(&conn);
        files_repo
            .upsert(&FileEntry {
                id: file_id,
                save_location_id: loc_id,
                path: tmp.path().join("saves/user1.dat"),
                hash: "testhash".to_string(),
                size_bytes: 100,
                modified_at: Utc::now(),
            })
            .unwrap();

        let backups_repo = BackupsRepo::new(&conn);
        backups_repo
            .insert(&Backup {
                id: backup_id,
                file_id,
                version: 1,
                cloud_path: None,
                drive_file_id: None,
                sync_status: SyncStatus::Pending,
                created_at: Utc::now(),
            })
            .unwrap();

        // Cria o arquivo de snapshot no disco local
        let snapshot_file = backup_root
            .join("Hollow Knight")
            .join("v1")
            .join("user1.dat");
        std::fs::create_dir_all(snapshot_file.parent().unwrap()).unwrap();
        std::fs::write(&snapshot_file, b"save_data_123").unwrap();

        // Insere item pendente na fila ANTES de iniciar o worker
        let queue_repo = SyncQueueRepo::new(&conn);
        queue_repo
            .enqueue(&SyncQueueItem {
                id: Uuid::new_v4(),
                backup_id,
                operation: SyncOperation::Upload,
                status: SyncStatus::Pending,
                attempts: 0,
                created_at: Utc::now(),
            })
            .unwrap();

        let db_conn = Arc::new(Mutex::new(conn));
        let worker_config = SyncWorkerConfig {
            provider: "local_folder".to_string(),
            local_sync_path: Some(remote_root.clone()),
            reconciliation_interval: Duration::from_millis(50),
            max_attempts: 3,
            base_backoff_ms: 10,
        };

        let worker = Arc::new(SyncWorker::new(db_conn.clone(), backup_root, worker_config));

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let worker_task = {
            let worker = worker.clone();
            tokio::spawn(async move { worker.start(shutdown_rx).await })
        };

        // Aguarda processamento inicial do startup
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Encerra o worker antes de inspecionar o banco para evitar contenção
        let _ = shutdown_tx.send(true);
        let res = worker_task.await.unwrap();
        assert!(res.is_ok());

        // Verifica que o arquivo foi copiado para o destino remoto
        let remote_file = remote_root
            .join(loc_id.to_string())
            .join(file_id.to_string())
            .join("v1_user1.dat");
        assert!(
            remote_file.exists(),
            "Arquivo remoto deve existir após processamento inicial"
        );
        assert_eq!(std::fs::read(&remote_file).unwrap(), b"save_data_123");

        // Verifica que a fila está limpa
        let conn_lock = db_conn.lock().await;
        let queue = SyncQueueRepo::new(&conn_lock);
        assert!(queue.list_pending().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_sync_worker_prevents_multiple_concurrent_instances() {
        let conn = setup_test_db();
        let tmp = tempdir().unwrap();
        let db_conn = Arc::new(Mutex::new(conn));
        let worker_config = SyncWorkerConfig::fast_for_tests();

        let worker = Arc::new(SyncWorker::new(
            db_conn,
            tmp.path().to_path_buf(),
            worker_config,
        ));

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let worker_clone = worker.clone();
        tokio::spawn(async move {
            let _ = worker_clone.start(shutdown_rx).await;
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(worker.is_running());

        // Segunda tentativa deve falhar com AlreadyRunning
        let (shutdown_tx2, shutdown_rx2) = watch::channel(false);
        let err = worker.start(shutdown_rx2).await.unwrap_err();
        match err {
            SyncWorkerError::AlreadyRunning => {}
            _ => panic!("Esperado AlreadyRunning, obtido: {:?}", err),
        }

        let _ = shutdown_tx.send(true);
        let _ = shutdown_tx2.send(true);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!worker.is_running());
    }
}
