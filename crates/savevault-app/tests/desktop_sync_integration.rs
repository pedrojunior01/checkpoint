//! Testes de integração do fluxo de sincronização em background do Desktop (Sprint 1: AUD-01).
//!
//! Valida o ciclo completo:
//! - Startup do SyncWorker
//! - Processamento da sync_queue
//! - Fluxo real: Snapshot Local → sync_queue → SyncWorker → LocalFolderProvider → Arquivo no destino remoto
//! - Processamento de itens pendentes já existentes no startup
//! - Tratamento de falha temporária com retry
//! - Prevenção de múltiplos workers concorrentes
//! - Shutdown gracioso e restart com preservação de fila
//! - Proteção de instância única (SingleInstanceGuard)

use chrono::Utc;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::sync::{mpsc, watch, Mutex};
use uuid::Uuid;

use savevault_app::pipeline::{process_file, run_pipeline_with_worker};
use savevault_app::sync_worker::{SyncWorker, SyncWorkerConfig, SyncWorkerError};
use savevault_app::SingleInstanceGuard;
use savevault_cloud::CloudProvider;
use savevault_core::models::{
    Backup, Confidence, FileEntry, Game, Platform, SaveLocation, SaveLocationType, SyncOperation,
    SyncQueueItem, SyncStatus,
};
use savevault_db::repositories::{
    BackupsRepo, FilesRepo, GamesRepo, SaveLocationsRepo, SyncQueueRepo,
};
use savevault_db::Database;
use savevault_monitor::events::{StableFileEvent, StableFileEventKind};
use savevault_sync::{SyncEngine, SyncEngineConfig};

fn setup_db() -> Connection {
    let db = Database::open_in_memory().unwrap();
    db.conn
}

fn create_game_and_location(
    conn: &Connection,
    name: &str,
    save_path: &Path,
) -> (Game, SaveLocation) {
    let games_repo = GamesRepo::new(conn);
    let game = Game::new(name, Platform::Steam, None, Confidence::High);
    games_repo.insert(&game).unwrap();

    let loc_repo = SaveLocationsRepo::new(conn);
    let loc = SaveLocation::new(
        game.id,
        save_path.to_path_buf(),
        SaveLocationType::Directory,
    );
    loc_repo.insert(&loc).unwrap();

    (game, loc)
}

/// Provider simulado que falha nas primeiras N tentativas e depois tem sucesso.
struct FlakyCloudProvider {
    pub failure_count: AtomicU32,
    pub max_failures: u32,
    pub uploaded_files: Arc<std::sync::Mutex<Vec<(PathBuf, String)>>>,
}

impl FlakyCloudProvider {
    fn new(max_failures: u32) -> Self {
        Self {
            failure_count: AtomicU32::new(0),
            max_failures,
            uploaded_files: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl CloudProvider for FlakyCloudProvider {
    async fn upload(
        &self,
        local_path: &Path,
        cloud_path: &str,
    ) -> savevault_cloud::CloudResult<String> {
        let current = self.failure_count.fetch_add(1, Ordering::SeqCst);
        if current < self.max_failures {
            return Err(savevault_cloud::CloudError::Network(format!(
                "Falha temporária de rede (tentativa {})",
                current + 1
            )));
        }

        self.uploaded_files
            .lock()
            .unwrap()
            .push((local_path.to_path_buf(), cloud_path.to_string()));
        Ok(format!(
            "flaky-remote-id-{}",
            uuid::Uuid::new_v4().as_simple()
        ))
    }

    async fn download(
        &self,
        _remote_file_id: &str,
        _local_path: &Path,
    ) -> savevault_cloud::CloudResult<()> {
        Ok(())
    }

    async fn delete(&self, _remote_file_id: &str) -> savevault_cloud::CloudResult<()> {
        Ok(())
    }

    async fn save_manifest(
        &self,
        _manifest: &savevault_cloud::CloudCatalogManifest,
    ) -> savevault_cloud::CloudResult<()> {
        Ok(())
    }
}

/// CASO 1: Startup do Sync Worker e Prevenção de Múltiplos Workers Concorrentes
#[tokio::test]
async fn test_desktop_sync_worker_startup_and_single_worker_prevention() {
    let conn = setup_db();
    let tmp = tempdir().unwrap();
    let db_conn = Arc::new(Mutex::new(conn));
    let worker_config = SyncWorkerConfig::fast_for_tests();

    let worker = Arc::new(SyncWorker::new(
        db_conn,
        tmp.path().to_path_buf(),
        worker_config,
    ));
    assert!(!worker.is_running());

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let worker_clone = worker.clone();
    let handle = tokio::spawn(async move { worker_clone.start(shutdown_rx).await });

    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        worker.is_running(),
        "SyncWorker deve estar ativo após start"
    );

    // Tentativa concorrente de iniciar o mesmo worker deve falhar
    let (shutdown_tx2, shutdown_rx2) = watch::channel(false);
    let second_start_res = worker.start(shutdown_rx2).await;
    assert!(matches!(
        second_start_res,
        Err(SyncWorkerError::AlreadyRunning)
    ));

    let _ = shutdown_tx.send(true);
    let _ = shutdown_tx2.send(true);
    let _ = handle.await.unwrap();

    assert!(
        !worker.is_running(),
        "SyncWorker deve liberar o flag de execução após shutdown"
    );
}

/// CASO 2: Fluxo Real — Novo Snapshot → sync_queue → Sync Worker → LocalFolderProvider → Arquivo Remoto
#[tokio::test]
async fn test_desktop_sync_full_flow_snapshot_to_local_folder_provider() {
    let conn = setup_db();
    let tmp = tempdir().unwrap();
    let save_dir = tmp.path().join("saves");
    let backup_dir = tmp.path().join("backups");
    let remote_sync_dir = tmp.path().join("remote_cloud_folder");

    std::fs::create_dir_all(&save_dir).unwrap();
    std::fs::create_dir_all(&backup_dir).unwrap();
    std::fs::create_dir_all(&remote_sync_dir).unwrap();

    let (_game, loc) = create_game_and_location(&conn, "Stardew Valley", &save_dir);

    // 1. Cria um save físico no diretório do jogo
    let save_file = save_dir.join("save_slot_01.dat");
    std::fs::write(&save_file, b"FARM_SAVE_DATA_VERSION_1").unwrap();

    let db_conn = Arc::new(Mutex::new(conn));

    // 2. Inicializa o SyncWorker com LocalFolderProvider apontando para a pasta remota
    let worker_config = SyncWorkerConfig {
        provider: "local_folder".to_string(),
        local_sync_path: Some(remote_sync_dir.clone()),
        reconciliation_interval: Duration::from_millis(100),
        max_attempts: 3,
        base_backoff_ms: 10,
    };
    let worker = Arc::new(SyncWorker::new(
        db_conn.clone(),
        backup_dir.clone(),
        worker_config,
    ));

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let worker_clone = worker.clone();
    let worker_handle = tokio::spawn(async move { worker_clone.start(shutdown_rx).await });

    // 3. Simula evento estável gerando snapshot via `process_file`
    let event = StableFileEvent {
        path: save_file.clone(),
        save_location_id: loc.id,
        kind: StableFileEventKind::Created,
        size_bytes: 24,
        observed_at: Utc::now().into(),
    };

    let result = process_file(&db_conn, &event, &backup_dir, 5)
        .await
        .unwrap();
    assert!(
        result.is_some(),
        "Snapshot local deve ser criado com sucesso"
    );
    let (backup, _queue_item) = result.unwrap();

    // 4. Notifica o worker sobre o novo snapshot (exatamente como a pipeline faz em produção)
    worker.notify_new_item();

    // Aguarda o upload assíncrono para o LocalFolderProvider
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Encerra o worker para finalizar a sincronização
    let _ = shutdown_tx.send(true);
    let _ = worker_handle.await.unwrap();

    // 5. Valida que o arquivo foi copiado para a pasta remota do LocalFolderProvider
    let remote_target = remote_sync_dir
        .join(loc.id.to_string())
        .join(backup.file_id.to_string())
        .join("v1_save_slot_01.dat");

    assert!(
        remote_target.exists(),
        "Arquivo de save deve existir na pasta remota do LocalFolderProvider: {}",
        remote_target.display()
    );
    assert_eq!(
        std::fs::read(&remote_target).unwrap(),
        b"FARM_SAVE_DATA_VERSION_1"
    );

    // 6. Valida que o item foi concluído e removido da sync_queue
    let conn_lock = db_conn.lock().await;
    let queue_repo = SyncQueueRepo::new(&conn_lock);
    let backups_repo = BackupsRepo::new(&conn_lock);

    let pending = queue_repo.list_pending().unwrap();
    assert!(pending.is_empty(), "sync_queue deve estar vazia após sync");

    let updated_backup = backups_repo.find_by_id(backup.id).unwrap().unwrap();
    assert_eq!(updated_backup.sync_status, SyncStatus::Synced);
    assert!(updated_backup.cloud_path.is_some());
    assert!(updated_backup.drive_file_id.is_some());
}

/// CASO 3: Fila Existente — Processamento Automático de Itens Pendentes no Startup
#[tokio::test]
async fn test_desktop_sync_existing_queue_processed_automatically_on_startup() {
    let conn = setup_db();
    let tmp = tempdir().unwrap();
    let save_dir = tmp.path().join("saves");
    let backup_dir = tmp.path().join("backups");
    let remote_sync_dir = tmp.path().join("remote_cloud_folder");

    std::fs::create_dir_all(&save_dir).unwrap();
    std::fs::create_dir_all(&backup_dir).unwrap();
    std::fs::create_dir_all(&remote_sync_dir).unwrap();

    let (_game, loc) = create_game_and_location(&conn, "Celeste", &save_dir);

    let save_file = save_dir.join("0.celeste");
    std::fs::write(&save_file, b"CELESTE_SAVE_DATA").unwrap();

    let file_id = Uuid::new_v4();
    let backup_id = Uuid::new_v4();

    let files_repo = FilesRepo::new(&conn);
    files_repo
        .upsert(&FileEntry {
            id: file_id,
            save_location_id: loc.id,
            path: save_file.clone(),
            hash: "hash_celeste".to_string(),
            size_bytes: 17,
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

    // Cria o snapshot local físico
    let snapshot_file = backup_dir.join("Celeste").join("v1").join("0.celeste");
    std::fs::create_dir_all(snapshot_file.parent().unwrap()).unwrap();
    std::fs::write(&snapshot_file, b"CELESTE_SAVE_DATA").unwrap();

    // Enfileira ANTES de iniciar a aplicação/worker
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
        local_sync_path: Some(remote_sync_dir.clone()),
        reconciliation_interval: Duration::from_millis(100),
        max_attempts: 3,
        base_backoff_ms: 10,
    };
    let worker = Arc::new(SyncWorker::new(
        db_conn.clone(),
        backup_dir.clone(),
        worker_config,
    ));

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let worker_clone = worker.clone();
    let worker_handle = tokio::spawn(async move { worker_clone.start(shutdown_rx).await });

    // Aguarda apenas o tempo de startup (o worker deve drenar a fila no boot!)
    tokio::time::sleep(Duration::from_millis(150)).await;

    let _ = shutdown_tx.send(true);
    let _ = worker_handle.await.unwrap();

    // Valida que o arquivo apareceu na pasta remota sem nenhuma notificação externa
    let remote_target = remote_sync_dir
        .join(loc.id.to_string())
        .join(file_id.to_string())
        .join("v1_0.celeste");
    assert!(
        remote_target.exists(),
        "Arquivo pendente do boot deve ter sido sincronizado"
    );
}

/// CASO 4: Falha Temporária com Retry e Recuperação
#[tokio::test]
async fn test_desktop_sync_temporary_failure_and_retry_recovery() {
    let conn = setup_db();
    let tmp = tempdir().unwrap();
    let backup_dir = tmp.path().join("backups");
    std::fs::create_dir_all(&backup_dir).unwrap();

    let file_id = Uuid::new_v4();
    let backup_id = Uuid::new_v4();
    let (_game, loc) = create_game_and_location(&conn, "Elden Ring", &tmp.path().join("saves"));

    let files_repo = FilesRepo::new(&conn);
    files_repo
        .upsert(&FileEntry {
            id: file_id,
            save_location_id: loc.id,
            path: tmp.path().join("saves/ER0000.sl2"),
            hash: "hash_er".to_string(),
            size_bytes: 50,
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

    let snapshot_file = backup_dir.join("Elden Ring").join("v1").join("ER0000.sl2");
    std::fs::create_dir_all(snapshot_file.parent().unwrap()).unwrap();
    std::fs::write(&snapshot_file, b"ELDEN_RING_DATA").unwrap();

    let queue_repo = SyncQueueRepo::new(&conn);
    let queue_id = Uuid::new_v4();
    queue_repo
        .enqueue(&SyncQueueItem {
            id: queue_id,
            backup_id,
            operation: SyncOperation::Upload,
            status: SyncStatus::Pending,
            attempts: 0,
            created_at: Utc::now(),
        })
        .unwrap();

    // Provedor que falha 1 vez e na 2ª tem sucesso
    let flaky_provider: Box<dyn CloudProvider> = Box::new(FlakyCloudProvider::new(1));
    let engine = Arc::new(SyncEngine::with_config(
        flaky_provider,
        backup_dir.clone(),
        SyncEngineConfig {
            max_attempts: 3,
            base_backoff_ms: 10,
        },
    ));

    let db_conn = Arc::new(Mutex::new(conn));
    let worker = Arc::new(SyncWorker::with_engine(
        db_conn.clone(),
        engine,
        Duration::from_millis(50),
    ));

    // 1ª execução: falha proposital
    let synced = worker.process_batch().await.unwrap();
    assert_eq!(synced, 0, "Primeira tentativa deve falhar");

    // Item não é perdido, permanece Pending com attempts == 1
    {
        let conn_lock = db_conn.lock().await;
        let q_repo = SyncQueueRepo::new(&conn_lock);
        let item = q_repo.find_by_id(queue_id).unwrap().unwrap();
        assert_eq!(item.status, SyncStatus::Pending);
        assert_eq!(item.attempts, 1, "Tentativa de retry deve ser incrementada");
    }

    // 2ª execução: provedor recuperado, deve ter sucesso
    let synced_retry = worker.process_batch().await.unwrap();
    assert_eq!(
        synced_retry, 1,
        "Segunda tentativa deve ter sucesso após recuperação"
    );

    {
        let conn_lock = db_conn.lock().await;
        let q_repo = SyncQueueRepo::new(&conn_lock);
        assert!(
            q_repo.find_by_id(queue_id).unwrap().is_none(),
            "Item concluído deve ser removido da fila"
        );

        let b_repo = BackupsRepo::new(&conn_lock);
        let b = b_repo.find_by_id(backup_id).unwrap().unwrap();
        assert_eq!(b.sync_status, SyncStatus::Synced);
    }
}

/// CASO 5: Shutdown e Restart — Persistência e Recuperação
#[tokio::test]
async fn test_desktop_sync_shutdown_and_restart_continuity() {
    let conn = setup_db();
    let tmp = tempdir().unwrap();
    let save_dir = tmp.path().join("saves");
    let backup_dir = tmp.path().join("backups");
    let remote_sync_dir = tmp.path().join("remote_cloud_folder");

    std::fs::create_dir_all(&save_dir).unwrap();
    std::fs::create_dir_all(&backup_dir).unwrap();
    std::fs::create_dir_all(&remote_sync_dir).unwrap();

    let (_game, loc) = create_game_and_location(&conn, "Hades", &save_dir);

    let save_file = save_dir.join("Profile1.sav");
    std::fs::write(&save_file, b"HADES_ESCAPE_ATTEMPT").unwrap();

    let db_conn = Arc::new(Mutex::new(conn));

    // 1. Cria snapshot e item na fila
    let event = StableFileEvent {
        path: save_file.clone(),
        save_location_id: loc.id,
        kind: StableFileEventKind::Created,
        size_bytes: 20,
        observed_at: Utc::now().into(),
    };
    let (backup, _item) = process_file(&db_conn, &event, &backup_dir, 5)
        .await
        .unwrap()
        .unwrap();

    // 2. Inicia primeira instância do worker e encerra imediatamente antes do processamento
    let (shutdown_tx1, shutdown_rx1) = watch::channel(false);
    let worker_config = SyncWorkerConfig {
        provider: "local_folder".to_string(),
        local_sync_path: Some(remote_sync_dir.clone()),
        reconciliation_interval: Duration::from_millis(50),
        max_attempts: 3,
        base_backoff_ms: 10,
    };
    let worker1 = Arc::new(SyncWorker::new(
        db_conn.clone(),
        backup_dir.clone(),
        worker_config.clone(),
    ));
    let _ = shutdown_tx1.send(true);
    let _ = worker1.start(shutdown_rx1).await;

    // 3. Simula RESTART do app (cria novo worker com o mesmo banco)
    let (shutdown_tx2, shutdown_rx2) = watch::channel(false);
    let worker2 = Arc::new(SyncWorker::new(
        db_conn.clone(),
        backup_dir.clone(),
        worker_config,
    ));
    let worker2_clone = worker2.clone();
    let handle2 = tokio::spawn(async move { worker2_clone.start(shutdown_rx2).await });

    tokio::time::sleep(Duration::from_millis(150)).await;

    let _ = shutdown_tx2.send(true);
    let _ = handle2.await.unwrap();

    // 4. Confirma que o item pendente foi processado após o restart
    let remote_target = remote_sync_dir
        .join(loc.id.to_string())
        .join(backup.file_id.to_string())
        .join("v1_Profile1.sav");
    assert!(
        remote_target.exists(),
        "Item deve ser processado normalmente após restart"
    );
}

/// CASO 6: Proteção de Instância Única (SingleInstanceGuard)
#[test]
fn test_single_instance_guard_behavior() {
    let mutex_name = format!("Local\\SaveVault_TestMutex_{}", Uuid::new_v4().as_simple());

    // Primeira instância adquire o guard
    let guard1 = SingleInstanceGuard::acquire(&mutex_name);
    assert!(
        guard1.is_some(),
        "Primeira instância deve adquirir o lock com sucesso"
    );

    // Segunda instância com o mesmo nome deve falhar imediatamente
    let guard2 = SingleInstanceGuard::acquire(&mutex_name);
    assert!(
        guard2.is_none(),
        "Segunda instância deve falhar ao tentar o mesmo mutex"
    );

    // Libera a primeira instância
    drop(guard1);

    // Agora uma nova tentativa deve ter sucesso
    let guard3 = SingleInstanceGuard::acquire(&mutex_name);
    assert!(
        guard3.is_some(),
        "Após drop da primeira instância, lock deve ser liberado para nova aquisição"
    );
}

/// Teste de Integração: Watcher Pipeline + SyncWorker
#[tokio::test]
async fn test_pipeline_with_worker_integration() {
    let conn = setup_db();
    let tmp = tempdir().unwrap();
    let save_dir = tmp.path().join("saves");
    let backup_dir = tmp.path().join("backups");
    let remote_sync_dir = tmp.path().join("remote_cloud_folder");

    std::fs::create_dir_all(&save_dir).unwrap();
    std::fs::create_dir_all(&backup_dir).unwrap();
    std::fs::create_dir_all(&remote_sync_dir).unwrap();

    let (_game, loc) = create_game_and_location(&conn, "Tomb Raider", &save_dir);

    let save_file = save_dir.join("save1.dat");
    std::fs::write(&save_file, b"LARA_CROFT_DATA").unwrap();

    let db_conn = Arc::new(Mutex::new(conn));

    let worker_config = SyncWorkerConfig {
        provider: "local_folder".to_string(),
        local_sync_path: Some(remote_sync_dir.clone()),
        reconciliation_interval: Duration::from_millis(100),
        max_attempts: 3,
        base_backoff_ms: 10,
    };
    let worker = Arc::new(SyncWorker::new(
        db_conn.clone(),
        backup_dir.clone(),
        worker_config,
    ));

    let (worker_shutdown_tx, worker_shutdown_rx) = watch::channel(false);
    let worker_clone = worker.clone();
    let worker_task = tokio::spawn(async move { worker_clone.start(worker_shutdown_rx).await });

    let (event_tx, event_rx) = mpsc::channel(16);
    let (pipeline_shutdown_tx, pipeline_shutdown_rx) = watch::channel(false);

    let pipeline_db = db_conn.clone();
    let pipeline_backup_root = backup_dir.clone();
    let pipeline_worker = Some(worker.clone());
    let pipeline_task = tokio::spawn(async move {
        run_pipeline_with_worker(
            pipeline_db,
            event_rx,
            pipeline_backup_root,
            5,
            pipeline_shutdown_rx,
            pipeline_worker,
        )
        .await
    });

    // Envia evento estável pelo canal
    event_tx
        .send(StableFileEvent {
            path: save_file.clone(),
            save_location_id: loc.id,
            kind: StableFileEventKind::Created,
            size_bytes: 15,
            observed_at: Utc::now().into(),
        })
        .await
        .unwrap();

    // Aguarda o pipeline processar e o worker sincronizar
    tokio::time::sleep(Duration::from_millis(250)).await;

    let _ = pipeline_shutdown_tx.send(true);
    let _ = worker_shutdown_tx.send(true);

    let _ = pipeline_task.await.unwrap();
    let _ = worker_task.await.unwrap();

    // Verifica que o arquivo foi criado e enviado para a pasta remota do LocalFolderProvider
    let remote_target = remote_sync_dir.join(loc.id.to_string());

    // Busca arquivos dentro de remote_target
    let mut found = false;
    if remote_target.exists() {
        for entry in std::fs::read_dir(&remote_target).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_dir() {
                for sub in std::fs::read_dir(entry.path()).unwrap() {
                    let sub = sub.unwrap();
                    if sub.file_name().to_string_lossy().contains("save1.dat") {
                        found = true;
                        break;
                    }
                }
            }
        }
    }
    assert!(
        found,
        "Arquivo sincronizado deve ser encontrado na estrutura de pastas da nuvem remota"
    );
}
