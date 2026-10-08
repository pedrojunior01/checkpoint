//! Teste de Integração End-to-End: Cloud Disaster Recovery (AUD-02).
//!
//! Simula o ciclo completo de Disaster Recovery entre duas máquinas independentes:
//!
//! MÁQUINA A:
//! 1. Criação do ambiente local (banco SQLite, save location, backup root).
//! 2. Criação do jogo e gravação do save original no disco.
//! 3. Criação de snapshot local versionado.
//! 4. Enfileiramento na sync_queue e upload para o Cloud Provider via SyncEngine.
//! 5. Confirmação da persistência remota do snapshot e do manifesto central (`savevault_manifest.json`).
//!
//! DESTRUIÇÃO TOTAL DA MÁQUINA A:
//! 6. Destruição do banco SQLite e exclusão de todos os diretórios locais de backup e save da Máquina A.
//!
//! MÁQUINA B (Novo Computador / Instalação Limpa):
//! 7. Inicialização de ambiente novo e isolado (banco SQLite completamente vazio, sem catálogo prévio).
//! 8. Execução do Cloud Disaster Recovery (`reconstruct_catalog_from_cloud`).
//! 9. Validação da reconstrução estrutural completa (jogos, save_locations, files e backups).
//! 10. Download do snapshot físico a partir do Cloud Provider para a Máquina B (`pull_snapshots_with_provider`).
//! 11. Restauração atômica do save para o destino (`restore_snapshot`).
//! 12. Validação criptográfica de integridade: hash BLAKE3 do save restaurado na Máquina B
//!     DEVE ser estritamente idêntico ao original gravado na Máquina A.
//!
//! CENÁRIOS NEGATIVOS:
//! 13. Manifest inexistente -> erro explícito ManifestNotFound (sem mascaramento para 0 saves).
//! 14. Provider offline/network error -> erro explícito Network (sem mascaramento).
//! 15. Token expirado / não autenticado -> erro explícito Authentication.
//! 16. Manifest corrompido -> erro explícito InvalidManifest.
//! 17. Manifest válido com 0 saves -> retorna Ok(0) distinguido de ManifestNotFound.

use chrono::Utc;
use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;
use uuid::Uuid;

use savevault_app::cloud_recovery::{
    list_cloud_snapshots, pull_snapshots_with_provider, reconstruct_catalog_from_cloud,
    RecoveryError,
};
use savevault_app::restore::{restore_snapshot, RestoreRequest};
use savevault_backup::snapshot::SnapshotManager;
use savevault_cloud::{
    CloudCatalogManifest, CloudError, CloudProvider, CloudResult, LocalFolderProvider,
};
use savevault_core::models::{
    Confidence, FileEntry, Game, Platform, SaveLocation, SaveLocationType, SyncOperation,
    SyncQueueItem, SyncStatus,
};
use savevault_db::repositories::{
    BackupsRepo, FilesRepo, GamesRepo, SaveLocationsRepo, SyncQueueRepo,
};
use savevault_db::Database;
use savevault_sync::{SyncEngine, SyncEngineConfig};

#[tokio::test]
async fn test_full_cloud_disaster_recovery_e2e() {
    // -------------------------------------------------------------------------
    // NUVEM: Diretório central compartilhado gerenciado pelo CloudProvider
    // -------------------------------------------------------------------------
    let cloud_remote_dir = tempdir().unwrap();
    let cloud_provider = LocalFolderProvider::new(cloud_remote_dir.path().to_path_buf());

    // -------------------------------------------------------------------------
    // MÁQUINA A: Setup inicial
    // -------------------------------------------------------------------------
    let machine_a_saves = tempdir().unwrap();
    let machine_a_backups = tempdir().unwrap();
    let db_a = Database::open_in_memory().unwrap();

    let games_repo_a = GamesRepo::new(&db_a.conn);
    let loc_repo_a = SaveLocationsRepo::new(&db_a.conn);
    let files_repo_a = FilesRepo::new(&db_a.conn);
    let backups_repo_a = BackupsRepo::new(&db_a.conn);
    let queue_repo_a = SyncQueueRepo::new(&db_a.conn);

    // 1. Cadastra jogo e local de save na Máquina A
    let game = Game::new(
        "Dark Souls III",
        Platform::Steam,
        Some("374320".into()),
        Confidence::High,
    );
    games_repo_a.insert(&game).unwrap();

    let save_loc = SaveLocation::new(
        game.id,
        machine_a_saves.path().to_path_buf(),
        SaveLocationType::Directory,
    );
    loc_repo_a.insert(&save_loc).unwrap();

    // 2. Cria arquivo de save no disco da Máquina A
    let save_filename = "DS30000.sl2";
    let save_file_path = machine_a_saves.path().join(save_filename);
    let original_save_bytes = b"DARKSOUdevice_ORIGINAL_SAVE_BYTES_LEVEL_120_SOULS_999999";
    tokio::fs::write(&save_file_path, original_save_bytes)
        .await
        .unwrap();

    let original_hash = savevault_backup::hasher::hash_file(&save_file_path)
        .await
        .unwrap();

    let file_entry = FileEntry {
        id: Uuid::new_v4(),
        save_location_id: save_loc.id,
        path: save_file_path.clone(),
        hash: original_hash.clone(),
        size_bytes: original_save_bytes.len() as u64,
        modified_at: Utc::now(),
    };
    files_repo_a.upsert(&file_entry).unwrap();

    // 3. Gera snapshot local na Máquina A
    let snapshot_manager = SnapshotManager::new(machine_a_backups.path().to_path_buf(), 5);
    let backup = snapshot_manager
        .create_snapshot_with_relpath(
            file_entry.id,
            &game.name,
            &save_file_path,
            Some(Path::new(save_filename)),
            1,
        )
        .await
        .expect("Snapshot deve ser criado com sucesso na Máquina A");

    backups_repo_a.insert(&backup).unwrap();

    // 4. Enfileira na sync_queue da Máquina A
    let queue_item = SyncQueueItem {
        id: Uuid::new_v4(),
        backup_id: backup.id,
        operation: SyncOperation::Upload,
        status: SyncStatus::Pending,
        attempts: 0,
        created_at: Utc::now(),
    };
    queue_repo_a.enqueue(&queue_item).unwrap();

    // 5. Executa sincronização com o Cloud Provider via SyncEngine
    let sync_engine_a = SyncEngine::with_config(
        cloud_provider,
        machine_a_backups.path().to_path_buf(),
        SyncEngineConfig::fast_for_tests(),
    );

    let synced = sync_engine_a
        .process_queue(&db_a.conn)
        .await
        .expect("Sincronização com a nuvem deve ser concluída");
    assert_eq!(synced, 1, "Snapshot deve ser sincronizado com a nuvem");

    // 6. Confirma que o manifesto foi persistido e contém as informações do snapshot
    let cloud_verifier = LocalFolderProvider::new(cloud_remote_dir.path().to_path_buf());
    let manifest_in_cloud = cloud_verifier
        .get_manifest()
        .await
        .unwrap()
        .expect("Manifesto central deve existir na nuvem");

    assert_eq!(manifest_in_cloud.schema_version, 1);
    assert_eq!(manifest_in_cloud.entries.len(), 1);
    let entry = &manifest_in_cloud.entries[0];
    assert_eq!(entry.game_name, "Dark Souls III");
    assert_eq!(entry.file_hash, original_hash);
    assert_eq!(entry.version, 1);
    assert_eq!(entry.relative_file_path, save_filename);

    // -------------------------------------------------------------------------
    // SIMULAÇÃO: Destruição total da Máquina A
    // -------------------------------------------------------------------------
    drop(db_a);
    drop(machine_a_saves);
    drop(machine_a_backups);

    // -------------------------------------------------------------------------
    // MÁQUINA B: Disaster Recovery em novo ambiente limpo
    // -------------------------------------------------------------------------
    let machine_b_saves = tempdir().unwrap();
    let machine_b_backups = tempdir().unwrap();
    let db_b = Database::open_in_memory().unwrap();

    // Valida que o banco da Máquina B está 100% vazio
    let count_games: usize = db_b
        .conn
        .query_row("SELECT count(*) FROM games", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count_games, 0, "Máquina B deve iniciar com catálogo vazio");

    let cloud_b = LocalFolderProvider::new(cloud_remote_dir.path().to_path_buf());

    // 7. Executa recuperação de desastre do catálogo a partir da nuvem
    let recovered_catalog_count = reconstruct_catalog_from_cloud(&db_b.conn, &cloud_b)
        .await
        .expect("Reconstrução do catálogo na Máquina B deve ser bem-sucedida");
    assert_eq!(
        recovered_catalog_count, 1,
        "Exatamente 1 snapshot deve ser importado para o catálogo da Máquina B"
    );

    // 8. Confirma que os registros foram criados coerentemente no SQLite da Máquina B
    let games_repo_b = GamesRepo::new(&db_b.conn);
    let recovered_games = games_repo_b.list_all().unwrap();
    assert_eq!(recovered_games.len(), 1);
    assert_eq!(recovered_games[0].name, "Dark Souls III");

    let snapshots_b = list_cloud_snapshots(&db_b.conn, machine_b_backups.path()).unwrap();
    assert_eq!(snapshots_b.len(), 1);
    assert_eq!(snapshots_b[0].game_name, "Dark Souls III");
    assert_eq!(snapshots_b[0].version, 1);
    assert!(
        !snapshots_b[0].local_exists,
        "Snapshot ainda não foi baixado fisicamente para a Máquina B"
    );

    // Atualiza o caminho de save_locations na Máquina B para o diretório de destino local da Máquina B
    db_b.conn
        .execute(
            "UPDATE save_locations SET path = ?1, mapping_status = 'user_mapped'",
            [machine_b_saves.path().to_string_lossy().to_string()],
        )
        .unwrap();
    let new_dest_file_path = machine_b_saves.path().join(save_filename);
    db_b.conn
        .execute(
            "UPDATE files SET path = ?1",
            [new_dest_file_path.to_string_lossy().to_string()],
        )
        .unwrap();

    // 9. Baixa snapshots ausentes da nuvem para o backup root da Máquina B
    let pull_report = pull_snapshots_with_provider(
        &db_b.conn,
        machine_b_backups.path(),
        &cloud_b,
        Some("Dark Souls III"),
    )
    .await
    .expect("Download dos snapshots ausentes deve ter sucesso");

    assert_eq!(pull_report.downloaded, 1);
    assert_eq!(pull_report.failed, 0);

    // Valida que o snapshot físico agora existe no disco da Máquina B
    let snapshots_after_pull = list_cloud_snapshots(&db_b.conn, machine_b_backups.path()).unwrap();
    assert_eq!(snapshots_after_pull.len(), 1);
    assert!(
        snapshots_after_pull[0].local_exists,
        "Snapshot deve existir fisicamente no disco da Máquina B após o pull"
    );

    // 10. Restaura o save usando o mecanismo de restauração atômico e seguro
    let restore_request =
        RestoreRequest::with_expected_hash(snapshots_after_pull[0].snapshot_id, &original_hash);

    let report = restore_snapshot(&db_b.conn, machine_b_backups.path(), &restore_request)
        .await
        .expect("Restauração do save na Máquina B deve ser bem-sucedida");

    assert_eq!(report.restored_hash, original_hash);

    // 11. Validação estrita de integridade: o arquivo no destino da Máquina B DEVE coincidir bit a bit
    assert!(new_dest_file_path.exists());
    let restored_bytes = tokio::fs::read(&new_dest_file_path).await.unwrap();
    assert_eq!(
        restored_bytes, original_save_bytes,
        "O arquivo restaurado na Máquina B deve ser idêntico ao save original da Máquina A"
    );
    let restored_hash = savevault_backup::hasher::hash_file(&new_dest_file_path)
        .await
        .unwrap();
    assert_eq!(
        restored_hash, original_hash,
        "Hash BLAKE3 do arquivo restaurado na Máquina B deve corresponder exatamente ao original"
    );
}

// ---------------------------------------------------------------------------
// TESTES NEGATIVOS E ADVERSARIAIS
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct AdversarialCloudMock {
    manifest_response: Arc<std::sync::Mutex<Option<CloudResult<Option<CloudCatalogManifest>>>>>,
    reject_filename_download: bool,
}

#[async_trait::async_trait]
impl CloudProvider for AdversarialCloudMock {
    async fn upload(&self, _: &Path, _: &str) -> CloudResult<String> {
        Ok("mock_remote_id_123".into())
    }

    async fn download(&self, remote_file_id: &str, _: &Path) -> CloudResult<()> {
        if self.reject_filename_download && remote_file_id.contains(".json") {
            return Err(CloudError::InvalidRemoteId(format!(
                "Nome de arquivo '{}' não é um ID remoto válido",
                remote_file_id
            )));
        }
        Ok(())
    }

    async fn delete(&self, _: &str) -> CloudResult<()> {
        Ok(())
    }

    async fn get_manifest(&self) -> CloudResult<Option<CloudCatalogManifest>> {
        let lock = self.manifest_response.lock().unwrap();
        match &*lock {
            Some(Ok(opt)) => Ok(opt.clone()),
            Some(Err(CloudError::Unauthenticated)) => Err(CloudError::Unauthenticated),
            Some(Err(CloudError::Authentication(s))) => Err(CloudError::Authentication(s.clone())),
            Some(Err(CloudError::ManifestNotFound)) => Err(CloudError::ManifestNotFound),
            Some(Err(CloudError::InvalidManifest(s))) => {
                Err(CloudError::InvalidManifest(s.clone()))
            }
            Some(Err(CloudError::Network(s))) => Err(CloudError::Network(s.clone())),
            Some(Err(CloudError::Api { status, message })) => Err(CloudError::Api {
                status: *status,
                message: message.clone(),
            }),
            Some(Err(CloudError::InvalidRemoteId(s))) => {
                Err(CloudError::InvalidRemoteId(s.clone()))
            }
            Some(Err(e)) => Err(CloudError::Other(anyhow::anyhow!("{}", e))),
            None => Ok(None),
        }
    }

    async fn save_manifest(&self, _: &CloudCatalogManifest) -> CloudResult<()> {
        Ok(())
    }
}

#[tokio::test]
async fn test_negative_disaster_recovery_manifest_not_found() {
    let db = Database::open_in_memory().unwrap();
    let mock = AdversarialCloudMock {
        manifest_response: Arc::new(std::sync::Mutex::new(Some(Ok(None)))),
        reject_filename_download: true,
    };

    let result = reconstruct_catalog_from_cloud(&db.conn, &mock).await;
    assert!(
        matches!(result, Err(RecoveryError::ManifestNotFound)),
        "Manifesto ausente na nuvem DEVE retornar RecoveryError::ManifestNotFound e NUNCA Ok(0)"
    );
}

#[tokio::test]
async fn test_negative_disaster_recovery_cloud_offline_network_error() {
    let db = Database::open_in_memory().unwrap();
    let mock = AdversarialCloudMock {
        manifest_response: Arc::new(std::sync::Mutex::new(Some(Err(CloudError::Network(
            "Connection reset by peer (offline)".into(),
        ))))),
        reject_filename_download: true,
    };

    let result = reconstruct_catalog_from_cloud(&db.conn, &mock).await;
    assert!(
        matches!(result, Err(RecoveryError::Network(_))),
        "Erro de rede DEVE propagar RecoveryError::Network e NUNCA Ok(0)"
    );
}

#[tokio::test]
async fn test_negative_disaster_recovery_authentication_failed_401() {
    let db = Database::open_in_memory().unwrap();
    let mock = AdversarialCloudMock {
        manifest_response: Arc::new(std::sync::Mutex::new(Some(Err(
            CloudError::Authentication("Token expirado (HTTP 401)".into()),
        )))),
        reject_filename_download: true,
    };

    let result = reconstruct_catalog_from_cloud(&db.conn, &mock).await;
    assert!(
        matches!(result, Err(RecoveryError::Authentication(_))),
        "Erro de autenticação DEVE propagar RecoveryError::Authentication e NUNCA Ok(0)"
    );
}

#[tokio::test]
async fn test_negative_disaster_recovery_corrupted_manifest_json() {
    let db = Database::open_in_memory().unwrap();
    let mock = AdversarialCloudMock {
        manifest_response: Arc::new(std::sync::Mutex::new(Some(Err(
            CloudError::InvalidManifest("Caractere inválido na linha 1 coluna 1".into()),
        )))),
        reject_filename_download: true,
    };

    let result = reconstruct_catalog_from_cloud(&db.conn, &mock).await;
    assert!(
        matches!(result, Err(RecoveryError::InvalidManifest(_))),
        "Manifesto corrompido DEVE propagar RecoveryError::InvalidManifest e NUNCA Ok(0)"
    );
}

#[tokio::test]
async fn test_negative_disaster_recovery_valid_manifest_with_zero_saves() {
    let db = Database::open_in_memory().unwrap();
    let mock = AdversarialCloudMock {
        manifest_response: Arc::new(std::sync::Mutex::new(Some(Ok(Some(
            CloudCatalogManifest {
                schema_version: 1,
                updated_at: Utc::now().to_rfc3339(),
                entries: vec![],
            },
        ))))),
        reject_filename_download: true,
    };

    let result = reconstruct_catalog_from_cloud(&db.conn, &mock).await;
    assert_eq!(
        result.unwrap(),
        0,
        "Manifesto válido com 0 saves DEVE retornar Ok(0), diferentemente de ManifestNotFound"
    );
}

#[tokio::test]
async fn test_negative_contract_download_rejects_manifest_filename_as_file_id() {
    let mock = AdversarialCloudMock {
        manifest_response: Arc::new(std::sync::Mutex::new(None)),
        reject_filename_download: true,
    };

    let tmp = tempfile::NamedTempFile::new().unwrap();
    let result = mock.download("savevault_manifest.json", tmp.path()).await;

    assert!(
        matches!(result, Err(CloudError::InvalidRemoteId(_))),
        "Mock estrito deve rejeitar 'savevault_manifest.json' como file_id"
    );
}
