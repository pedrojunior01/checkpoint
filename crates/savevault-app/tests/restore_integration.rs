//! Testes de Integração de Ponta a Ponta para Restauração de Snapshots (RES-01).

use chrono::Utc;
use std::path::PathBuf;
use tempfile::tempdir;
use uuid::Uuid;

use savevault_app::pipeline::handle_stable_event;
use savevault_app::restore::{restore_by_id, restore_snapshot, RestoreError, RestoreRequest};
use savevault_core::models::{Confidence, Game, Platform, SaveLocation, SaveLocationType};
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

    // 8. Confirma que os snapshots físicos v1 e v2 continuam intactos no vault (daily e latest)
    let v1_disk_path = backup_dir
        .join(&game.name)
        .join("daily")
        .join("save_slot_1.sav");
    let v2_disk_path = backup_dir
        .join(&game.name)
        .join("latest")
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
    let v1_dir = backup_root_dir(&backup_dir, &game.name, 1);
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

    let v2_dir = backup_root_dir(&backup_dir, &game.name, 2);
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

    let v1_dir = backup_root_dir(&backup_dir, &game.name, 1);
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

#[tokio::test]
async fn test_restore_version_greater_than_1_missing_fails_without_fallback_to_latest() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let db_path = temp_root.path().join("vault_missing_v2.db");
    let db = Database::open(&db_path).expect("Falha ao abrir banco");

    let backup_dir = temp_root.path().join("backups");
    let save_dir = temp_root.path().join("saves");
    std::fs::create_dir_all(&save_dir).unwrap();

    let save_file = save_dir.join("game.sav");
    std::fs::write(&save_file, "current_save").unwrap();

    let games_repo = GamesRepo::new(&db.conn);
    let loc_repo = SaveLocationsRepo::new(&db.conn);
    let files_repo = FilesRepo::new(&db.conn);
    let backups_repo = BackupsRepo::new(&db.conn);

    let game = Game::new("MissingV2Game", Platform::Generic, None, Confidence::High);
    games_repo.insert(&game).unwrap();

    let loc = SaveLocation::new(game.id, save_dir.clone(), SaveLocationType::Directory);
    let saved_loc = loc_repo.upsert(&loc).unwrap();

    let file_id = Uuid::new_v4();
    files_repo
        .upsert(&savevault_core::models::FileEntry {
            id: file_id,
            save_location_id: saved_loc.id,
            path: save_file.clone(),
            hash: "h".into(),
            size_bytes: 8,
            modified_at: Utc::now(),
        })
        .unwrap();

    // Cria apenas "latest", NÃO cria "v2"
    let latest_dir = backup_dir.join(&game.name).join("latest");
    std::fs::create_dir_all(&latest_dir).unwrap();
    std::fs::write(latest_dir.join("game.sav"), "latest_payload").unwrap();

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

    // Tentar restaurar v2 quando não existe v2 fisicamente DEVE falhar e NÃO cair no fallback de latest
    let res = restore_by_id(&db.conn, &backup_dir, b2.id).await;
    assert!(res.is_err());
    let err_str = res.unwrap_err().to_string();
    assert!(err_str.contains("não encontrado em"), "Erro: {}", err_str);

    // E o arquivo original não deve ter sido sobrescrito por latest_payload
    assert_eq!(std::fs::read_to_string(&save_file).unwrap(), "current_save");
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

fn backup_root_dir(backup_root: &std::path::Path, game_name: &str, version: u32) -> PathBuf {
    backup_root
        .join(game_name)
        .join(format!("v{}", version))
}
