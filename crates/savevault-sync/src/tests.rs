//! Testes unitários do motor de sincronização (SyncEngine).
//!
//! Cobre exaustivamente:
//! - Caso A: Sucesso ponta a ponta (fila -> backup -> file -> upload -> Synced)
//! - Caso B: Backup inexistente (não apaga silenciosamente; gera estado observável Failed)
//! - Caso C: Arquivo físico inexistente no disco
//! - Caso D: Falha temporária da cloud (retry e incremento de tentativas)
//! - Caso E: Sucesso após retry
//! - Caso F: Identificação estrita onde `backup_id != file_id`

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use async_trait::async_trait;
use chrono::Utc;
use rusqlite::Connection;
use tempfile::tempdir;
use uuid::Uuid;

use savevault_cloud::CloudProvider;
use savevault_core::models::{
    Backup, Confidence, FileEntry, Game, Platform, SaveLocation, SaveLocationType,
    SyncOperation, SyncQueueItem, SyncStatus,
};
use savevault_db::Database;
use savevault_db::repositories::{BackupsRepo, FilesRepo, GamesRepo, SaveLocationsRepo, SyncQueueRepo};

use crate::{SyncEngine, SyncEngineConfig};

// ---------------------------------------------------------------------------
// MOCK CLOUD PROVIDER
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
struct MockCloudProvider {
    uploaded_files: Arc<Mutex<Vec<(PathBuf, String)>>>,
    deleted_files: Arc<Mutex<Vec<String>>>,
    fail_uploads_remaining: Arc<AtomicUsize>,
    permanent_failure: Arc<AtomicBool>,
}

impl MockCloudProvider {
    fn new() -> Self {
        Self::default()
    }

    fn with_temporary_failures(failures: usize) -> Self {
        let mock = Self::default();
        mock.fail_uploads_remaining.store(failures, Ordering::SeqCst);
        mock
    }

    fn with_permanent_failure() -> Self {
        let mock = Self::default();
        mock.permanent_failure.store(true, Ordering::SeqCst);
        mock
    }

    fn uploads(&self) -> Vec<(PathBuf, String)> {
        self.uploaded_files.lock().unwrap().clone()
    }

    fn deleted(&self) -> Vec<String> {
        self.deleted_files.lock().unwrap().clone()
    }
}

#[async_trait]
impl CloudProvider for MockCloudProvider {
    async fn upload(&self, local_path: &Path, cloud_path: &str) -> anyhow::Result<String> {
        if self.permanent_failure.load(Ordering::SeqCst) {
            anyhow::bail!("Erro de rede permanente simulado no CloudProvider");
        }

        let remaining = self.fail_uploads_remaining.load(Ordering::SeqCst);
        if remaining > 0 {
            self.fail_uploads_remaining.fetch_sub(1, Ordering::SeqCst);
            anyhow::bail!("Erro temporário simulado de rede/timeout na Cloud");
        }

        self.uploaded_files
            .lock()
            .unwrap()
            .push((local_path.to_path_buf(), cloud_path.to_string()));
        let remote_id = format!("drive-id-{}", Uuid::new_v4().simple());
        Ok(remote_id)
    }

    async fn download(&self, _remote_file_id: &str, _local_path: &Path) -> anyhow::Result<()> {
        Ok(())
    }

    async fn delete(&self, remote_file_id: &str) -> anyhow::Result<()> {
        self.deleted_files.lock().unwrap().push(remote_file_id.to_string());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// FIXTURE AUXILIAR
// ---------------------------------------------------------------------------

fn setup_test_db() -> Connection {
    Database::open_in_memory().unwrap().conn
}

fn create_test_hierarchy(
    conn: &Connection,
    backup_root: &Path,
    filename: &str,
    file_bytes: &[u8],
) -> (Game, SaveLocation, FileEntry, Backup) {
    let games_repo = GamesRepo::new(conn);
    let loc_repo = SaveLocationsRepo::new(conn);
    let files_repo = FilesRepo::new(conn);
    let backups_repo = BackupsRepo::new(conn);

    let game = Game::new("SyncTestGame", Platform::Steam, Some("12345".into()), Confidence::High);
    games_repo.insert(&game).unwrap();

    let loc = SaveLocation {
        id: Uuid::new_v4(),
        game_id: game.id,
        path: PathBuf::from(r"C:\Saves\SyncTestGame"),
        location_type: SaveLocationType::Directory,
    };
    loc_repo.insert(&loc).unwrap();

    let file_id = Uuid::new_v4();
    let file_entry = FileEntry {
        id: file_id,
        save_location_id: loc.id,
        path: PathBuf::from(format!(r"C:\Saves\SyncTestGame\{}", filename)),
        hash: "dummy_blake3_hash".into(),
        size_bytes: file_bytes.len() as u64,
        modified_at: Utc::now(),
    };
    files_repo.upsert(&file_entry).unwrap();

    let backup_id = Uuid::new_v4();
    let backup = Backup {
        id: backup_id,
        file_id,
        version: 1,
        cloud_path: None,
        drive_file_id: None,
        sync_status: SyncStatus::Pending,
        created_at: Utc::now(),
    };
    backups_repo.insert(&backup).unwrap();

    // Cria o arquivo físico na estrutura correta: <backup_root>/<safe_game_name>/latest/<filename>
    // Deve espelhar exatamente o que o SnapshotManager e o SyncEngine esperam encontrar no disco.
    let safe_name = savevault_backup::snapshot::sanitize_name("SyncTestGame");
    let version_dir = backup_root.join(&safe_name).join("latest");
    std::fs::create_dir_all(&version_dir).unwrap();
    let disk_file = version_dir.join(filename);
    std::fs::write(&disk_file, file_bytes).unwrap();

    (game, loc, file_entry, backup)
}

// ---------------------------------------------------------------------------
// TESTES OBRIGATÓRIOS DO SYNC ENGINE
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_case_a_successful_upload_flow() {
    let conn = setup_test_db();
    let temp = tempdir().unwrap();
    let backup_root = temp.path().to_path_buf();

    let (_game, _loc, _file, backup) =
        create_test_hierarchy(&conn, &backup_root, "savegame.sav", b"savedata123");

    let queue_repo = SyncQueueRepo::new(&conn);
    let queue_item = SyncQueueItem {
        id: Uuid::new_v4(),
        backup_id: backup.id,
        operation: SyncOperation::Upload,
        status: SyncStatus::Pending,
        attempts: 0,
        created_at: Utc::now(),
    };
    queue_repo.enqueue(&queue_item).unwrap();

    let mock_cloud = MockCloudProvider::new();
    let engine = SyncEngine::with_config(
        mock_cloud.clone(),
        backup_root,
        SyncEngineConfig::fast_for_tests(),
    );

    let synced = engine.process_queue(&conn).await.unwrap();
    assert_eq!(synced, 1, "Exatamente 1 item deve ser sincronizado com sucesso");

    // Prova que upload foi registrado na cloud
    let uploads = mock_cloud.uploads();
    assert_eq!(uploads.len(), 1);
    assert!(uploads[0].1.contains("v1_savegame.sav"), "cloud_path deve conter o nome versionado do arquivo");

    // Prova que item foi removido da fila após sucesso
    let remaining_queue = queue_repo.list_pending().unwrap();
    assert!(remaining_queue.is_empty(), "Fila deve estar vazia após upload bem-sucedido");

    // Prova que o status do Backup no banco foi atualizado para Synced com cloud_path e drive_file_id preenchidos
    let backups_repo = BackupsRepo::new(&conn);
    let updated_backup = backups_repo.find_by_id(backup.id).unwrap().unwrap();
    assert_eq!(updated_backup.sync_status, SyncStatus::Synced);
    assert!(updated_backup.cloud_path.is_some());
    assert!(updated_backup.drive_file_id.is_some());
    assert!(updated_backup.drive_file_id.as_ref().unwrap().starts_with("drive-id-"));
}

#[tokio::test]
async fn test_case_b_missing_backup_does_not_silently_delete_queue() {
    let conn = setup_test_db();
    let temp = tempdir().unwrap();
    let backup_root = temp.path().to_path_buf();

    let (_game, _loc, _file, backup) =
        create_test_hierarchy(&conn, &backup_root, "missing_test.sav", b"data");

    let queue_repo = SyncQueueRepo::new(&conn);
    let queue_item_id = Uuid::new_v4();

    let queue_item = SyncQueueItem {
        id: queue_item_id,
        backup_id: backup.id,
        operation: SyncOperation::Upload,
        status: SyncStatus::Pending,
        attempts: 0,
        created_at: Utc::now(),
    };
    queue_repo.enqueue(&queue_item).unwrap();

    // Simula a perda do backup no banco (ex.: corrupção ou remoção física da linha de backup)
    conn.execute_batch(
        "PRAGMA foreign_keys = OFF; DELETE FROM backups; PRAGMA foreign_keys = ON;",
    )
    .unwrap();

    let mock_cloud = MockCloudProvider::new();
    let engine = SyncEngine::with_config(
        mock_cloud.clone(),
        backup_root,
        SyncEngineConfig::fast_for_tests(),
    );

    let synced = engine.process_queue(&conn).await.unwrap();
    assert_eq!(synced, 0, "Nenhum upload deve ser realizado");
    assert!(mock_cloud.uploads().is_empty(), "Mock cloud não deve receber uploads");

    // O item NÃO deve ter sido deletado silenciosamente.
    // Ele deve ter seu status atualizado para Failed no banco e ser observável.
    let pending_items = queue_repo.list_pending().unwrap();
    assert!(pending_items.is_empty(), "Não deve mais estar como Pending");

    let updated_item = queue_repo
        .find_by_id(queue_item_id)
        .unwrap()
        .expect("Item da fila NÃO pode ter sido deletado silenciosamente!");

    assert_eq!(
        updated_item.status,
        SyncStatus::Failed,
        "Item deve ser marcado como failed para observabilidade"
    );
    assert_eq!(updated_item.attempts, 1);
}

#[tokio::test]
async fn test_case_c_missing_physical_file_fails_deterministically() {
    let conn = setup_test_db();
    let temp = tempdir().unwrap();
    let backup_root = temp.path().to_path_buf();

    let (_game, _loc, _file, backup) =
        create_test_hierarchy(&conn, &backup_root, "corrupted.sav", b"data");

    // Remove propositalmente o arquivo do disco local para simular ausência física
    let safe_name = savevault_backup::snapshot::sanitize_name("SyncTestGame");
    let disk_file = backup_root
        .join(&safe_name)
        .join("latest")
        .join("corrupted.sav");
    std::fs::remove_file(disk_file).unwrap();

    let queue_repo = SyncQueueRepo::new(&conn);
    let queue_item = SyncQueueItem {
        id: Uuid::new_v4(),
        backup_id: backup.id,
        operation: SyncOperation::Upload,
        status: SyncStatus::Pending,
        attempts: 0,
        created_at: Utc::now(),
    };
    queue_repo.enqueue(&queue_item).unwrap();

    let mock_cloud = MockCloudProvider::new();
    let mut config = SyncEngineConfig::fast_for_tests();
    config.max_attempts = 2;

    let engine = SyncEngine::with_config(mock_cloud.clone(), backup_root, config);

    // Tentativa 1: falha ao não encontrar o arquivo no disco, incrementa attempts
    let synced = engine.process_queue(&conn).await.unwrap();
    assert_eq!(synced, 0);

    let pending = queue_repo.list_pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].attempts, 1);

    // Tentativa 2: atinge max_attempts (2) e é marcado como Failed
    let synced2 = engine.process_queue(&conn).await.unwrap();
    assert_eq!(synced2, 0);

    let pending2 = queue_repo.list_pending().unwrap();
    assert!(pending2.is_empty(), "Não deve mais estar pendente após atingir max_attempts");

    let backups_repo = BackupsRepo::new(&conn);
    let failed_backup = backups_repo.find_by_id(backup.id).unwrap().unwrap();
    assert_eq!(failed_backup.sync_status, SyncStatus::Failed);
}

#[tokio::test]
async fn test_case_d_temporary_cloud_failure_triggers_retry_and_preserves_item() {
    let conn = setup_test_db();
    let temp = tempdir().unwrap();
    let backup_root = temp.path().to_path_buf();

    let (_game, _loc, _file, backup) =
        create_test_hierarchy(&conn, &backup_root, "retry.sav", b"retry_data");

    let queue_repo = SyncQueueRepo::new(&conn);
    let queue_item = SyncQueueItem {
        id: Uuid::new_v4(),
        backup_id: backup.id,
        operation: SyncOperation::Upload,
        status: SyncStatus::Pending,
        attempts: 0,
        created_at: Utc::now(),
    };
    queue_repo.enqueue(&queue_item).unwrap();

    // Configura o mock para falhar 1 vez
    let mock_cloud = MockCloudProvider::with_temporary_failures(1);
    let engine = SyncEngine::with_config(
        mock_cloud.clone(),
        backup_root,
        SyncEngineConfig::fast_for_tests(),
    );

    let synced = engine.process_queue(&conn).await.unwrap();
    assert_eq!(synced, 0, "Primeira tentativa falhou na cloud");

    // Item permanece na fila como Pending com tentativa incrementada
    let pending = queue_repo.list_pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].attempts, 1);
    assert_eq!(pending[0].status, SyncStatus::Pending);
}

#[tokio::test]
async fn test_case_e_success_after_retry() {
    let conn = setup_test_db();
    let temp = tempdir().unwrap();
    let backup_root = temp.path().to_path_buf();

    let (_game, _loc, _file, backup) =
        create_test_hierarchy(&conn, &backup_root, "recover.sav", b"recover_data");

    let queue_repo = SyncQueueRepo::new(&conn);
    let queue_item = SyncQueueItem {
        id: Uuid::new_v4(),
        backup_id: backup.id,
        operation: SyncOperation::Upload,
        status: SyncStatus::Pending,
        attempts: 0,
        created_at: Utc::now(),
    };
    queue_repo.enqueue(&queue_item).unwrap();

    // 1 falha temporária
    let mock_cloud = MockCloudProvider::with_temporary_failures(1);
    let engine = SyncEngine::with_config(
        mock_cloud.clone(),
        backup_root,
        SyncEngineConfig::fast_for_tests(),
    );

    // Ciclo 1: falha
    let s1 = engine.process_queue(&conn).await.unwrap();
    assert_eq!(s1, 0);

    // Ciclo 2: sucesso após retry
    let s2 = engine.process_queue(&conn).await.unwrap();
    assert_eq!(s2, 1);

    // Fila esvaziada e backup marcado como Synced
    assert!(queue_repo.list_pending().unwrap().is_empty());
    let backups_repo = BackupsRepo::new(&conn);
    let synced_backup = backups_repo.find_by_id(backup.id).unwrap().unwrap();
    assert_eq!(synced_backup.sync_status, SyncStatus::Synced);
}

#[tokio::test]
async fn test_case_f_strict_identification_backup_id_is_distinct_from_file_id() {
    let conn = setup_test_db();
    let temp = tempdir().unwrap();
    let backup_root = temp.path().to_path_buf();

    let (_game, _loc, file, backup) =
        create_test_hierarchy(&conn, &backup_root, "strict_id.sav", b"strict_data");

    // Prova matemática fundamental que os dois UUIDs são comprovadamente distintos
    assert_ne!(
        backup.id, file.id,
        "backup.id e file.id DEVEM ser IDs distintos para testar a prevenção de confusão semântica"
    );

    let queue_repo = SyncQueueRepo::new(&conn);
    let queue_item = SyncQueueItem {
        id: Uuid::new_v4(),
        backup_id: backup.id, // O item da fila recebe EXCLUSIVAMENTE o backup_id
        operation: SyncOperation::Upload,
        status: SyncStatus::Pending,
        attempts: 0,
        created_at: Utc::now(),
    };
    queue_repo.enqueue(&queue_item).unwrap();

    let mock_cloud = MockCloudProvider::new();
    let engine = SyncEngine::with_config(
        mock_cloud.clone(),
        backup_root,
        SyncEngineConfig::fast_for_tests(),
    );

    // Se o código confundisse backup_id com file_id:
    // 1. Procuraria WHERE file_id = backup.id e falharia; OU
    // 2. Procuraria na pasta <backup_root>/<backup.id>/v1/ em vez de <file.id>/v1/ e falharia.
    let synced = engine.process_queue(&conn).await.unwrap();
    assert_eq!(
        synced, 1,
        "SyncEngine deve localizar Backup via backup_id E arquivo no disco via backup.file_id"
    );

    let uploads = mock_cloud.uploads();
    assert_eq!(uploads.len(), 1);
    let (disk_path, cloud_path) = &uploads[0];

    // Na nova arquitetura, o caminho físico usa o nome sanitizado do jogo (não o file_id).
    // O file_id aparece apenas no cloud_path lógico (garantia de rastreabilidade remota).
    let safe_game_name = savevault_backup::snapshot::sanitize_name("SyncTestGame");
    assert!(
        disk_path.to_string_lossy().contains(&safe_game_name),
        "Caminho local deve conter o nome do jogo sanitizado ({}), mas foi: {}",
        safe_game_name,
        disk_path.display()
    );

    // Verifica que o cloud_path contém file.id
    assert!(
        cloud_path.contains(&file.id.to_string()),
        "Cloud path deve conter file_id ({}), mas foi: {}",
        file.id,
        cloud_path
    );
}

#[tokio::test]
async fn test_case_g_permanent_cloud_failure_marks_failed_after_max_attempts() {
    let conn = setup_test_db();
    let temp = tempdir().unwrap();
    let backup_root = temp.path().to_path_buf();

    let (_game, _loc, _file, backup) =
        create_test_hierarchy(&conn, &backup_root, "perm_fail.sav", b"data");

    let queue_repo = SyncQueueRepo::new(&conn);
    let queue_item = SyncQueueItem {
        id: Uuid::new_v4(),
        backup_id: backup.id,
        operation: SyncOperation::Upload,
        status: SyncStatus::Pending,
        attempts: 0,
        created_at: Utc::now(),
    };
    queue_repo.enqueue(&queue_item).unwrap();

    let mock_cloud = MockCloudProvider::with_permanent_failure();
    let mut config = SyncEngineConfig::fast_for_tests();
    config.max_attempts = 2;

    let engine = SyncEngine::with_config(mock_cloud, backup_root, config);

    // Tentativa 1
    let s1 = engine.process_queue(&conn).await.unwrap();
    assert_eq!(s1, 0);
    assert_eq!(queue_repo.list_pending().unwrap().len(), 1);

    // Tentativa 2 -> atinge max_attempts (2) e falha permanentemente
    let s2 = engine.process_queue(&conn).await.unwrap();
    assert_eq!(s2, 0);
    assert!(queue_repo.list_pending().unwrap().is_empty());

    let failed_item = queue_repo.find_by_id(queue_item.id).unwrap().unwrap();
    assert_eq!(failed_item.status, SyncStatus::Failed);
    assert_eq!(failed_item.attempts, 2);
}

#[tokio::test]
async fn test_case_h_delete_uses_drive_file_id_not_cloud_path() {
    let conn = setup_test_db();
    let temp = tempdir().unwrap();
    let backup_root = temp.path().to_path_buf();

    let (_game, _loc, _file, backup) =
        create_test_hierarchy(&conn, &backup_root, "delete_target.sav", b"data");

    // Configura o backup como já sincronizado, tendo AMBOS cloud_path e drive_file_id
    let backups_repo = BackupsRepo::new(&conn);
    let expected_drive_id = "drive-file-uuid-777";
    let logical_cloud_path = "loc-123/file-456/v1_delete_target.sav";
    backups_repo
        .update_cloud_metadata(backup.id, logical_cloud_path, expected_drive_id)
        .unwrap();

    let queue_repo = SyncQueueRepo::new(&conn);
    let queue_item = SyncQueueItem {
        id: Uuid::new_v4(),
        backup_id: backup.id,
        operation: SyncOperation::Delete,
        status: SyncStatus::Pending,
        attempts: 0,
        created_at: Utc::now(),
    };
    queue_repo.enqueue(&queue_item).unwrap();

    let mock_cloud = MockCloudProvider::new();
    let engine = SyncEngine::with_config(
        mock_cloud.clone(),
        backup_root,
        SyncEngineConfig::fast_for_tests(),
    );

    let deleted_count = engine.process_queue(&conn).await.unwrap();
    assert_eq!(deleted_count, 0, "Delete não incrementa contador de upload/synced");

    // Verifica que o método delete da cloud recebeu EXATAMENTE o drive_file_id, e NUNCA o cloud_path
    let deleted = mock_cloud.deleted();
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0], expected_drive_id, "Delete DEVE receber drive_file_id");
    assert_ne!(deleted[0], logical_cloud_path, "Delete NUNCA deve receber cloud_path");

    // Verifica que a fila foi limpa
    assert!(queue_repo.list_pending().unwrap().is_empty());
}

#[tokio::test]
async fn test_case_i_upload_succeeds_but_db_fails_preserves_queue() {
    let conn = setup_test_db();
    let temp = tempdir().unwrap();
    let backup_root = temp.path().to_path_buf();

    let (_game, _loc, _file, backup) =
        create_test_hierarchy(&conn, &backup_root, "partial_fail.sav", b"data");

    let queue_repo = SyncQueueRepo::new(&conn);
    let queue_item = SyncQueueItem {
        id: Uuid::new_v4(),
        backup_id: backup.id,
        operation: SyncOperation::Upload,
        status: SyncStatus::Pending,
        attempts: 0,
        created_at: Utc::now(),
    };
    queue_repo.enqueue(&queue_item).unwrap();

    let mock_cloud = MockCloudProvider::new();
    let engine = SyncEngine::with_config(
        mock_cloud.clone(),
        backup_root,
        SyncEngineConfig::fast_for_tests(),
    );

    // Simulamos a condição de falha no banco tornando a tabela backups de somente leitura
    // através de um trigger que impede UPDATE em backups
    conn.execute_batch(
        "CREATE TRIGGER prevent_backup_update BEFORE UPDATE ON backups \
         BEGIN \
             SELECT RAISE(FAIL, 'DB write failure simulated'); \
         END;",
    )
    .unwrap();

    // Executa a fila: upload na cloud deve ocorrer, mas o update no banco vai falhar
    let result = engine.process_queue(&conn).await;
    assert!(result.is_err(), "A falha na persistência DB deve propagar erro");

    // Evidência de falha parcial:
    // 1. Arquivo foi enviado para a cloud
    assert_eq!(mock_cloud.uploads().len(), 1);

    // 2. Fila NÃO foi limpa (o item continua registrado)
    let pending = queue_repo.list_pending().unwrap();
    assert_eq!(pending.len(), 1, "Item continua na fila para retry");

    // 3. Backup no DB não possui drive_file_id (estado local não avançou)
    let backups_repo = BackupsRepo::new(&conn);
    let b = backups_repo.find_by_id(backup.id).unwrap().unwrap();
    assert!(b.drive_file_id.is_none());
    assert_eq!(b.sync_status, SyncStatus::Pending);
}

#[tokio::test]
async fn test_case_j_delete_clears_cloud_metadata_when_local_remains() {
    let conn = setup_test_db();
    let temp = tempdir().unwrap();
    let backup_root = temp.path().to_path_buf();

    let (_game, _loc, _file, backup) =
        create_test_hierarchy(&conn, &backup_root, "keep_local.sav", b"data");

    let backups_repo = BackupsRepo::new(&conn);
    backups_repo
        .update_cloud_metadata(backup.id, "cloud/keep_local.sav", "drive_id_preserve_local")
        .unwrap();
    backups_repo
        .update_sync_status(backup.id, &SyncStatus::Synced)
        .unwrap();

    let queue_repo = SyncQueueRepo::new(&conn);
    let queue_item = SyncQueueItem {
        id: Uuid::new_v4(),
        backup_id: backup.id,
        operation: SyncOperation::Delete,
        status: SyncStatus::Pending,
        attempts: 0,
        created_at: Utc::now(),
    };
    queue_repo.enqueue(&queue_item).unwrap();

    let mock_cloud = MockCloudProvider::new();
    let engine = SyncEngine::with_config(
        mock_cloud.clone(),
        backup_root,
        SyncEngineConfig::fast_for_tests(),
    );

    engine.process_queue(&conn).await.unwrap();

    // 1. Remoto foi deletado
    assert_eq!(mock_cloud.deleted(), vec!["drive_id_preserve_local"]);

    // 2. Fila foi limpa
    assert!(queue_repo.list_pending().unwrap().is_empty());

    // 3. Metadados remotos foram limpos mas backup local PERMANECE
    let updated_backup = backups_repo.find_by_id(backup.id).unwrap().expect("Backup local deve continuar existindo");
    assert!(updated_backup.cloud_path.is_none(), "cloud_path deve ser limpo após delete");
    assert!(updated_backup.drive_file_id.is_none(), "drive_file_id deve ser limpo após delete");
    assert_eq!(updated_backup.sync_status, SyncStatus::PendingOffline, "sync_status deve ser pending_offline");
}

#[tokio::test]
async fn test_case_k_delete_pruned_backup_removes_db_record() {
    let conn = setup_test_db();
    let temp = tempdir().unwrap();
    let backup_root = temp.path().to_path_buf();

    let (_game, _loc, _file, backup) =
        create_test_hierarchy(&conn, &backup_root, "pruned_target.sav", b"data");

    let backups_repo = BackupsRepo::new(&conn);
    backups_repo
        .update_cloud_metadata(backup.id, "cloud/pruned_target.sav", "drive_id_pruned_item")
        .unwrap();

    // Simula pruning local: arquivo em disco foi podado/removido
    let safe_name = savevault_backup::snapshot::sanitize_name("SyncTestGame");
    let snapshot_file = backup_root
        .join(&safe_name)
        .join("latest")
        .join("pruned_target.sav");
    assert!(snapshot_file.exists());
    std::fs::remove_file(&snapshot_file).unwrap();

    let queue_repo = SyncQueueRepo::new(&conn);
    let queue_item = SyncQueueItem {
        id: Uuid::new_v4(),
        backup_id: backup.id,
        operation: SyncOperation::Delete,
        status: SyncStatus::Pending,
        attempts: 0,
        created_at: Utc::now(),
    };
    queue_repo.enqueue(&queue_item).unwrap();

    let mock_cloud = MockCloudProvider::new();
    let engine = SyncEngine::with_config(
        mock_cloud.clone(),
        backup_root,
        SyncEngineConfig::fast_for_tests(),
    );

    engine.process_queue(&conn).await.unwrap();

    // 1. Remoto foi deletado
    assert_eq!(mock_cloud.deleted(), vec!["drive_id_pruned_item"]);

    // 2. Fila foi limpa
    assert!(queue_repo.list_pending().unwrap().is_empty());

    // 3. Backup podado foi completamente removido do SQLite
    assert!(backups_repo.find_by_id(backup.id).unwrap().is_none(), "Backup podado localmente e deletado remotamente deve ser removido do SQLite");
}

#[tokio::test]
async fn test_case_l_atomic_upload_success_commits_metadata_and_deletes_queue_item() {
    let conn = setup_test_db();
    let temp = tempdir().unwrap();
    let backup_root = temp.path().to_path_buf();

    let (_game, _loc, _file, backup) =
        create_test_hierarchy(&conn, &backup_root, "atomic_sync.sav", b"data");

    let queue_repo = SyncQueueRepo::new(&conn);
    let queue_item = SyncQueueItem {
        id: Uuid::new_v4(),
        backup_id: backup.id,
        operation: SyncOperation::Upload,
        status: SyncStatus::Pending,
        attempts: 0,
        created_at: Utc::now(),
    };
    queue_repo.enqueue(&queue_item).unwrap();

    let mock_cloud = MockCloudProvider::new();
    let engine = SyncEngine::with_config(
        mock_cloud.clone(),
        backup_root,
        SyncEngineConfig::fast_for_tests(),
    );

    let synced = engine.process_queue(&conn).await.unwrap();
    assert_eq!(synced, 1);

    // Fila limpa
    assert!(queue_repo.list_pending().unwrap().is_empty());

    // Metadados atualizados atomicamente
    let backups_repo = BackupsRepo::new(&conn);
    let updated = backups_repo.find_by_id(backup.id).unwrap().unwrap();
    assert_eq!(updated.sync_status, SyncStatus::Synced);
    assert!(updated.cloud_path.is_some());
    assert!(updated.drive_file_id.is_some());
}
