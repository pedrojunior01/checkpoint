//! Testes de Integração e Sistema: Remapeamento de Saves Multiplataforma (AUD-03).
//!
//! Cobre exaustivamente:
//! - Etapa 13 (20 cenários de resolução, segurança de path, conflito, persistência e discovery)
//! - Etapa 14 (Fluxo E2E completo: Máquina A -> Cloud -> Máquina B -> Detecção -> Remapeamento -> Restore -> Hash BLAKE3)

use chrono::Utc;
use std::path::{Path, PathBuf};
use tempfile::tempdir;
use uuid::Uuid;

use savevault_app::cloud_recovery::{
    list_cloud_snapshots, pull_snapshots_with_provider, reconstruct_catalog_from_cloud,
};
use savevault_app::remapping::{
    apply_remapping, list_pending_remappings, reset_remapping, resolve_path_for_game,
    validate_candidate_path,
};
use savevault_app::restore::{restore_snapshot, RestoreError, RestoreRequest};
use savevault_backup::snapshot::SnapshotManager;
use savevault_cloud::LocalFolderProvider;
use savevault_core::models::{
    Confidence, FileEntry, Game, MappingSource, MappingStatus, Platform, SaveLocation,
    SaveLocationType, SyncOperation, SyncQueueItem, SyncStatus,
};
use savevault_core::path::{
    extract_username_from_path, get_current_system_username, replace_username_in_path,
    validate_remapping_path_security,
};
use savevault_db::repositories::{
    BackupsRepo, FilesRepo, GamesRepo, SaveLocationsRepo, SyncQueueRepo,
};
use savevault_db::Database;
use savevault_sync::{SyncEngine, SyncEngineConfig};

// =============================================================================
// ETAPA 13 — CENÁRIOS 1 A 20
// =============================================================================

#[test]
fn test_01_caminho_original_existente() {
    let tmp = tempdir().unwrap();
    let existing_path = tmp.path().join("ExistingSaveDir");
    std::fs::create_dir_all(&existing_path).unwrap();

    let (res_path, status, source, _reason, _candidate) =
        resolve_path_for_game("The Witcher 3", &existing_path, None);

    assert_eq!(status, MappingStatus::Resolved);
    assert_eq!(source, MappingSource::Original);
    assert_eq!(res_path, existing_path);
}

#[test]
fn test_02_caminho_original_inexistente() {
    let non_existent = PathBuf::from("C:\\FakeNonExistentDrive_XYZ\\NonExistentFolder_123");
    let (res_path, status, _source, reason, _candidate) =
        resolve_path_for_game("Cyberpunk 2077", &non_existent, None);

    assert_eq!(status, MappingStatus::NeedsUserMapping);
    assert_eq!(res_path, non_existent);
    assert!(reason.unwrap().contains("não encontrado"));
}

#[test]
fn test_03_resolucao_automatica_valida() {
    let current_user = get_current_system_username().unwrap_or_else(|| "TestUser".to_string());

    #[cfg(target_os = "windows")]
    let old_path = PathBuf::from(r"C:\Users\Alice\Saved Games\GameA");
    #[cfg(not(target_os = "windows"))]
    let old_path = PathBuf::from("/home/alice/Saved Games/GameA");

    let replaced = replace_username_in_path(&old_path, &current_user);
    assert!(replaced.is_some());
    let replaced_path = replaced.unwrap();
    assert!(replaced_path.to_string_lossy().contains(&current_user));
    assert!(!replaced_path.to_string_lossy().contains("Alice"));
}

#[test]
fn test_04_resolucao_automatica_rejeitada() {
    let obscure_path = PathBuf::from("Z:\\Deep\\Custom\\Dir\\Without\\User\\Saves");
    let (res_path, status, _source, _reason, _candidate) =
        resolve_path_for_game("ObscureGame", &obscure_path, None);

    assert_eq!(status, MappingStatus::NeedsUserMapping);
    assert_eq!(res_path, obscure_path);
}

#[test]
fn test_05_username_diferente() {
    #[cfg(target_os = "windows")]
    let p = Path::new(r"C:\Users\Alice\AppData\Local\MyGame\Saves");
    #[cfg(not(target_os = "windows"))]
    let p = Path::new("/home/alice/.config/MyGame/Saves");

    assert_eq!(extract_username_from_path(p), Some("Alice".to_string()));

    let switched = replace_username_in_path(p, "Bob").unwrap();
    assert_eq!(
        extract_username_from_path(&switched),
        Some("Bob".to_string())
    );
}

#[test]
fn test_06_windows_path() {
    let win_path = Path::new(r"C:\Users\Gamer\Saved Games\EldenRing");
    assert_eq!(
        extract_username_from_path(win_path),
        Some("Gamer".to_string())
    );
    let replaced = replace_username_in_path(win_path, "NewUser").unwrap();
    let norm = replaced.to_string_lossy();
    assert!(norm.contains("NewUser"));
    assert!(!norm.contains("Gamer"));
}

#[test]
fn test_07_linux_path() {
    let linux_home = Path::new("/home/user1/.local/share/Steam/saves");
    assert_eq!(
        extract_username_from_path(linux_home),
        Some("user1".to_string())
    );
    let replaced = replace_username_in_path(linux_home, "user2").unwrap();
    assert_eq!(
        replaced,
        PathBuf::from("/home/user2/.local/share/Steam/saves")
    );
}

#[test]
fn test_08_macos_path() {
    let mac_path = Path::new("/Users/developer/Library/Application Support/GameX");
    assert_eq!(
        extract_username_from_path(mac_path),
        Some("developer".to_string())
    );
    let replaced = replace_username_in_path(mac_path, "player").unwrap();
    assert_eq!(
        replaced,
        PathBuf::from("/Users/player/Library/Application Support/GameX")
    );
}

#[test]
fn test_09_path_traversal() {
    let traversal_paths = [
        "C:\\Users\\Bob\\..\\..\\Windows\\System32",
        "/home/bob/../../etc/passwd",
        "relative/../traversal/path",
        "..\\..\\AppData\\Roaming",
    ];

    for bad in traversal_paths {
        let res = validate_remapping_path_security(Path::new(bad));
        assert!(
            res.is_err(),
            "Deveria ter rejeitado path traversal: {}",
            bad
        );
    }
}

#[test]
fn test_10_absolute_path_malicioso() {
    // Raiz de disco pura (C:\ ou /)
    assert!(validate_remapping_path_security(Path::new("C:\\")).is_err());
    assert!(validate_remapping_path_security(Path::new("/")).is_err());

    // Diretórios de sistema protegidos
    #[cfg(target_os = "windows")]
    {
        assert!(validate_remapping_path_security(Path::new(r"C:\Windows\System32")).is_err());
        assert!(validate_remapping_path_security(Path::new(r"C:\Program Files\Game")).is_err());
    }
    #[cfg(not(target_os = "windows"))]
    {
        assert!(validate_remapping_path_security(Path::new("/etc")).is_err());
        assert!(validate_remapping_path_security(Path::new("/usr/bin")).is_err());
    }
}

#[test]
fn test_11_destino_inexistente() {
    let tmp = tempdir().unwrap();
    let future_path = tmp.path().join("FutureDir").join("GameSaves");
    let val = validate_candidate_path(&future_path);
    assert!(val.valid);
    assert!(!val.has_conflicts);
    assert_eq!(val.existing_files_count, 0);
}

#[test]
fn test_12_destino_valido() {
    let tmp = tempdir().unwrap();
    let valid_dir = tmp.path().join("MyValidSaves");
    std::fs::create_dir_all(&valid_dir).unwrap();

    let val = validate_candidate_path(&valid_dir);
    assert!(val.valid);
    assert!(!val.has_conflicts);
    assert_eq!(val.existing_files_count, 0);
}

#[test]
fn test_13_destino_com_conflito() {
    let tmp = tempdir().unwrap();
    let conflict_dir = tmp.path().join("ConflictDir");
    std::fs::create_dir_all(&conflict_dir).unwrap();

    std::fs::write(conflict_dir.join("save1.dat"), b"existing data 1").unwrap();
    std::fs::write(conflict_dir.join("save2.dat"), b"existing data 2").unwrap();

    let val = validate_candidate_path(&conflict_dir);
    assert!(val.valid);
    assert!(val.has_conflicts);
    assert_eq!(val.existing_files_count, 2);
}

#[test]
fn test_14_mapping_persistido() {
    let db = Database::open_in_memory().unwrap();
    let games_repo = GamesRepo::new(&db.conn);
    let loc_repo = SaveLocationsRepo::new(&db.conn);

    let game = Game::new("Hollow Knight", Platform::Steam, None, Confidence::High);
    games_repo.insert(&game).unwrap();

    let old_path = PathBuf::from("C:\\OldMachine\\Alice\\HollowKnight");
    let loc = SaveLocation::new(game.id, old_path.clone(), SaveLocationType::Directory)
        .with_mapping(
            old_path.clone(),
            MappingStatus::NeedsUserMapping,
            MappingSource::Original,
            Some("Caminho não encontrado".into()),
        );
    loc_repo.insert(&loc).unwrap();

    let tmp = tempdir().unwrap();
    let new_path = tmp.path().join("Bob").join("HollowKnight");
    std::fs::create_dir_all(&new_path).unwrap();

    let updated = apply_remapping(&db.conn, loc.id, &new_path).unwrap();
    assert_eq!(updated.mapping_status, MappingStatus::UserMapped);
    assert_eq!(updated.mapping_source, MappingSource::User);
    assert_eq!(updated.original_path, Some(old_path));
    assert_eq!(updated.path, new_path);

    let persisted = loc_repo.find_by_id(loc.id).unwrap().unwrap();
    assert_eq!(persisted.path, new_path);
    assert_eq!(persisted.mapping_status, MappingStatus::UserMapped);
}

#[test]
fn test_15_mapping_reaplicado() {
    let db = Database::open_in_memory().unwrap();
    let games_repo = GamesRepo::new(&db.conn);
    let loc_repo = SaveLocationsRepo::new(&db.conn);

    let game = Game::new("Celeste", Platform::Steam, None, Confidence::High);
    games_repo.insert(&game).unwrap();

    let loc = SaveLocation::new(
        game.id,
        PathBuf::from("C:\\Original\\Celeste"),
        SaveLocationType::Directory,
    )
    .with_mapping(
        PathBuf::from("C:\\Original\\Celeste"),
        MappingStatus::NeedsUserMapping,
        MappingSource::Original,
        None,
    );
    loc_repo.insert(&loc).unwrap();

    let tmp = tempdir().unwrap();
    let dest_1 = tmp.path().join("Dest1");
    std::fs::create_dir_all(&dest_1).unwrap();
    apply_remapping(&db.conn, loc.id, &dest_1).unwrap();

    let dest_2 = tmp.path().join("Dest2");
    std::fs::create_dir_all(&dest_2).unwrap();
    let re_applied = apply_remapping(&db.conn, loc.id, &dest_2).unwrap();

    assert_eq!(re_applied.path, dest_2);
    assert_eq!(
        re_applied.original_path,
        Some(PathBuf::from("C:\\Original\\Celeste"))
    );
}

#[test]
fn test_16_mapping_removido() {
    let db = Database::open_in_memory().unwrap();
    let games_repo = GamesRepo::new(&db.conn);
    let loc_repo = SaveLocationsRepo::new(&db.conn);

    let game = Game::new("Portal 2", Platform::Steam, None, Confidence::High);
    games_repo.insert(&game).unwrap();

    let orig = PathBuf::from("C:\\Original\\Portal2");
    let loc = SaveLocation::new(game.id, orig.clone(), SaveLocationType::Directory).with_mapping(
        orig.clone(),
        MappingStatus::NeedsUserMapping,
        MappingSource::Original,
        None,
    );
    loc_repo.insert(&loc).unwrap();

    let tmp = tempdir().unwrap();
    let mapped = tmp.path().join("Mapped");
    std::fs::create_dir_all(&mapped).unwrap();
    apply_remapping(&db.conn, loc.id, &mapped).unwrap();

    let reset = reset_remapping(&db.conn, loc.id).unwrap();
    assert_eq!(reset.path, orig);
    assert_eq!(reset.mapping_status, MappingStatus::NeedsUserMapping);
}

#[test]
fn test_17_mapping_idempotente() {
    let db = Database::open_in_memory().unwrap();
    let games_repo = GamesRepo::new(&db.conn);
    let loc_repo = SaveLocationsRepo::new(&db.conn);

    let game = Game::new("Sekiro", Platform::Steam, None, Confidence::High);
    games_repo.insert(&game).unwrap();

    let orig = PathBuf::from("C:\\MachineA\\Sekiro");
    let loc = SaveLocation::new(game.id, orig.clone(), SaveLocationType::Directory).with_mapping(
        orig.clone(),
        MappingStatus::NeedsUserMapping,
        MappingSource::Original,
        None,
    );
    loc_repo.insert(&loc).unwrap();

    let tmp = tempdir().unwrap();
    let user_dest = tmp.path().join("UserSekiro");
    std::fs::create_dir_all(&user_dest).unwrap();
    apply_remapping(&db.conn, loc.id, &user_dest).unwrap();

    // Simula recuperação repetida da nuvem: upsert no banco
    let cloud_loc = SaveLocation::new(game.id, orig.clone(), SaveLocationType::Directory)
        .with_mapping(
            orig.clone(),
            MappingStatus::NeedsUserMapping,
            MappingSource::Original,
            Some("Nuvem".into()),
        );
    loc_repo.upsert(&cloud_loc).unwrap();

    let re_fetched = loc_repo.find_by_id(loc.id).unwrap().unwrap();
    assert_eq!(re_fetched.mapping_status, MappingStatus::UserMapped);
    assert_eq!(re_fetched.path, user_dest);
}

#[test]
fn test_18_mapping_invalido() {
    let db = Database::open_in_memory().unwrap();
    let games_repo = GamesRepo::new(&db.conn);
    let loc_repo = SaveLocationsRepo::new(&db.conn);

    let game = Game::new("Fallout 4", Platform::Steam, None, Confidence::High);
    games_repo.insert(&game).unwrap();

    let loc = SaveLocation::new(
        game.id,
        PathBuf::from("C:\\Fake\\Fallout4"),
        SaveLocationType::Directory,
    );
    loc_repo.insert(&loc).unwrap();

    let res = apply_remapping(&db.conn, loc.id, Path::new("C:\\Windows\\System32"));
    assert!(res.is_err());
    let err_str = res.as_ref().unwrap_err();
    assert!(err_str.contains("sistema protegido") || err_str.contains("segurança"));
}

#[test]
fn test_19_discovery_plus_mapping() {
    let tmp = tempdir().unwrap();
    let discovered_dir = tmp.path().join("DiscoveredGames").join("Witcher3");
    std::fs::create_dir_all(&discovered_dir).unwrap();

    let db = Database::open_in_memory().unwrap();
    let games_repo = GamesRepo::new(&db.conn);
    let loc_repo = SaveLocationsRepo::new(&db.conn);

    let game = Game::new("The Witcher 3", Platform::Steam, None, Confidence::High);
    games_repo.insert(&game).unwrap();

    // Simula SaveLocation cadastrado pelo discovery local
    let disc_loc = SaveLocation::new(game.id, discovered_dir.clone(), SaveLocationType::Directory);
    loc_repo.insert(&disc_loc).unwrap();

    // Se o usuário aplicar um mapping explícito, ele tem precedência sobre o discovery
    let user_custom_dir = tmp.path().join("UserCustomWitcher");
    std::fs::create_dir_all(&user_custom_dir).unwrap();
    let mapped = apply_remapping(&db.conn, disc_loc.id, &user_custom_dir).unwrap();

    assert_eq!(mapped.mapping_status, MappingStatus::UserMapped);
    assert_eq!(mapped.mapping_source, MappingSource::User);
    assert_eq!(mapped.path, user_custom_dir);
}

#[test]
fn test_20_recovery_plus_mapping() {
    let db = Database::open_in_memory().unwrap();
    let games_repo = GamesRepo::new(&db.conn);
    let loc_repo = SaveLocationsRepo::new(&db.conn);

    let game = Game::new("Stardew Valley", Platform::Steam, None, Confidence::High);
    games_repo.insert(&game).unwrap();

    let old_path = PathBuf::from("C:\\NonExistent\\StardewValley");
    let loc = SaveLocation::new(game.id, old_path.clone(), SaveLocationType::Directory)
        .with_mapping(
            old_path.clone(),
            MappingStatus::NeedsUserMapping,
            MappingSource::Original,
            Some("Caminho não encontrado nesta máquina".into()),
        );
    loc_repo.insert(&loc).unwrap();

    let pending = list_pending_remappings(&db.conn).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].game_name, "Stardew Valley");
    assert_eq!(pending[0].mapping_status, MappingStatus::NeedsUserMapping);

    let tmp = tempdir().unwrap();
    let target = tmp.path().join("StardewValleySaves");
    std::fs::create_dir_all(&target).unwrap();

    apply_remapping(&db.conn, loc.id, &target).unwrap();

    let remaining_pending = list_pending_remappings(&db.conn).unwrap();
    assert_eq!(remaining_pending.len(), 0);
}

// =============================================================================
// ETAPA 14 — TESTE E2E COMPLETO CROSS-MACHINE
// =============================================================================

#[tokio::test]
async fn test_full_cross_machine_disaster_recovery_with_remapping_e2e() {
    // -------------------------------------------------------------------------
    // NUVEM: Storage compartilhado remoto
    // -------------------------------------------------------------------------
    let cloud_dir = tempdir().unwrap();
    let cloud_provider = LocalFolderProvider::new(cloud_dir.path().to_path_buf());

    // -------------------------------------------------------------------------
    // MÁQUINA A (Usuária "Alice"):
    // -------------------------------------------------------------------------
    let machine_a_saves = tempdir().unwrap();
    let machine_a_backups = tempdir().unwrap();
    let db_a = Database::open_in_memory().unwrap();

    let games_repo_a = GamesRepo::new(&db_a.conn);
    let loc_repo_a = SaveLocationsRepo::new(&db_a.conn);
    let files_repo_a = FilesRepo::new(&db_a.conn);
    let backups_repo_a = BackupsRepo::new(&db_a.conn);
    let queue_repo_a = SyncQueueRepo::new(&db_a.conn);

    let game = Game::new(
        "Baldurs Gate 3",
        Platform::Steam,
        Some("1086940".into()),
        Confidence::High,
    );
    games_repo_a.insert(&game).unwrap();

    let alice_save_dir = machine_a_saves.path().join("AliceSaves");
    std::fs::create_dir_all(&alice_save_dir).unwrap();

    let save_loc_a =
        SaveLocation::new(game.id, alice_save_dir.clone(), SaveLocationType::Directory);
    loc_repo_a.insert(&save_loc_a).unwrap();

    let save_filename = "savegame.lsv";
    let save_path_a = alice_save_dir.join(save_filename);
    let original_bytes = b"BALDURS_GATE_3_HONOUR_MODE_ACT_3_FINAL_BOSS_SAVE_DATA";
    tokio::fs::write(&save_path_a, original_bytes)
        .await
        .unwrap();

    let expected_blake3_hash = savevault_backup::hasher::hash_file(&save_path_a)
        .await
        .unwrap();

    let file_entry_a = FileEntry {
        id: Uuid::new_v4(),
        save_location_id: save_loc_a.id,
        path: save_path_a.clone(),
        hash: expected_blake3_hash.clone(),
        size_bytes: original_bytes.len() as u64,
        modified_at: Utc::now(),
    };
    files_repo_a.upsert(&file_entry_a).unwrap();

    let snap_mgr_a = SnapshotManager::new(machine_a_backups.path().to_path_buf(), 5);
    let backup_record_a = snap_mgr_a
        .create_snapshot_with_relpath(
            file_entry_a.id,
            &game.name,
            &save_path_a,
            Some(Path::new(save_filename)),
            1,
        )
        .await
        .expect("Snapshot deve ser criado com sucesso na Máquina A");

    backups_repo_a.insert(&backup_record_a).unwrap();

    let queue_item = SyncQueueItem {
        id: Uuid::new_v4(),
        backup_id: backup_record_a.id,
        operation: SyncOperation::Upload,
        status: SyncStatus::Pending,
        attempts: 0,
        created_at: Utc::now(),
    };
    queue_repo_a.enqueue(&queue_item).unwrap();

    let sync_engine_a = SyncEngine::with_config(
        cloud_provider,
        machine_a_backups.path().to_path_buf(),
        SyncEngineConfig::fast_for_tests(),
    );
    let uploaded = sync_engine_a.process_queue(&db_a.conn).await.unwrap();
    assert_eq!(uploaded, 1);

    // -------------------------------------------------------------------------
    // DESTRUIÇÃO TOTAL DA MÁQUINA A:
    // -------------------------------------------------------------------------
    drop(db_a);
    drop(machine_a_saves);
    drop(machine_a_backups);

    // -------------------------------------------------------------------------
    // MÁQUINA B (Novo Computador do usuário "Bob"):
    // -------------------------------------------------------------------------
    let machine_b_backups = tempdir().unwrap();
    let db_b = Database::open_in_memory().unwrap();
    let cloud_b = LocalFolderProvider::new(cloud_dir.path().to_path_buf());

    // 1. Executa recuperação do catálogo a partir da nuvem
    let rec_count = reconstruct_catalog_from_cloud(&db_b.conn, &cloud_b)
        .await
        .unwrap();
    assert_eq!(rec_count, 1);

    // 2. Verifica que a Máquina B detectou que o caminho original de Alice não existe
    let pending_before = list_pending_remappings(&db_b.conn).unwrap();
    assert_eq!(pending_before.len(), 1);
    assert_eq!(pending_before[0].game_name, "Baldurs Gate 3");
    assert_eq!(
        pending_before[0].mapping_status,
        MappingStatus::NeedsUserMapping
    );

    let loc_b_id = pending_before[0].location_id;

    // 3. Puxa snapshots da nuvem para o backup_root da Máquina B
    let pull_res = pull_snapshots_with_provider(
        &db_b.conn,
        machine_b_backups.path(),
        &cloud_b,
        Some("Baldurs Gate 3"),
    )
    .await
    .unwrap();
    assert_eq!(pull_res.downloaded, 1);

    let snapshots = list_cloud_snapshots(&db_b.conn, machine_b_backups.path()).unwrap();
    assert_eq!(snapshots.len(), 1);
    let snapshot_id = snapshots[0].snapshot_id;

    // 4. Tenta restaurar ANTES do remapeamento -> DEVE falhar com SaveLocationNeedsMapping
    let premature_restore = restore_snapshot(
        &db_b.conn,
        machine_b_backups.path(),
        &RestoreRequest::with_expected_hash(snapshot_id, &expected_blake3_hash),
    )
    .await;

    match premature_restore {
        Err(RestoreError::SaveLocationNeedsMapping(id, _path, reason)) => {
            assert_eq!(id, loc_b_id);
            assert!(
                reason.contains("não encontrado")
                    || reason.contains("remapeada")
                    || reason.contains("remapeamento")
            );
        }
        other => panic!(
            "Esperava RestoreError::SaveLocationNeedsMapping, obteve: {:?}",
            other
        ),
    }

    // 5. Usuário Bob escolhe o novo diretório de destino na Máquina B
    let machine_b_saves = tempdir().unwrap();
    let bob_dest_dir = machine_b_saves.path().join("Bob_BG3_Saves");
    std::fs::create_dir_all(&bob_dest_dir).unwrap();

    let val = validate_candidate_path(&bob_dest_dir);
    assert!(val.valid);
    assert_eq!(val.existing_files_count, 0);

    // 6. Aplica o remapeamento no SQLite
    apply_remapping(&db_b.conn, loc_b_id, &bob_dest_dir).unwrap();

    // 7. Confirma que a pendência foi sanada
    let pending_after = list_pending_remappings(&db_b.conn).unwrap();
    assert_eq!(pending_after.len(), 0);

    // 8. Agora executa a restauração -> DEVE ter sucesso!
    let restore_res = restore_snapshot(
        &db_b.conn,
        machine_b_backups.path(),
        &RestoreRequest::with_expected_hash(snapshot_id, &expected_blake3_hash),
    )
    .await
    .unwrap();

    // 9. O arquivo restaurado deve estar EXATAMENTE no novo diretório de Bob
    let expected_restored_file = bob_dest_dir.join(save_filename);
    assert_eq!(restore_res.dest_path, expected_restored_file);
    assert!(expected_restored_file.exists());

    // 10. Validação estrita do conteúdo e hash BLAKE3 no destino remapeado
    let restored_bytes = tokio::fs::read(&expected_restored_file).await.unwrap();
    assert_eq!(restored_bytes, original_bytes);

    let actual_blake3_hash = savevault_backup::hasher::hash_file(&expected_restored_file)
        .await
        .unwrap();
    assert_eq!(actual_blake3_hash, expected_blake3_hash);
    assert_eq!(restore_res.restored_hash, expected_blake3_hash);

    // 11. Testa Idempotência em uma nova rodada de disaster recovery:
    // O remapeamento de Bob não pode ser perdido se o catálogo for reconstruído novamente
    let second_rec = reconstruct_catalog_from_cloud(&db_b.conn, &cloud_b)
        .await
        .unwrap();
    // Idempotência: nenhum registro duplicado é criado
    assert_eq!(second_rec, 0);

    let loc_after_second_rec = SaveLocationsRepo::new(&db_b.conn)
        .find_by_id(loc_b_id)
        .unwrap()
        .unwrap();

    assert_eq!(loc_after_second_rec.path, bob_dest_dir);
    assert_eq!(
        loc_after_second_rec.mapping_status,
        MappingStatus::UserMapped
    );
    assert_eq!(loc_after_second_rec.mapping_source, MappingSource::User);
    assert_eq!(loc_after_second_rec.original_path, Some(alice_save_dir));
}
