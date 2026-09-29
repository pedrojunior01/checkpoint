//! Testes de Integração de Ponta a Ponta para Restauração de Snapshots (RES-01).

use chrono::Utc;
use std::path::PathBuf;
use tempfile::tempdir;
use uuid::Uuid;

use savevault_app::pipeline::handle_stable_event;
use savevault_app::restore::{restore_by_id, restore_snapshot, RestoreError, RestoreRequest};
use savevault_backup::SnapshotManager;
use savevault_core::models::{
    Confidence, FileEntry, Game, Platform, SaveLocation, SaveLocationType,
};
use savevault_db::repositories::{BackupsRepo, FilesRepo, GamesRepo, SaveLocationsRepo};
use savevault_db::Database;
use savevault_monitor::events::{StableFileEvent, StableFileEventKind};

#[tokio::test]
async fn test_end_to_end_pipeline_snapshot_and_restore() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let db_path = temp_root.path().join("vault.db");
    let db = Database::open(&db_path).expect("Falha ao abrir banco");

    let backup_dir = temp_root.path().join("backups");
    let save_dir = temp_root.path().join("game_saves");
    std::fs::create_dir_all(&save_dir).expect("Falha ao criar save_dir");

    let save_file = save_dir.join("save_slot_1.sav");

    // 1. Configura jogo e local de save no SQLite
    let games_repo = GamesRepo::new(&db.conn);
    let loc_repo = SaveLocationsRepo::new(&db.conn);

    let game = Game::new(
        "Chronicles of Rust",
        Platform::Steam,
        Some("123456".into()),
        Confidence::High,
    );
    games_repo.insert(&game).unwrap();

    let save_loc = SaveLocation::new(game.id, save_dir.clone(), SaveLocationType::Directory);
    let saved_loc = loc_repo.upsert(&save_loc).unwrap();

    // 2. Cria arquivo no disco (v1) e processa no pipeline
    std::fs::write(&save_file, "progresso_capitulo_1").unwrap();

    let event_v1 = StableFileEvent {
        save_location_id: saved_loc.id,
        path: save_file.clone(),
        kind: StableFileEventKind::Created,
        size_bytes: "progresso_capitulo_1".len() as u64,
        observed_at: std::time::SystemTime::now(),
    };
    let res_v1 = handle_stable_event(&db.conn, &event_v1, &backup_dir, 10)
        .await
        .unwrap()
        .expect("Deveria gerar snapshot v1");
    let (backup_v1, _) = res_v1;
    assert_eq!(backup_v1.version, 1);

    // 3. Atualiza arquivo no disco (v2) e processa no pipeline
    std::fs::write(&save_file, "progresso_capitulo_2").unwrap();

    let event_v2 = StableFileEvent {
        save_location_id: saved_loc.id,
        path: save_file.clone(),
        kind: StableFileEventKind::Modified,
        size_bytes: "progresso_capitulo_2".len() as u64,
        observed_at: std::time::SystemTime::now(),
    };
    let res_v2 = handle_stable_event(&db.conn, &event_v2, &backup_dir, 10)
        .await
        .unwrap()
        .expect("Deveria gerar snapshot v2");
    let (backup_v2, _) = res_v2;
    assert_eq!(backup_v2.version, 2);

    // 4. Modifica arquivo para estado v3 (corrompido ou progresso não desejado)
    std::fs::write(&save_file, "progresso_capitulo_3_corrompido").unwrap();

    // 5. Restaura snapshot v1
    let report = restore_by_id(&db.conn, &backup_dir, backup_v1.id)
        .await
        .expect("Restauração do snapshot v1 deve ter sucesso");

    assert_eq!(report.snapshot_id, backup_v1.id);
    assert_eq!(report.version, 1);
    assert_eq!(report.dest_path, save_file);

    // 6. Confirma que o save ativo agora tem o conteúdo de v1
    let active_content = std::fs::read_to_string(&save_file).unwrap();
    assert_eq!(
        active_content, "progresso_capitulo_1",
        "O arquivo original deve ter sido restaurado para a versão 1"
    );

    // 7. Confirma que o safety backup contém o estado v3 prévio à restauração
    assert!(
        report.safety_backup_path.is_some(),
        "Safety backup deve ter sido gerado"
    );
    let safety_file = report.safety_backup_path.unwrap();
    assert!(safety_file.exists(), "Safety backup deve existir no disco");
    let safety_content = std::fs::read_to_string(&safety_file).unwrap();
    assert_eq!(
        safety_content, "progresso_capitulo_3_corrompido",
        "Safety backup deve preservar exatamente o estado antes da substituição"
    );

    // 8. Confirma que os snapshots físicos v1 e v2 continuam intactos no vault
    let v1_disk_path = backup_dir
        .join(backup_v1.file_id.to_string())
        .join("v1")
        .join("save_slot_1.sav");
    let v2_disk_path = backup_dir
        .join(backup_v2.file_id.to_string())
        .join("v2")
        .join("save_slot_1.sav");

    assert!(v1_disk_path.exists());
    assert_eq!(
        std::fs::read_to_string(&v1_disk_path).unwrap(),
        "progresso_capitulo_1"
    );

    assert!(v2_disk_path.exists());
    assert_eq!(
        std::fs::read_to_string(&v2_disk_path).unwrap(),
        "progresso_capitulo_2"
    );

    // 9. Confirma que os metadados em files refletem o hash restaurado
    let files_repo = FilesRepo::new(&db.conn);
    let file_in_db = files_repo.find_by_id(backup_v1.file_id).unwrap().unwrap();
    assert_eq!(file_in_db.hash, report.restored_hash);
}

#[tokio::test]
async fn test_end_to_end_consecutive_restores_and_safety_backups() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let db_path = temp_root.path().join("vault_consecutive.db");
    let db = Database::open(&db_path).expect("Falha ao abrir banco");

    let backup_dir = temp_root.path().join("backups");
    let save_dir = temp_root.path().join("saves");
    std::fs::create_dir_all(&save_dir).unwrap();

    let save_file = save_dir.join("game.sav");

    let games_repo = GamesRepo::new(&db.conn);
    let loc_repo = SaveLocationsRepo::new(&db.conn);
    let files_repo = FilesRepo::new(&db.conn);
    let backups_repo = BackupsRepo::new(&db.conn);

    let game = Game::new("Adventure", Platform::Generic, None, Confidence::High);
    games_repo.insert(&game).unwrap();

    let loc = SaveLocation::new(game.id, save_dir.clone(), SaveLocationType::Directory);
    let saved_loc = loc_repo.upsert(&loc).unwrap();

    let file_id = Uuid::new_v4();
    files_repo
        .upsert(&savevault_core::models::FileEntry {
            id: file_id,
            save_location_id: saved_loc.id,
            path: save_file.clone(),
            hash: "initial_h".into(),
            size_bytes: 8,
            modified_at: Utc::now(),
        })
        .unwrap();

    // Cria snapshots v1 e v2 fisicamente e no DB
    let v1_dir = backup_root_dir(&backup_dir, file_id, 1);
    std::fs::create_dir_all(&v1_dir).unwrap();
    std::fs::write(v1_dir.join("game.sav"), "v1_payload").unwrap();
    let b1 = savevault_core::models::Backup {
        id: Uuid::new_v4(),
        file_id,
        version: 1,
        cloud_path: None,
        drive_file_id: None,
        sync_status: savevault_core::models::SyncStatus::Pending,
        created_at: Utc::now(),
    };
    backups_repo.insert(&b1).unwrap();

    let v2_dir = backup_root_dir(&backup_dir, file_id, 2);
    std::fs::create_dir_all(&v2_dir).unwrap();
    std::fs::write(v2_dir.join("game.sav"), "v2_payload").unwrap();
    let b2 = savevault_core::models::Backup {
        id: Uuid::new_v4(),
        file_id,
        version: 2,
        cloud_path: None,
        drive_file_id: None,
        sync_status: savevault_core::models::SyncStatus::Pending,
        created_at: Utc::now(),
    };
    backups_repo.insert(&b2).unwrap();

    // Estado inicial no disco (v3)
    std::fs::write(&save_file, "v3_payload").unwrap();

    // 1. Restaura v1 (deve salvar v3 no safety backup)
    let rep1 = restore_by_id(&db.conn, &backup_dir, b1.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(&save_file).unwrap(), "v1_payload");
    let safety1 = rep1.safety_backup_path.unwrap();
    assert_eq!(std::fs::read_to_string(&safety1).unwrap(), "v3_payload");

    // 2. Restaura v2 (deve salvar v1 no safety backup)
    let rep2 = restore_by_id(&db.conn, &backup_dir, b2.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(&save_file).unwrap(), "v2_payload");
    let safety2 = rep2.safety_backup_path.unwrap();
    assert_eq!(std::fs::read_to_string(&safety2).unwrap(), "v1_payload");

    // Valida que ambos os safety backups coexistem e são diferentes
    assert_ne!(safety1, safety2);
    assert!(safety1.exists());
    assert!(safety2.exists());
}

#[tokio::test]
async fn test_restore_tampered_snapshot_fails_integrity_and_leaves_save_intact() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let db_path = temp_root.path().join("vault_tampered.db");
    let db = Database::open(&db_path).expect("Falha ao abrir banco");

    let backup_dir = temp_root.path().join("backups");
    let save_dir = temp_root.path().join("saves");
    std::fs::create_dir_all(&save_dir).unwrap();

    let save_file = save_dir.join("precious_save.dat");
    std::fs::write(&save_file, "save_intacto_original").unwrap();

    let games_repo = GamesRepo::new(&db.conn);
    let loc_repo = SaveLocationsRepo::new(&db.conn);
    let files_repo = FilesRepo::new(&db.conn);
    let backups_repo = BackupsRepo::new(&db.conn);

    let game = Game::new("RPG", Platform::Generic, None, Confidence::High);
    games_repo.insert(&game).unwrap();

    let loc = SaveLocation::new(game.id, save_dir.clone(), SaveLocationType::Directory);
    let saved_loc = loc_repo.upsert(&loc).unwrap();

    let file_id = Uuid::new_v4();
    files_repo
        .upsert(&savevault_core::models::FileEntry {
            id: file_id,
            save_location_id: saved_loc.id,
            path: save_file.clone(),
            hash: "orig_h".into(),
            size_bytes: 19,
            modified_at: Utc::now(),
        })
        .unwrap();

    let v1_dir = backup_root_dir(&backup_dir, file_id, 1);
    std::fs::create_dir_all(&v1_dir).unwrap();
    std::fs::write(v1_dir.join("precious_save.dat"), "conteudo_v1_snapshot").unwrap();

    let b1 = savevault_core::models::Backup {
        id: Uuid::new_v4(),
        file_id,
        version: 1,
        cloud_path: None,
        drive_file_id: None,
        sync_status: savevault_core::models::SyncStatus::Pending,
        created_at: Utc::now(),
    };
    backups_repo.insert(&b1).unwrap();

    // Solicita restauração com hash esperado falso
    let req = RestoreRequest::with_expected_hash(
        b1.id,
        "0000000000000000000000000000000000000000000000000000000000000000",
    );
    let err = restore_snapshot(&db.conn, &backup_dir, &req)
        .await
        .unwrap_err();

    match err {
        RestoreError::IntegrityCheckFailed(id, expected, _) => {
            assert_eq!(id, b1.id);
            assert_eq!(
                expected,
                "0000000000000000000000000000000000000000000000000000000000000000"
            );
        }
        other => panic!("Esperado IntegrityCheckFailed, obtido: {:?}", other),
    }

    // O save no disco NÃO pode ter sido alterado
    let current_content = std::fs::read_to_string(&save_file).unwrap();
    assert_eq!(current_content, "save_intacto_original");

    // Nenhum safety backup espúrio deve ter sido criado
    let entries: Vec<_> = std::fs::read_dir(&save_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .filter(|n| n.contains("savevault_safety"))
        .collect();
    assert!(
        entries.is_empty(),
        "Nenhum safety backup deve ser gerado se o snapshot falhar na validação"
    );
}

#[test]
fn test_cli_restore_binary_execution() {
    let bin_path = env!("CARGO_BIN_EXE_savevault");

    // 1. savevault restore --help
    let output_help = std::process::Command::new(bin_path)
        .arg("restore")
        .arg("--help")
        .output()
        .expect("Falha ao executar savevault restore --help");

    assert!(output_help.status.success());
    let stdout_help = String::from_utf8_lossy(&output_help.stdout);
    assert!(stdout_help.contains("Uso: savevault restore <snapshot-id>"));
    assert!(stdout_help.contains("savevault restore list"));

    // 2. savevault restore sem argumentos
    let output_no_args = std::process::Command::new(bin_path)
        .arg("restore")
        .output()
        .expect("Falha ao executar savevault restore");

    assert!(output_no_args.status.success());
    let stdout_no_args = String::from_utf8_lossy(&output_no_args.stdout);
    assert!(stdout_no_args.contains("Uso: savevault restore <snapshot-id>"));

    // 3. savevault restore not-a-uuid (deve falhar com código 1 e mensagem clara)
    let output_invalid_uuid = std::process::Command::new(bin_path)
        .arg("restore")
        .arg("not-a-uuid")
        .output()
        .expect("Falha ao executar savevault restore not-a-uuid");

    assert!(!output_invalid_uuid.status.success());
    let stderr_invalid = String::from_utf8_lossy(&output_invalid_uuid.stderr);
    assert!(stderr_invalid.contains("[FALHA] ID de snapshot inválido: 'not-a-uuid'"));
}

fn backup_root_dir(backup_root: &std::path::Path, file_id: Uuid, version: u32) -> PathBuf {
    backup_root
        .join(file_id.to_string())
        .join(format!("v{}", version))
}

#[test]
fn test_restore_rejects_ntfs_junction_escape() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let allowed_root = temp_root.path().join("allowed_saves");
    let sensitive_dir = temp_root.path().join("sensitive_outside");

    std::fs::create_dir_all(&allowed_root).unwrap();
    std::fs::create_dir_all(&sensitive_dir).unwrap();

    let junction_path = allowed_root.join("junction_link");

    // Cria a junção NTFS: allowed_saves\junction_link -> sensitive_outside
    let status = std::process::Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            junction_path.to_str().unwrap(),
            sensitive_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Falha ao invocar mklink /J");

    if !status.status.success() {
        eprintln!(
            "Aviso: ambiente não permitiu mklink /J: {}",
            String::from_utf8_lossy(&status.stderr)
        );
        return;
    }

    let save_loc = SaveLocation::new(
        Uuid::new_v4(),
        allowed_root.clone(),
        SaveLocationType::Directory,
    );
    let malicious_target = junction_path.join("sensitive_file.sav");

    let result =
        savevault_app::restore::validate_destination_security(&malicious_target, &save_loc);

    assert!(
        result.is_err(),
        "Restauração através de junction apontando para fora da raiz autorizada DEVE ser rejeitada!"
    );
    match result.unwrap_err() {
        RestoreError::PathSecurityViolation(msg) => {
            assert!(
                msg.contains("Reparse Point") || msg.contains("escapa"),
                "Mensagem de erro deve detalhar violação de reparse point: {}",
                msg
            );
        }
        other => panic!("Esperado PathSecurityViolation, obteve {:?}", other),
    }
}

#[tokio::test]
async fn test_restore_rollback_on_post_write_verification_failure() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let save_dir = temp_root.path().join("game_saves");
    std::fs::create_dir_all(&save_dir).unwrap();

    let save_file = save_dir.join("save_slot.sav");
    let original_content = "ORIGINAL_PLAYER_SAVE_DATA_VALID";
    std::fs::write(&save_file, original_content).unwrap();

    // Simula a criação de um safety backup antes da escrita
    let safety_backup = save_dir.join("save_slot.sav.savevault_safety_test");
    std::fs::write(&safety_backup, original_content).unwrap();

    // Simula corrupção durante a substituição pós-escrita
    let corrupted_content = "CORRUPTED_INCOMPLETE_WRITE_DATA";
    std::fs::write(&save_file, corrupted_content).unwrap();

    // Executa o rollback de segurança
    let res = savevault_app::restore::apply_safety_rollback(&save_file, Some(&safety_backup)).await;

    // 1. A falha de integridade deve ser reportada como erro
    assert!(
        res.is_err(),
        "Deve retornar erro indicando falha com rollback"
    );
    match res.unwrap_err() {
        RestoreError::PostWriteValidationFailedWithRollback {
            path,
            safety_backup: sb,
        } => {
            assert_eq!(path, save_file);
            assert_eq!(sb, safety_backup);
        }
        other => panic!(
            "Esperava PostWriteValidationFailedWithRollback, obteve {:?}",
            other
        ),
    }

    // 2. O arquivo final deve corresponder exatamente ao conteúdo original (rollback efetuado)
    let restored_content = std::fs::read_to_string(&save_file).unwrap();
    assert_eq!(
        restored_content, original_content,
        "O arquivo original deve ter sido restaurado a partir do safety backup!"
    );

    // 3. O safety backup deve permanecer preservado no disco
    assert!(
        safety_backup.exists(),
        "O safety backup deve permanecer no disco após o rollback para segurança adicional"
    );
}

#[tokio::test]
async fn test_restore_rollback_failure_reports_critical_error() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let save_dir = temp_root.path().join("game_saves");
    std::fs::create_dir_all(&save_dir).unwrap();

    let save_file = save_dir.join("save_slot.sav");
    std::fs::write(&save_file, "corrupted_content").unwrap();

    // Safety backup apontando para arquivo que não existe no disco
    let missing_safety_backup = save_dir.join("non_existent_safety_backup.sav");

    let res =
        savevault_app::restore::apply_safety_rollback(&save_file, Some(&missing_safety_backup))
            .await;

    assert!(res.is_err());
    match res.unwrap_err() {
        RestoreError::PostWriteValidationAndRollbackFailed {
            path,
            safety_backup,
            rollback_error,
        } => {
            assert_eq!(path, save_file);
            assert_eq!(safety_backup, missing_safety_backup);
            assert!(!rollback_error.is_empty());
        }
        other => panic!(
            "Esperava PostWriteValidationAndRollbackFailed, obteve {:?}",
            other
        ),
    }
}

#[test]
fn test_restore_rejects_reparse_point_when_allowed_root_does_not_exist_yet() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let sensitive_dir = temp_root.path().join("sensitive_outside");
    std::fs::create_dir_all(&sensitive_dir).unwrap();

    let junction_dir = temp_root.path().join("fake_junction");

    // Cria a junção NTFS: fake_junction -> sensitive_outside
    let status = std::process::Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            junction_dir.to_str().unwrap(),
            sensitive_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Falha ao invocar mklink /J");

    if !status.status.success() {
        eprintln!(
            "Aviso: ambiente não permitiu mklink /J: {}",
            String::from_utf8_lossy(&status.stderr)
        );
        return;
    }

    // allowed_root aponta para subpasta da junction que AINDA NÃO EXISTE!
    let allowed_root = junction_dir.join("subfolder_saves");
    assert!(
        !allowed_root.exists(),
        "allowed_root não deve existir ainda!"
    );

    let save_loc = SaveLocation::new(
        Uuid::new_v4(),
        allowed_root.clone(),
        SaveLocationType::Directory,
    );
    let malicious_target = allowed_root.join("pwned.sav");

    let result =
        savevault_app::restore::validate_destination_security(&malicious_target, &save_loc);

    assert!(
        result.is_err(),
        "Restauração onde allowed_root não existe mas ancestral é uma junction DEVE ser rejeitada!"
    );
    match result.unwrap_err() {
        RestoreError::PathSecurityViolation(msg) => {
            assert!(
                msg.contains("Reparse Point") || msg.contains("escapa"),
                "Mensagem de erro deve detalhar violação de reparse point: {}",
                msg
            );
        }
        other => panic!("Esperado PathSecurityViolation, obteve {:?}", other),
    }
}

#[test]
fn test_restore_accepts_valid_path_with_case_variation() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let allowed_root = temp_root.path().join("My_Game_Saves");
    std::fs::create_dir_all(&allowed_root).unwrap();

    let save_loc = SaveLocation::new(
        Uuid::new_v4(),
        allowed_root.clone(),
        SaveLocationType::Directory,
    );

    // Caminho com variação de case (minúsculas vs maiúsculas)
    let lower_path_str = allowed_root.to_str().unwrap().to_lowercase();
    let dest_file = PathBuf::from(lower_path_str).join("save_slot.sav");

    let result = savevault_app::restore::validate_destination_security(&dest_file, &save_loc);

    assert!(
        result.is_ok(),
        "Variação legítima de casing no Windows NÃO deve ser rejeitada como PathSecurityViolation: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_restore_transaction_multi_file_rollback_on_partial_failure() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn;
    let temp_root = tempdir().unwrap();
    let save_dir = temp_root.path().join("saves");
    let backup_dir = temp_root.path().join("backups");
    std::fs::create_dir_all(&save_dir).unwrap();
    std::fs::create_dir_all(&backup_dir).unwrap();

    let games_repo = GamesRepo::new(&conn);
    let loc_repo = SaveLocationsRepo::new(&conn);
    let files_repo = FilesRepo::new(&conn);
    let backups_repo = BackupsRepo::new(&conn);

    let game = Game::new(
        "MultiFileRPG",
        Platform::Steam,
        Some("12345".into()),
        Confidence::High,
    );
    games_repo.insert(&game).unwrap();

    let loc = SaveLocation::new(game.id, save_dir.clone(), SaveLocationType::Directory);
    let loc = loc_repo.upsert(&loc).unwrap();

    let file_a_path = save_dir.join("slot1.sav");
    let file_b_path = save_dir.join("slot1.meta");
    let file_c_path = save_dir.join("slot1.header");

    // Conteúdo original V1
    let v1_a = "SLOT1_DATA_V1";
    let v1_b = "SLOT1_META_V1";
    let v1_c = "SLOT1_HEADER_V1";

    std::fs::write(&file_a_path, v1_a).unwrap();
    std::fs::write(&file_b_path, v1_b).unwrap();
    std::fs::write(&file_c_path, v1_c).unwrap();

    let snapshot_mgr = SnapshotManager::new(backup_dir.clone(), 10);
    let snap_a_v1 = snapshot_mgr
        .create_snapshot(Uuid::new_v4(), &file_a_path, 1)
        .await
        .unwrap();
    let snap_b_v1 = snapshot_mgr
        .create_snapshot(Uuid::new_v4(), &file_b_path, 1)
        .await
        .unwrap();
    let snap_c_v1 = snapshot_mgr
        .create_snapshot(Uuid::new_v4(), &file_c_path, 1)
        .await
        .unwrap();

    // Registra V1 no DB
    let entry_a = FileEntry {
        id: snap_a_v1.file_id,
        save_location_id: loc.id,
        path: file_a_path.clone(),
        hash: "hash_v1_a".into(),
        size_bytes: v1_a.len() as u64,
        modified_at: Utc::now(),
    };
    let entry_b = FileEntry {
        id: snap_b_v1.file_id,
        save_location_id: loc.id,
        path: file_b_path.clone(),
        hash: "hash_v1_b".into(),
        size_bytes: v1_b.len() as u64,
        modified_at: Utc::now(),
    };
    let entry_c = FileEntry {
        id: snap_c_v1.file_id,
        save_location_id: loc.id,
        path: file_c_path.clone(),
        hash: "hash_v1_c".into(),
        size_bytes: v1_c.len() as u64,
        modified_at: Utc::now(),
    };
    files_repo.upsert(&entry_a).unwrap();
    files_repo.upsert(&entry_b).unwrap();
    files_repo.upsert(&entry_c).unwrap();
    backups_repo.insert(&snap_a_v1).unwrap();
    backups_repo.insert(&snap_b_v1).unwrap();
    backups_repo.insert(&snap_c_v1).unwrap();

    // Agora o jogo avança para V2 no disco do usuário
    let v2_a = "SLOT1_DATA_V2_CURRENT";
    let v2_b = "SLOT1_META_V2_CURRENT";
    let v2_c = "SLOT1_HEADER_V2_CURRENT";
    std::fs::write(&file_a_path, v2_a).unwrap();
    std::fs::write(&file_b_path, v2_b).unwrap();
    std::fs::write(&file_c_path, v2_c).unwrap();

    // Iniciamos uma transação para restaurar V1
    let mut tx = savevault_app::restore::RestoreTransaction::new(&backup_dir);
    tx.add_request(&conn, RestoreRequest::new(snap_a_v1.id))
        .await
        .unwrap();
    tx.add_request(&conn, RestoreRequest::new(snap_b_v1.id))
        .await
        .unwrap();
    tx.add_request(&conn, RestoreRequest::new(snap_c_v1.id))
        .await
        .unwrap();

    // Adversarial: Corrompemos intencionalmente o arquivo físico do snapshot C
    // para que a verificação de hash pós-escrita falhe no terceiro arquivo!
    let snap_c_physical_file = backup_dir
        .join(snap_c_v1.file_id.to_string())
        .join("v1")
        .join("slot1.header");
    // Altera a integridade do snapshot C
    std::fs::write(&snap_c_physical_file, "TAMPERED_HEADER_DATA").unwrap();

    // Executa a transação multi-arquivo
    let res = tx.execute(&conn).await;

    // A transação DEVE falhar devido ao erro de integridade em C
    assert!(
        res.is_err(),
        "Transação com falha de integridade em um arquivo DEVE falhar"
    );

    // E O MAIS IMPORTANTE:
    // NENHUM arquivo deve ter ficado em estado misto/inconsistente!
    // Arquivo A e B devem ter sofrido ROLLBACK de volta para V2!
    let current_a = std::fs::read_to_string(&file_a_path).unwrap();
    let current_b = std::fs::read_to_string(&file_b_path).unwrap();
    let current_c = std::fs::read_to_string(&file_c_path).unwrap();

    assert_eq!(
        current_a, v2_a,
        "Arquivo A deve ter sofrido rollback total para o estado anterior V2!"
    );
    assert_eq!(
        current_b, v2_b,
        "Arquivo B deve ter sofrido rollback total para o estado anterior V2!"
    );
    assert_eq!(
        current_c, v2_c,
        "Arquivo C deve ter preservado o estado anterior V2!"
    );
}

#[tokio::test]
async fn test_restore_transaction_multi_file_success() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn;
    let temp_root = tempdir().unwrap();
    let save_dir = temp_root.path().join("saves");
    let backup_dir = temp_root.path().join("backups");
    std::fs::create_dir_all(&save_dir).unwrap();
    std::fs::create_dir_all(&backup_dir).unwrap();

    let games_repo = GamesRepo::new(&conn);
    let loc_repo = SaveLocationsRepo::new(&conn);
    let files_repo = FilesRepo::new(&conn);
    let backups_repo = BackupsRepo::new(&conn);

    let game = Game::new(
        "MultiFileSuccessGame",
        Platform::Steam,
        Some("54321".into()),
        Confidence::High,
    );
    games_repo.insert(&game).unwrap();

    let loc = SaveLocation::new(game.id, save_dir.clone(), SaveLocationType::Directory);
    let loc = loc_repo.upsert(&loc).unwrap();

    let file_a_path = save_dir.join("save.dat");
    let file_b_path = save_dir.join("save.meta");

    let v1_a = "DATA_V1_CLEAN";
    let v1_b = "META_V1_CLEAN";
    std::fs::write(&file_a_path, v1_a).unwrap();
    std::fs::write(&file_b_path, v1_b).unwrap();

    let snapshot_mgr = SnapshotManager::new(backup_dir.clone(), 10);
    let snap_a = snapshot_mgr
        .create_snapshot(Uuid::new_v4(), &file_a_path, 1)
        .await
        .unwrap();
    let snap_b = snapshot_mgr
        .create_snapshot(Uuid::new_v4(), &file_b_path, 1)
        .await
        .unwrap();

    files_repo
        .upsert(&FileEntry {
            id: snap_a.file_id,
            save_location_id: loc.id,
            path: file_a_path.clone(),
            hash: "h_a".into(),
            size_bytes: v1_a.len() as u64,
            modified_at: Utc::now(),
        })
        .unwrap();
    files_repo
        .upsert(&FileEntry {
            id: snap_b.file_id,
            save_location_id: loc.id,
            path: file_b_path.clone(),
            hash: "h_b".into(),
            size_bytes: v1_b.len() as u64,
            modified_at: Utc::now(),
        })
        .unwrap();
    backups_repo.insert(&snap_a).unwrap();
    backups_repo.insert(&snap_b).unwrap();

    // Modifica para V2
    std::fs::write(&file_a_path, "DIRTY_A").unwrap();
    std::fs::write(&file_b_path, "DIRTY_B").unwrap();

    // Restauração com Two-Phase Commit
    let mut tx = savevault_app::restore::RestoreTransaction::new(&backup_dir);
    tx.add_request(&conn, RestoreRequest::new(snap_a.id))
        .await
        .unwrap();
    tx.add_request(&conn, RestoreRequest::new(snap_b.id))
        .await
        .unwrap();

    let reports = tx.execute(&conn).await.unwrap();
    assert_eq!(reports.len(), 2);

    assert_eq!(std::fs::read_to_string(&file_a_path).unwrap(), v1_a);
    assert_eq!(std::fs::read_to_string(&file_b_path).unwrap(), v1_b);
}

#[test]
fn test_cleanup_stale_staging_files_removes_orphans_only() {
    let temp_root = tempdir().unwrap();
    let root = temp_root.path();

    // 1. Arquivos de staging que devem ser limpos
    let stale_restore = root.join("save.sav.restore_tmp_12345678");
    let stale_download = root.join(".save.sav.download_tmp.87654321");
    let stale_manifest = root.join("manifest.yaml.tmp.abcdef12");
    std::fs::write(&stale_restore, "orphan_restore").unwrap();
    std::fs::write(&stale_download, "orphan_download").unwrap();
    std::fs::write(&stale_manifest, "orphan_manifest").unwrap();

    // 2. Arquivos legítimos do usuário que NUNCA devem ser deletados
    let legit_save = root.join("save.sav");
    let legit_backup = root.join("save.sav.savevault_safety_test");
    std::fs::write(&legit_save, "LEGIT_SAVE_DATA").unwrap();
    std::fs::write(&legit_backup, "SAFETY_BACKUP_DATA").unwrap();

    let removed = savevault_app::restore::cleanup_stale_staging_files(root).unwrap();
    assert_eq!(
        removed, 3,
        "Deve remover exatamente os 3 arquivos de staging órfãos"
    );

    assert!(!stale_restore.exists());
    assert!(!stale_download.exists());
    assert!(!stale_manifest.exists());

    assert!(
        legit_save.exists(),
        "Arquivo legítimo não pode ser removido!"
    );
    assert!(
        legit_backup.exists(),
        "Safety backup não pode ser removido!"
    );
}

#[test]
fn test_active_restore_guard_suppresses_and_releases() {
    use savevault_app::restore::{is_path_under_active_restore, ActiveRestoreGuard};

    let p1 = PathBuf::from(r"C:\Games\GameA\slot1.sav");
    let p2 = PathBuf::from(r"C:\Games\GameA\slot2.sav");

    assert!(!is_path_under_active_restore(&p1));
    assert!(!is_path_under_active_restore(&p2));

    {
        let _guard = ActiveRestoreGuard::new(&[p1.clone(), p2.clone()]);
        assert!(is_path_under_active_restore(&p1));
        assert!(is_path_under_active_restore(&p2));
        // Teste de case variation no Windows
        assert!(is_path_under_active_restore(&PathBuf::from(
            r"c:\games\gamea\slot1.sav"
        )));
    }

    // Após o drop do guard, os caminhos devem ser liberados
    assert!(!is_path_under_active_restore(&p1));
    assert!(!is_path_under_active_restore(&p2));
}
