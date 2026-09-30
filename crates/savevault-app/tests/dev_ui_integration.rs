//! Testes de Integração da Interface de Desenvolvimento (DEV-UI-01).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{watch, Mutex};
use uuid::Uuid;

use savevault_app::pipeline::handle_stable_event;
use savevault_app::ui::{
    start_dev_ui_server, DashboardSummary, DevUiEventLevel, DevUiState, LocationView,
    ReconciliationView, RestoreApiResponse, SnapshotView,
};
use savevault_app::Reconciler;
use savevault_core::models::{Confidence, Game, Platform, SaveLocation, SaveLocationType};
use savevault_db::repositories::{BackupsRepo, FilesRepo, GamesRepo, SaveLocationsRepo};
use savevault_db::Database;
use savevault_monitor::events::{StableFileEvent, StableFileEventKind};

/// Helper para requisições HTTP GET na DEV-UI.
async fn http_get(addr: SocketAddr, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr)
        .await
        .expect("Falha ao conectar no servidor DEV-UI");
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        path, addr
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut resp = String::new();
    stream.read_to_string(&mut resp).await.unwrap();

    let status = resp
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse::<u16>()
        .unwrap();

    let body = resp.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

/// Helper para requisições HTTP POST na DEV-UI.
async fn http_post(addr: SocketAddr, path: &str, json_body: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr)
        .await
        .expect("Falha ao conectar no servidor DEV-UI");
    let req = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nOrigin: http://{}\r\nX-SaveVault-Request: 1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        path,
        addr,
        addr,
        json_body.len(),
        json_body
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut resp = String::new();
    stream.read_to_string(&mut resp).await.unwrap();

    let status = resp
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse::<u16>()
        .unwrap();

    let body = resp.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

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
async fn test_dev_ui_http_server_endpoints_and_shutdown() {
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

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Inicia o servidor em porta dinâmica (0)
    let (addr, server_handle) = start_dev_ui_server(state.clone(), 0, shutdown_rx)
        .await
        .expect("Falha ao iniciar servidor DEV-UI");

    // 1. Testa GET / (Assets HTML)
    let (status, html) = http_get(addr, "/").await;
    assert_eq!(status, 200);
    assert!(html.contains("SaveVault"));
    assert!(html.contains("DEV-UI-01"));

    // 2. Testa GET /api/dashboard
    let (status, dash_body) = http_get(addr, "/api/dashboard").await;
    assert_eq!(status, 200);
    let dash: DashboardSummary = serde_json::from_str(&dash_body).unwrap();
    assert_eq!(dash.app_status, "Online");

    // 3. Testa GET /api/locations
    let (status, locs_body) = http_get(addr, "/api/locations").await;
    assert_eq!(status, 200);
    let locs: Vec<LocationView> = serde_json::from_str(&locs_body).unwrap();
    assert!(locs.is_empty());

    // 4. Testa GET /api/snapshots
    let (status, snaps_body) = http_get(addr, "/api/snapshots").await;
    assert_eq!(status, 200);
    let snaps: Vec<SnapshotView> = serde_json::from_str(&snaps_body).unwrap();
    assert!(snaps.is_empty());

    // 5. Testa GET /api/reconciliation
    let (status, recon_body) = http_get(addr, "/api/reconciliation").await;
    assert_eq!(status, 200);
    let recon: ReconciliationView = serde_json::from_str(&recon_body).unwrap();
    assert_eq!(recon.status, "Idle");

    // 6. Testa GET /api/events
    let (status, events_body) = http_get(addr, "/api/events").await;
    assert_eq!(status, 200);
    assert!(events_body.contains("Servidor DEV-UI iniciado"));

    // 7. Testa POST /api/restore (erro gracioso para snapshot inexistente)
    let bad_payload = serde_json::json!({
        "snapshot_id": Uuid::new_v4().to_string()
    })
    .to_string();
    let (status, restore_body) = http_post(addr, "/api/restore", &bad_payload).await;
    assert_eq!(status, 200);
    let restore_resp: RestoreApiResponse = serde_json::from_str(&restore_body).unwrap();
    assert!(!restore_resp.success);
    assert!(restore_resp.error.unwrap().contains("não encontrado"));

    // 7.1 Testa GET /api/autostart
    let (status, autostart_body) = http_get(addr, "/api/autostart").await;
    assert_eq!(status, 200);
    assert!(autostart_body.contains("enabled"));

    // 7.2 Testa POST /api/autostart/toggle
    let (status, toggle_body) = http_post(addr, "/api/autostart/toggle", "{}").await;
    assert_eq!(status, 200);
    assert!(toggle_body.contains("enabled"));
    // Reverte o toggle para manter o estado limpo
    let _ = http_post(addr, "/api/autostart/toggle", "{}").await;

    // 8. Testa rota 404
    let (status, _) = http_get(addr, "/api/rota_inexistente").await;
    assert_eq!(status, 404);

    // 9. Shutdown gracioso
    let _ = shutdown_tx.send(true);
    let server_res = tokio::time::timeout(Duration::from_secs(3), server_handle).await;
    assert!(server_res.is_ok(), "Servidor demorou para encerrar");
}

#[tokio::test]
async fn test_dev_ui_cors_origin_protection() {
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

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (addr, server_handle) = start_dev_ui_server(state.clone(), 0, shutdown_rx)
        .await
        .expect("Falha ao iniciar servidor DEV-UI");

    async fn send_raw_origin(addr: SocketAddr, origin: &str) -> u16 {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let req = format!(
            "GET /api/dashboard HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\nConnection: close\r\n\r\n",
            addr, origin
        );
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).await.unwrap();
        resp.lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse::<u16>()
            .unwrap()
    }

    // 1. Origens permitidas
    assert_eq!(send_raw_origin(addr, "http://127.0.0.1").await, 200);
    assert_eq!(send_raw_origin(addr, "http://localhost").await, 200);
    assert_eq!(send_raw_origin(addr, "http://127.0.0.1:4040").await, 200);
    assert_eq!(send_raw_origin(addr, "http://localhost:3000").await, 200);

    // 2. Origens maliciosas bloqueadas (403 Forbidden)
    assert_eq!(send_raw_origin(addr, "http://127.0.0.1.attacker.com").await, 403);
    assert_eq!(send_raw_origin(addr, "http://localhost.evil.com").await, 403);
    assert_eq!(send_raw_origin(addr, "http://evil.com").await, 403);
    assert_eq!(send_raw_origin(addr, "https://127.0.0.1").await, 403);

    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(3), server_handle).await;
}
