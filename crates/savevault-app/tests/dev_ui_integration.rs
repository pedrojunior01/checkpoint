//! Testes de Integração da Interface de Desenvolvimento (DEV-UI-01).

use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::sync::{watch, Mutex};
use uuid::Uuid;

use savevault_app::pipeline::handle_stable_event;
use savevault_app::ui::{DevUiEventLevel, DevUiState};
use savevault_app::Reconciler;
use savevault_core::models::{
    Backup, Confidence, FileEntry, Game, Platform, SaveLocation, SaveLocationType, SyncOperation,
    SyncQueueItem, SyncStatus,
};
use savevault_db::repositories::{
    BackupsRepo, FilesRepo, GamesRepo, SaveLocationsRepo, SyncQueueRepo,
};
use savevault_db::Database;
use savevault_monitor::events::{StableFileEvent, StableFileEventKind};

#[tokio::test]
async fn test_dev_ui_state_dashboard_and_views() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let db_path = temp_root.path().join("vault.db");
    let db = Database::open(&db_path).expect("Falha ao abrir banco");

    let backup_dir = temp_root.path().join("backups");
    let save_dir = temp_root.path().join("saves");
    std::fs::create_dir_all(&save_dir).unwrap();
    std::fs::create_dir_all(&backup_dir).unwrap();

    // 1. Cadastra Game e SaveLocation
    let game = Game::new(
        "Cyberpunk Odyssey",
        Platform::Steam,
        Some("999999".into()),
        Confidence::High,
    );
    GamesRepo::new(&db.conn).insert(&game).unwrap();

    let save_loc = SaveLocation::new(game.id, save_dir.clone(), SaveLocationType::Directory);
    let saved_loc = SaveLocationsRepo::new(&db.conn).upsert(&save_loc).unwrap();

    // 2. Cria arquivo e snapshot via pipeline
    let save_file = save_dir.join("save_slot_01.dat");
    std::fs::write(&save_file, "save_conteudo_v1").unwrap();

    let event = StableFileEvent {
        save_location_id: saved_loc.id,
        path: save_file.clone(),
        kind: StableFileEventKind::Created,
        size_bytes: "save_conteudo_v1".len() as u64,
        observed_at: std::time::SystemTime::now(),
    };
    handle_stable_event(&db.conn, &event, &backup_dir, 5)
        .await
        .unwrap();

    let db_mutex = Arc::new(Mutex::new(db.conn));
    let state = DevUiState::new(
        db_mutex,
        db_path.clone(),
        backup_dir.clone(),
        None,
        Duration::from_secs(30),
    );

    // 3. Valida Dashboard
    let dashboard = state.get_dashboard().await.unwrap();
    assert_eq!(dashboard.app_status, "Online");
    assert_eq!(dashboard.db_status, "Connected");
    assert_eq!(dashboard.save_locations_count, 1);
    assert_eq!(dashboard.monitored_files_count, 1);
    assert_eq!(dashboard.snapshots_count, 1);
    assert_eq!(dashboard.watcher_status, "Running");
    assert_eq!(dashboard.reconciler_status, "Running");

    // 4. Valida Save Locations View
    let locations = state.get_locations().await.unwrap();
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].game_name, "Cyberpunk Odyssey");
    assert!(locations[0].exists_on_disk);
    assert_eq!(locations[0].monitored_files_count, 1);

    // 5. Valida Snapshots View
    let snapshots = state.get_snapshots().await.unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].game_name, "Cyberpunk Odyssey");
    assert_eq!(snapshots[0].version, 1);
    assert!(snapshots[0].physical_exists);
}

#[tokio::test]
async fn test_dev_ui_scan_location() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let db_path = temp_root.path().join("vault.db");
    let db = Database::open(&db_path).expect("Falha ao abrir banco");

    let backup_dir = temp_root.path().join("backups");
    let save_dir = temp_root.path().join("game_saves");
    std::fs::create_dir_all(&save_dir).unwrap();

    let game = Game::new("Stardew Valley", Platform::Steam, None, Confidence::High);
    GamesRepo::new(&db.conn).insert(&game).unwrap();

    let save_loc = SaveLocation::new(game.id, save_dir.clone(), SaveLocationType::Directory);
    let saved_loc = SaveLocationsRepo::new(&db.conn).upsert(&save_loc).unwrap();

    // Cria arquivos físicos na pasta
    std::fs::write(save_dir.join("save1.sav"), "dados_1").unwrap();
    std::fs::write(save_dir.join("save2.sav"), "dados_2_longos").unwrap();

    let db_mutex = Arc::new(Mutex::new(db.conn));
    let state = DevUiState::new(db_mutex, db_path, backup_dir, None, Duration::from_secs(30));

    let result = state.scan_location(saved_loc.id).await.unwrap();
    assert_eq!(result.files_found.len(), 2);
    assert!(result.files_found.iter().any(|f| f.filename == "save1.sav"));
    assert!(result.files_found.iter().any(|f| f.filename == "save2.sav"));
}

#[tokio::test]
async fn test_dev_ui_restore_success_and_safety_backup() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let db_path = temp_root.path().join("vault.db");
    let db = Database::open(&db_path).expect("Falha ao abrir banco");

    let backup_dir = temp_root.path().join("backups");
    let save_dir = temp_root.path().join("game_saves");
    std::fs::create_dir_all(&save_dir).unwrap();

    let save_file = save_dir.join("player.sav");

    let game = Game::new("Dark Souls", Platform::Steam, None, Confidence::High);
    GamesRepo::new(&db.conn).insert(&game).unwrap();

    let save_loc = SaveLocation::new(game.id, save_dir.clone(), SaveLocationType::Directory);
    let saved_loc = SaveLocationsRepo::new(&db.conn).upsert(&save_loc).unwrap();

    // v1: arquivo inicial
    std::fs::write(&save_file, "soul_level_1").unwrap();
    let event1 = StableFileEvent {
        save_location_id: saved_loc.id,
        path: save_file.clone(),
        kind: StableFileEventKind::Created,
        size_bytes: "soul_level_1".len() as u64,
        observed_at: std::time::SystemTime::now(),
    };
    handle_stable_event(&db.conn, &event1, &backup_dir, 5)
        .await
        .unwrap();

    // Modifica arquivo no disco para simular estado alterado
    std::fs::write(&save_file, "conteudo_corrompido_ou_novo").unwrap();

    let files_repo = FilesRepo::new(&db.conn);
    let file = files_repo.find_by_path(&save_file).unwrap().unwrap();
    let backups_repo = BackupsRepo::new(&db.conn);
    let all_backups = backups_repo.list_by_file(file.id).unwrap();
    assert_eq!(all_backups.len(), 1);
    let snapshot_id = all_backups[0].id;

    let db_mutex = Arc::new(Mutex::new(db.conn));
    let state = DevUiState::new(db_mutex, db_path, backup_dir, None, Duration::from_secs(30));

    // Executa Restore via DEV-UI
    let resp = state.trigger_restore(snapshot_id, None).await;
    assert!(resp.success);
    assert_eq!(resp.version, Some(1));
    assert!(resp.safety_backup_path.is_some());

    // Verifica que o arquivo no disco voltou para "soul_level_1"
    let restored_content = std::fs::read_to_string(&save_file).unwrap();
    assert_eq!(restored_content, "soul_level_1");

    // Verifica que Safety Backup foi listado na UI
    let safety_groups = state.get_safety_backups().await.unwrap();
    assert_eq!(safety_groups.len(), 1);
    assert_eq!(safety_groups[0].backups.len(), 1);
    assert!(safety_groups[0].backups[0]
        .filename
        .contains(".savevault_safety_"));
}

#[tokio::test]
async fn test_dev_ui_restore_error_handling() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let db_path = temp_root.path().join("vault.db");
    let db = Database::open(&db_path).expect("Falha ao abrir banco");
    let backup_dir = temp_root.path().join("backups");

    let db_mutex = Arc::new(Mutex::new(db.conn));
    let state = DevUiState::new(db_mutex, db_path, backup_dir, None, Duration::from_secs(30));

    let nonexistent_id = Uuid::new_v4();
    let resp = state.trigger_restore(nonexistent_id, None).await;

    assert!(!resp.success);
    assert!(resp.error.is_some());
    assert!(resp.details.is_some());
    assert!(resp.error.unwrap().contains("não encontrado"));

    let events = state.get_events().await;
    assert!(events.iter().any(|e| e.level == DevUiEventLevel::Error));
}

#[tokio::test]
async fn test_dev_ui_reconciliation_trigger() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let db_path = temp_root.path().join("vault.db");
    let db = Database::open(&db_path).expect("Falha ao abrir banco");

    let backup_dir = temp_root.path().join("backups");
    let save_dir = temp_root.path().join("game_saves");
    std::fs::create_dir_all(&save_dir).unwrap();

    let game = Game::new("Terraria", Platform::Steam, None, Confidence::High);
    GamesRepo::new(&db.conn).insert(&game).unwrap();

    let save_loc = SaveLocation::new(game.id, save_dir.clone(), SaveLocationType::Directory);
    SaveLocationsRepo::new(&db.conn).upsert(&save_loc).unwrap();

    // Cria arquivo no disco sem passar pelo pipeline
    std::fs::write(save_dir.join("world.wld"), "dados_do_mundo").unwrap();

    let db_mutex = Arc::new(Mutex::new(db.conn));
    let reconciler = Arc::new(Reconciler::new(db_mutex.clone(), backup_dir.clone(), 5));

    let state = DevUiState::new(
        db_mutex,
        db_path,
        backup_dir,
        Some(reconciler),
        Duration::from_secs(30),
    );

    // Aciona Reconciliação manual
    let rep = state.trigger_reconcile().await.unwrap();
    assert_eq!(rep.locations_scanned, 1);
    assert_eq!(rep.files_seen, 1);
    assert_eq!(rep.files_added, 1);

    let status = state.get_reconciliation_status().await.unwrap();
    assert_eq!(status.files_seen, 1);
    assert_eq!(status.files_added, 1);
    assert!(status.last_run.is_some());
}

#[tokio::test]
async fn test_dev_ui_graceful_shutdown_and_ipc_state() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let db_path = temp_root.path().join("vault.db");
    let db = Database::open(&db_path).expect("Falha ao abrir banco");
    let backup_dir = temp_root.path().join("backups");

    let db_mutex = Arc::new(Mutex::new(db.conn));
    let state = Arc::new(DevUiState::new(
        db_mutex,
        db_path,
        backup_dir,
        None,
        Duration::from_secs(30),
    ));

    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    state.set_shutdown_sender(shutdown_tx).await;

    // Estado inicial ativo
    assert!(state
        .watcher_active
        .load(std::sync::atomic::Ordering::SeqCst));
    assert!(state
        .reconciler_active
        .load(std::sync::atomic::Ordering::SeqCst));
    assert!(!*shutdown_rx.borrow());

    // Aciona shutdown gracioso
    state.trigger_graceful_shutdown().await;

    // Valida flags atômicas e sinal no canal
    assert!(!state
        .watcher_active
        .load(std::sync::atomic::Ordering::SeqCst));
    assert!(!state
        .reconciler_active
        .load(std::sync::atomic::Ordering::SeqCst));
    assert!(*shutdown_rx.borrow_and_update());
}

#[tokio::test]
async fn test_graceful_shutdown_waits_for_in_flight_operations() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let db_path = temp_root.path().join("vault.db");
    let db = Database::open(&db_path).expect("Falha ao abrir banco");
    let backup_dir = temp_root.path().join("backups");

    let db_mutex = Arc::new(Mutex::new(db.conn));
    let state = Arc::new(DevUiState::new(
        db_mutex,
        db_path,
        backup_dir,
        None,
        Duration::from_secs(30),
    ));

    // 1. Simula um worker ocupado com uma operação crítica (snapshot ou restore)
    let in_flight_guard = state
        .register_in_flight_operation()
        .expect("Deve aceitar operação antes do shutdown");
    assert_eq!(state.in_flight_count(), 1);

    // 2. Dispara o shutdown em uma task assíncrona
    let state_clone = state.clone();
    let shutdown_handle = tokio::spawn(async move {
        state_clone
            .wait_graceful_shutdown(Duration::from_secs(3))
            .await
    });

    // 3. Aguarda 150ms: o shutdown já foi solicitado, mas NÃO pode concluir enquanto o worker estiver ativo
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        !shutdown_handle.is_finished(),
        "Shutdown NÃO deve concluir enquanto houver worker ativo!"
    );
    assert!(
        state.is_shutting_down(),
        "Sistema deve marcar que está em shutdown"
    );

    // 4. Nova tentativa de trabalho DEVE ser rejeitada para impedir novas tarefas
    let rejected_op = state.register_in_flight_operation();
    assert!(
        rejected_op.is_none(),
        "Novas operações DEVEM ser rejeitadas durante o shutdown!"
    );

    // 5. Worker finaliza o trabalho crítico (drop do guard)
    drop(in_flight_guard);

    // 6. Agora o shutdown deve concluir com sucesso (sem timeout)
    let clean_exit = tokio::time::timeout(Duration::from_secs(1), shutdown_handle)
        .await
        .expect("Shutdown deveria ter finalizado rapidamente após drop do worker")
        .expect("Task de shutdown falhou");

    assert!(clean_exit, "Shutdown deve relatar conclusão limpa");
    assert_eq!(state.in_flight_count(), 0);
}

#[tokio::test]
async fn test_graceful_shutdown_timeout_when_worker_stuck() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let db_path = temp_root.path().join("vault.db");
    let db = Database::open(&db_path).expect("Falha ao abrir banco");
    let backup_dir = temp_root.path().join("backups");

    let db_mutex = Arc::new(Mutex::new(db.conn));
    let state = Arc::new(DevUiState::new(
        db_mutex,
        db_path,
        backup_dir,
        None,
        Duration::from_secs(30),
    ));

    // Simula worker preso que não conclui dentro do prazo
    let _stuck_guard = state
        .register_in_flight_operation()
        .expect("Deve registrar");

    let clean = state
        .wait_graceful_shutdown(Duration::from_millis(100))
        .await;

    assert!(
        !clean,
        "Shutdown deve retornar false quando timeout for atingido com worker pendente"
    );
    assert_eq!(state.in_flight_count(), 1);
}

#[tokio::test]
async fn test_get_sync_queue_returns_expected_items() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test_ui_sync.db");
    let backup_dir = dir.path().join("backups");
    let db = Database::open(&db_path).unwrap();

    let game_id = Uuid::new_v4();
    let loc_id = Uuid::new_v4();
    let file_id = Uuid::new_v4();
    let backup_id = Uuid::new_v4();
    let queue_id = Uuid::new_v4();

    // 1. Popula dados completos
    let game = Game {
        id: game_id,
        name: "Sync Test Game".to_string(),
        platform: Platform::Steam,
        platform_id: Some("12345".to_string()),
        confidence: Confidence::High,
        created_at: chrono::Utc::now(),
    };
    GamesRepo::new(&db.conn).insert(&game).unwrap();

    let loc = SaveLocation {
        id: loc_id,
        game_id,
        path: std::path::PathBuf::from(r"C:\Games\SyncTest\save.dat"),
        location_type: SaveLocationType::File,
    };
    SaveLocationsRepo::new(&db.conn).insert(&loc).unwrap();

    let file = FileEntry {
        id: file_id,
        save_location_id: loc_id,
        path: std::path::PathBuf::from(r"C:\Games\SyncTest\save.dat"),
        hash: "abc123blake3".to_string(),
        size_bytes: 1024,
        modified_at: chrono::Utc::now(),
    };
    FilesRepo::new(&db.conn).upsert(&file).unwrap();

    let backup = Backup {
        id: backup_id,
        file_id,
        version: 1,
        cloud_path: None,
        drive_file_id: None,
        created_at: chrono::Utc::now(),
        sync_status: SyncStatus::Pending,
    };
    BackupsRepo::new(&db.conn).insert(&backup).unwrap();

    let queue_item = SyncQueueItem {
        id: queue_id,
        backup_id,
        operation: SyncOperation::Upload,
        status: SyncStatus::Pending,
        attempts: 2,
        created_at: chrono::Utc::now(),
    };
    SyncQueueRepo::new(&db.conn).enqueue(&queue_item).unwrap();

    let db_conn = Arc::new(Mutex::new(db.conn));
    let state = Arc::new(DevUiState::new(
        db_conn,
        db_path,
        backup_dir,
        None,
        Duration::from_secs(30),
    ));

    // 2. Invoca get_sync_queue
    let queue_views = state
        .get_sync_queue()
        .await
        .expect("get_sync_queue deve ter sucesso sem erros de SQL");

    // 3. Validações determinísticas
    assert_eq!(queue_views.len(), 1);
    let view = &queue_views[0];
    assert_eq!(view.id, queue_id);
    assert_eq!(view.backup_id, backup_id);
    assert_eq!(view.operation, "upload");
    assert_eq!(view.status, "pending");
    assert_eq!(view.attempts, 2);
    assert_eq!(view.file_name, "save.dat");
    assert_eq!(view.game_name, "Sync Test Game");
    assert!(view.last_error.is_none());
}
