//! Teste de integração ponta a ponta da Reconciliação Periódica (MON-01):
//!
//! SaveLocation
//!     ↓
//! Filesystem inicial
//!     ↓
//! Estado inicial via Reconciler
//!     ↓
//! Alteração sem evento (simulando perda de watcher / app desligado)
//!     ↓
//! Reconciliação periódica
//!     ↓
//! Pipeline (process_file)
//!     ↓
//! Snapshot físico em disco
//!     ↓
//! Persistência SQLite & SyncQueue

use anyhow::Result;
use std::sync::Arc;
use tempfile::tempdir;

use savevault_app::{run_reconciliation_loop, Reconciler, ReconciliationConfig};
use savevault_core::models::{Confidence, Game, Platform, SaveLocation, SaveLocationType};
use savevault_db::repositories::{
    BackupsRepo, FilesRepo, GamesRepo, SaveLocationsRepo, SyncQueueRepo,
};
use savevault_db::Database;

#[tokio::test]
async fn test_end_to_end_reconciliation_flow() -> Result<()> {
    // 1. Diretórios temporários isolados
    let temp_root = tempdir()?;
    let saves_dir = temp_root.path().join("gamesaves");
    let backup_dir = temp_root.path().join("backups");
    std::fs::create_dir_all(&saves_dir)?;

    // 2. Banco SQLite isolado em memória
    let db = Database::open_in_memory()?;
    let db_conn = Arc::new(tokio::sync::Mutex::new(db.conn));

    // 3. Cadastra Game e SaveLocation no banco
    let game = Game::new(
        "Eldenhall",
        Platform::Steam,
        Some("482910".into()),
        Confidence::High,
    );
    {
        let conn = db_conn.lock().await;
        GamesRepo::new(&conn).insert(&game)?;
    }

    let location = SaveLocation::new(game.id, saves_dir.clone(), SaveLocationType::Directory);
    {
        let conn = db_conn.lock().await;
        SaveLocationsRepo::new(&conn).insert(&location)?;
    }

    // 4. Cria arquivo inicial no disco antes do rescan
    let file1 = saves_dir.join("save_slot0.sav");
    std::fs::write(&file1, b"SAVE_DATA_LEVEL_1_HP_100")?;

    // 5. Inicializa o Reconciler
    let reconciler = Arc::new(Reconciler::new(db_conn.clone(), backup_dir.clone(), 5));

    // Ciclo 1 de reconciliação: deve descobrir e persistir o arquivo inicial
    let r1 = reconciler
        .reconcile()
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    assert_eq!(r1.locations_scanned, 1);
    assert_eq!(r1.files_seen, 1);
    assert_eq!(r1.files_added, 1);
    assert_eq!(r1.files_changed, 0);
    assert_eq!(r1.errors, 0);

    // Valida persistência SQLite do Ciclo 1
    {
        let conn = db_conn.lock().await;
        let files_repo = FilesRepo::new(&conn);
        let entry1 = files_repo
            .find_by_path(&file1)?
            .expect("FileEntry deve existir");

        let backups_repo = BackupsRepo::new(&conn);
        let backups1 = backups_repo.list_by_file(entry1.id)?;
        assert_eq!(backups1.len(), 1);
        assert_eq!(backups1[0].version, 1);

        let queue_repo = SyncQueueRepo::new(&conn);
        let pending = queue_repo.list_pending()?;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].backup_id, backups1[0].id);
    }

    // 6. Simula mutação no arquivo existente SEM envio de evento pelo watcher
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    std::fs::write(&file1, b"SAVE_DATA_LEVEL_2_BOSS_DEFEATED_HP_95")?;

    // 7. Simula criação de subdiretório e novo arquivo SEM watcher
    let profile_dir = saves_dir.join("profiles");
    std::fs::create_dir_all(&profile_dir)?;
    let file2 = profile_dir.join("meta.json");
    std::fs::write(&file2, b"{\"playtime\": 3600}")?;

    // Ciclo 2 de reconciliação: recupera a alteração e o arquivo no novo subdiretório
    let r2 = reconciler
        .reconcile()
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    assert_eq!(r2.locations_scanned, 1);
    assert_eq!(r2.files_seen, 2);
    assert_eq!(
        r2.files_added, 1,
        "Deve adicionar o novo meta.json no subdiretório"
    );
    assert_eq!(
        r2.files_changed, 1,
        "Deve detectar a modificação em save_slot0.sav"
    );
    assert_eq!(r2.errors, 0);

    // Valida persistência SQLite e snapshot físico do Ciclo 2
    {
        let conn = db_conn.lock().await;
        let files_repo = FilesRepo::new(&conn);
        let entry1 = files_repo
            .find_by_path(&file1)?
            .expect("FileEntry file1 deve existir");
        let entry2 = files_repo
            .find_by_path(&file2)?
            .expect("FileEntry file2 deve existir");

        let backups_repo = BackupsRepo::new(&conn);
        let backups1 = backups_repo.list_by_file(entry1.id)?;
        assert_eq!(backups1.len(), 2, "file1 deve ter 2 versões (v2 e v1)");
        assert_eq!(backups1[0].version, 2);
        assert_eq!(backups1[1].version, 1);

        let backups2 = backups_repo.list_by_file(entry2.id)?;
        assert_eq!(backups2.len(), 1, "file2 deve ter 1 versão (v1)");
        assert_eq!(backups2[0].version, 1);

        let queue_repo = SyncQueueRepo::new(&conn);
        let pending = queue_repo.list_pending()?;
        assert_eq!(
            pending.len(),
            3,
            "Deve conter 3 itens enfileirados para sync"
        );
    }

    // 8. Ciclo 3 de reconciliação: idempotência estrita (sem alterações no filesystem)
    let r3 = reconciler
        .reconcile()
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    assert_eq!(r3.locations_scanned, 1);
    assert_eq!(r3.files_seen, 2);
    assert_eq!(r3.files_added, 0, "Nenhum arquivo novo deve ser adicionado");
    assert_eq!(
        r3.files_changed, 0,
        "Nenhum arquivo deve ser marcado como alterado"
    );
    assert_eq!(r3.files_removed, 0);
    assert_eq!(r3.errors, 0);

    // Valida que idempotência não poluiu o banco
    {
        let conn = db_conn.lock().await;
        let files_repo = FilesRepo::new(&conn);
        let entry1 = files_repo.find_by_path(&file1)?.unwrap();
        let backups_repo = BackupsRepo::new(&conn);
        let backups1 = backups_repo.list_by_file(entry1.id)?;
        assert_eq!(
            backups1.len(),
            2,
            "Idempotência mantida: 2 versões para file1"
        );

        let queue_repo = SyncQueueRepo::new(&conn);
        let pending = queue_repo.list_pending()?;
        assert_eq!(pending.len(), 3, "Idempotência mantida: 3 itens na fila");
    }

    Ok(())
}

#[tokio::test]
async fn test_reconciliation_background_loop_integration() -> Result<()> {
    let temp_root = tempdir()?;
    let saves_dir = temp_root.path().join("saves_bg");
    let backup_dir = temp_root.path().join("backups_bg");
    std::fs::create_dir_all(&saves_dir)?;

    let db = Database::open_in_memory()?;
    let db_conn = Arc::new(tokio::sync::Mutex::new(db.conn));

    let game = Game::new("BGGame", Platform::Generic, None, Confidence::High);
    {
        let conn = db_conn.lock().await;
        GamesRepo::new(&conn).insert(&game)?;
    }

    let location = SaveLocation::new(game.id, saves_dir.clone(), SaveLocationType::Directory);
    {
        let conn = db_conn.lock().await;
        SaveLocationsRepo::new(&conn).insert(&location)?;
    }

    let reconciler = Arc::new(Reconciler::new(db_conn.clone(), backup_dir, 5));
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Loop configurado com intervalo rápido para o teste (50ms)
    let config = ReconciliationConfig::fast_for_tests();
    let loop_handle = tokio::spawn(run_reconciliation_loop(
        reconciler.clone(),
        config,
        shutdown_rx,
    ));

    // Escreve arquivo após o loop já estar rodando
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let save_file = saves_dir.join("live_save.dat");
    std::fs::write(&save_file, b"LIVE_TICK_PAYLOAD")?;

    // Aguarda ticks do background loop
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // Encerra cooperativamente
    shutdown_tx.send(true)?;
    let join_res = tokio::time::timeout(std::time::Duration::from_secs(2), loop_handle).await?;
    assert!(join_res?.is_ok());

    // Valida que o loop em background detectou o arquivo e gravou no DB
    {
        let conn = db_conn.lock().await;
        let files_repo = FilesRepo::new(&conn);
        let entry = files_repo.find_by_path(&save_file)?;
        assert!(
            entry.is_some(),
            "Arquivo criado durante o background loop deve ser persistido"
        );
    }

    Ok(())
}
