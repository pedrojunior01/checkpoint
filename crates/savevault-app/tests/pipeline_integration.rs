//! Teste de integração ponta a ponta do pipeline completo do SaveVault:
//!
//! Evento de Filesystem
//!     ↓
//! SaveWatcherService (Debounce + Estabilidade)
//!     ↓
//! StableFileEvent
//!     ↓
//! handle_stable_event (Hash BLAKE3 + Deduplicação + SnapshotManager)
//!     ↓
//! Persistência SQLite (FileEntry + Backup)
//!     ↓
//! SyncQueue (SyncQueueItem { backup_id })
//!     ↓
//! SyncEngine::process_queue
//!     ↓
//! MockCloud (Upload validado)

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

use savevault_app::pipeline::handle_stable_event;
use savevault_cloud::CloudProvider;
use savevault_core::models::{
    Confidence, Game, Platform, SaveLocation, SaveLocationType, SyncStatus,
};
use savevault_db::repositories::{
    BackupsRepo, FilesRepo, GamesRepo, SaveLocationsRepo, SyncQueueRepo,
};
use savevault_db::Database;
use savevault_monitor::debouncer::DebounceConfig;
use savevault_monitor::events::{RawFileEvent, RawFileEventKind};
use savevault_monitor::file_access::OsFileAccess;
use savevault_monitor::stability::StabilityConfig;
use savevault_monitor::watcher::{SaveWatcherConfig, SaveWatcherService};
use savevault_sync::{SyncEngine, SyncEngineConfig};

/// Mock determinístico de provedor em nuvem para validação de ponta a ponta.
#[derive(Default, Clone)]
struct MockCloud {
    pub uploads: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

#[async_trait]
impl CloudProvider for MockCloud {
    async fn upload(&self, local_path: &Path, cloud_path: &str) -> Result<String> {
        let content = tokio::fs::read(local_path).await?;
        self.uploads
            .lock()
            .unwrap()
            .insert(cloud_path.to_string(), content);
        let drive_id = format!("mock-drive-id-{}", uuid::Uuid::new_v4().simple());
        Ok(drive_id)
    }

    async fn download(&self, _remote_file_id: &str, _local_path: &Path) -> Result<()> {
        Ok(())
    }

    async fn delete(&self, remote_file_id: &str) -> Result<()> {
        self.uploads.lock().unwrap().remove(remote_file_id);
        Ok(())
    }
}

#[tokio::test]
async fn test_end_to_end_pipeline_from_event_to_cloud() {
    let save_dir = tempdir().unwrap();
    let backup_dir = tempdir().unwrap();

    let db = Database::open_in_memory().unwrap();
    let games_repo = GamesRepo::new(&db.conn);
    let loc_repo = SaveLocationsRepo::new(&db.conn);

    // 1. Configura jogo e local de save no SQLite
    let game = Game::new(
        "Pipeline Hero",
        Platform::Steam,
        Some("42".into()),
        Confidence::High,
    );
    games_repo.insert(&game).unwrap();

    let save_loc = SaveLocation::new(
        game.id,
        save_dir.path().to_path_buf(),
        SaveLocationType::Directory,
    );
    loc_repo.insert(&save_loc).unwrap();

    // 2. Inicializa o serviço de monitoramento com configurações aceleradas para testes
    let mut watcher_config = SaveWatcherConfig::fast_for_tests();
    watcher_config.debounce = DebounceConfig::fast_for_tests();
    watcher_config.stability = StabilityConfig::fast_for_tests();

    let (watcher, raw_tx, mut stable_rx) = SaveWatcherService::start_pipeline(
        watcher_config,
        vec![save_loc.clone()],
        Arc::new(OsFileAccess),
    );

    // 3. Cria arquivo físico de save no disco
    let save_file_path = save_dir.path().join("hero_save.dat");
    let save_payload = b"SAVE_DATA_INITIAL_STAGE_1_LEVEL_10";
    std::fs::write(&save_file_path, save_payload).unwrap();

    // 4. Injeta evento no pipeline de monitoramento (simulando captura do SO)
    raw_tx
        .send(RawFileEvent {
            path: save_file_path.clone(),
            save_location_id: save_loc.id,
            kind: RawFileEventKind::Created,
        })
        .await
        .unwrap();

    // 5. Aguarda emissão do evento estabilizado após debounce + stability check
    let stable_event = tokio::time::timeout(std::time::Duration::from_secs(3), stable_rx.recv())
        .await
        .expect("Timeout aguardando evento estabilizado do monitor")
        .expect("Canal de eventos estáveis fechado prematuramente");

    assert_eq!(stable_event.path, save_file_path);
    assert_eq!(stable_event.save_location_id, save_loc.id);
    assert_eq!(stable_event.size_bytes, save_payload.len() as u64);

    // 6. Processa o evento estável através do pipeline
    let max_versions = 5;
    let res = handle_stable_event(&db.conn, &stable_event, backup_dir.path(), max_versions)
        .await
        .expect("handle_stable_event falhou");

    let (backup, queue_item) = res.expect("Esperado que snapshot e backup fossem gerados");

    // 7. Validação estrita de identificadores: backup_id != file_id
    assert_ne!(
        backup.id, backup.file_id,
        "Identificador de backup deve ser estritamente diferente de file_id"
    );
    assert_eq!(
        queue_item.backup_id, backup.id,
        "SyncQueueItem.backup_id DEVE referenciar Backup.id e NUNCA file_id"
    );
    assert_eq!(backup.version, 1);

    // 8. Valida existência do snapshot físico no diretório de backups
    let files_repo = FilesRepo::new(&db.conn);
    let file_entry = files_repo.find_by_id(backup.file_id).unwrap().unwrap();
    let expected_snapshot_path = backup_dir
        .path()
        .join("Pipeline Hero")
        .join("latest")
        .join("hero_save.dat");
    assert!(
        expected_snapshot_path.exists(),
        "O arquivo de snapshot físico deve existir no caminho {}",
        expected_snapshot_path.display()
    );
    let snapshot_content = std::fs::read(&expected_snapshot_path).unwrap();
    assert_eq!(snapshot_content, save_payload);

    // 9. Executa a sincronização em nuvem via SyncEngine com MockCloud
    let mock_cloud = MockCloud::default();
    let sync_engine = SyncEngine::with_config(
        mock_cloud.clone(),
        backup_dir.path().to_path_buf(),
        SyncEngineConfig::fast_for_tests(),
    );

    let synced_count = sync_engine
        .process_queue(&db.conn)
        .await
        .expect("SyncEngine::process_queue falhou");

    assert_eq!(
        synced_count, 1,
        "Exatamente 1 item deveria ser sincronizado"
    );

    let expected_cloud_path = format!(
        "{}/{}/v1_hero_save.dat",
        file_entry.save_location_id, file_entry.id
    );

    // 10. Valida upload no MockCloud
    {
        let uploads = mock_cloud.uploads.lock().unwrap();
        assert_eq!(uploads.len(), 1);
        let uploaded_data = uploads
            .get(&expected_cloud_path)
            .expect("Arquivo não encontrado no cloud mock");
        assert_eq!(uploaded_data, save_payload);
    }

    // 11. Valida estado final no SQLite: fila vazia e status Synced
    let queue_repo = SyncQueueRepo::new(&db.conn);
    let pending_queue = queue_repo.list_pending().unwrap();
    assert_eq!(
        pending_queue.len(),
        0,
        "Item processado com sucesso deve ser removido da fila pendente"
    );

    let backups_repo = BackupsRepo::new(&db.conn);
    let backup_in_db = backups_repo.find_by_id(backup.id).unwrap().unwrap();
    assert_eq!(backup_in_db.sync_status, SyncStatus::Synced);
    assert_eq!(backup_in_db.cloud_path, Some(expected_cloud_path));
    assert!(backup_in_db.drive_file_id.is_some());

    // 12. Encerramento gracioso do serviço de monitoramento
    watcher.shutdown().await;
}

#[tokio::test]
async fn test_end_to_end_pipeline_deduplication_and_versioning() {
    let save_dir = tempdir().unwrap();
    let backup_dir = tempdir().unwrap();

    let db = Database::open_in_memory().unwrap();
    let games_repo = GamesRepo::new(&db.conn);
    let loc_repo = SaveLocationsRepo::new(&db.conn);

    let game = Game::new("RPG Game", Platform::Gog, None, Confidence::High);
    games_repo.insert(&game).unwrap();

    let save_loc = SaveLocation::new(
        game.id,
        save_dir.path().to_path_buf(),
        SaveLocationType::Directory,
    );
    loc_repo.insert(&save_loc).unwrap();

    let save_file_path = save_dir.path().join("save.bin");
    std::fs::write(&save_file_path, b"STAGE_1").unwrap();

    let watcher_config = SaveWatcherConfig::fast_for_tests();
    let (watcher, raw_tx, mut stable_rx) = SaveWatcherService::start_pipeline(
        watcher_config,
        vec![save_loc.clone()],
        Arc::new(OsFileAccess),
    );

    // Envia evento da versão 1
    raw_tx
        .send(RawFileEvent {
            path: save_file_path.clone(),
            save_location_id: save_loc.id,
            kind: RawFileEventKind::Created,
        })
        .await
        .unwrap();

    let ev1 = stable_rx.recv().await.unwrap();
    let res1 = handle_stable_event(&db.conn, &ev1, backup_dir.path(), 3)
        .await
        .unwrap();
    let (b1, _) = res1.unwrap();
    assert_eq!(b1.version, 1);

    // Simula evento sem alteração de conteúdo (mesmo hash): DEVE ser deduplicado
    raw_tx
        .send(RawFileEvent {
            path: save_file_path.clone(),
            save_location_id: save_loc.id,
            kind: RawFileEventKind::Modified,
        })
        .await
        .unwrap();

    let ev2 = stable_rx.recv().await.unwrap();
    let res2 = handle_stable_event(&db.conn, &ev2, backup_dir.path(), 3)
        .await
        .unwrap();
    assert!(
        res2.is_none(),
        "Arquivo com hash inalterado não deve gerar novo snapshot duplicado"
    );

    // Altera o conteúdo e emite versão 2
    std::fs::write(&save_file_path, b"STAGE_2_BOSS_DEFEATED").unwrap();
    raw_tx
        .send(RawFileEvent {
            path: save_file_path.clone(),
            save_location_id: save_loc.id,
            kind: RawFileEventKind::Modified,
        })
        .await
        .unwrap();

    let ev3 = stable_rx.recv().await.unwrap();
    let res3 = handle_stable_event(&db.conn, &ev3, backup_dir.path(), 3)
        .await
        .unwrap();
    let (b2, _) = res3.unwrap();
    assert_eq!(b2.version, 2);
    assert_eq!(b2.file_id, b1.file_id);

    // Verifica que ambas as versões estão no banco
    let backups_repo = BackupsRepo::new(&db.conn);
    let all_backups = backups_repo.list_by_file(b1.file_id).unwrap();
    assert_eq!(all_backups.len(), 2);
    assert_eq!(all_backups[0].version, 2);
    assert_eq!(all_backups[1].version, 1);

    watcher.shutdown().await;
}
