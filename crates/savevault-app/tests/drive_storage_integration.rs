//! Testes de integração para a nova estratégia de armazenamento no Google Drive para computador.
//!
//! Valida:
//! 1. Configuração e validação de pasta (DriveRoot)
//! 2. Estrutura de diretórios por plataforma e jogo com sanitização adversarial de nomes
//! 3. Backup atômico seguro (staging .tmp -> flush -> hash BLAKE3 + tamanho -> atomic rename)
//! 4. Resiliência offline (Drive indisponível -> snapshot local intacto -> retry com sucesso)
//! 5. Restauração a partir do Drive (Drive -> staging local -> safety backup -> atomic replace -> verify)
//! 6. Concorrência entre watcher/pipeline, Drive sync e reconciliação.

use std::sync::Arc;
use std::time::SystemTime;
use tempfile::tempdir;
use tokio::sync::Mutex;
use uuid::Uuid;

use savevault_app::pipeline::{
    handle_stable_event, sync_all_pending_to_drive_folder, sync_backup_to_drive_folder,
};
use savevault_app::restore::{restore_snapshot_with_db_and_drive, RestoreRequest};
use savevault_app::Reconciler;
use savevault_backup::storage::{
    BackupSnapshotParams, BackupStorage, DriveFolderStatus, FilesystemBackupStorage,
};
use savevault_core::models::{
    Confidence, Game, Platform, SaveLocation, SaveLocationType, SyncStatus,
};
use savevault_core::path::{disambiguated_game_folder_name, sanitize_game_name};
use savevault_db::repositories::{BackupsRepo, FilesRepo, GamesRepo, SaveLocationsRepo};
use savevault_db::Database;
use savevault_monitor::events::{StableFileEvent, StableFileEventKind};

/// 1. Teste de Configuração e Validação de Pasta
#[test]
fn test_drive_folder_config_and_validation() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let valid_dir = temp_root.path().join("Meu Drive").join("SaveVault");
    std::fs::create_dir_all(&valid_dir).unwrap();

    let storage = FilesystemBackupStorage::new(valid_dir.clone());

    // Valida pasta existente
    let status = storage.validate_folder();
    assert!(
        status.is_available(),
        "Deveria ser disponível: {:?}",
        status
    );

    // Valida resolução inteligente do diretório base
    let base = storage.resolve_base_dir();
    assert!(base.ends_with("SaveVault"));

    // Se usuário passar pasta pai "Meu Drive", deve resolver para "Meu Drive\SaveVault"
    let parent_dir = temp_root.path().join("Meu Drive");
    let storage_parent = FilesystemBackupStorage::new(parent_dir);
    assert_eq!(storage_parent.resolve_base_dir(), valid_dir);

    // Pasta inexistente
    let non_existent = temp_root.path().join("Inexistente").join("Drive");
    let storage_invalid = FilesystemBackupStorage::new(non_existent.clone());
    let status_invalid = storage_invalid.validate_folder();
    assert!(
        matches!(status_invalid, DriveFolderStatus::NotFound { .. }),
        "Deveria ser NotFound: {:?}",
        status_invalid
    );
}

/// 2. Teste de Estrutura de Diretórios, Plataformas e Sanitização Adversarial
#[test]
fn test_drive_directory_structure_platforms_and_sanitization() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let drive_root = temp_root.path().join("Google Drive").join("SaveVault");
    std::fs::create_dir_all(&drive_root).unwrap();

    let storage = FilesystemBackupStorage::new(drive_root.clone());

    // Testa plataformas: Steam, GOG, Epic, Other
    assert_eq!(Platform::Steam.display_folder_name(), "Steam");
    assert_eq!(Platform::Gog.display_folder_name(), "GOG");
    assert_eq!(Platform::Epic.display_folder_name(), "Epic");
    assert_eq!(Platform::Generic.display_folder_name(), "Other");

    // Testa criação de estrutura de pastas
    let steam_dir = storage.ensure_game_structure(&Platform::Steam, "Half-Life 2", None);
    assert!(steam_dir.is_ok());
    let steam_paths = steam_dir.unwrap();
    assert!(steam_paths.snapshots_dir.exists());
    assert!(steam_paths.metadata_dir.exists());
    assert!(steam_paths.game_dir.to_str().unwrap().contains("Steam"));

    // Testa nomes adversariais e perigosos no Windows
    let test_cases = vec![
        ("CON", "CON reserved name"),
        ("AUX", "AUX reserved name"),
        ("NUL", "NUL reserved name"),
        ("PRN", "PRN reserved name"),
        ("COM1", "COM1 reserved name"),
        ("LPT1", "LPT1 reserved name"),
        ("CON.txt", "CON with extension reserved name"),
        ("PRN.log", "PRN with extension reserved name"),
        ("AUX.dat", "AUX with extension reserved name"),
        ("Game/Name", "Slash traversal"),
        ("Game\\Name", "Backslash traversal"),
        ("Game..", "Trailing dots"),
        ("..\\..\\Windows\\System32", "Path traversal"),
        ("Cyberpunk 2077: Phantom Liberty", "Colon replacement"),
        (
            "The Witcher® 3: Wild Hunt <GOTY>",
            "Unicode and angle brackets",
        ),
        ("🎮 Elfen Lied — 英雄传说", "Complex Unicode and emoji"),
    ];

    for (raw_name, desc) in test_cases {
        let stable_id = Uuid::new_v4();
        let sanitized = sanitize_game_name(raw_name, Some(stable_id));

        // Não pode conter separadores
        assert!(
            !sanitized.contains('/') && !sanitized.contains('\\'),
            "Falha em {}: contém separador",
            desc
        );
        // Não pode terminar em ponto ou espaço
        assert!(
            !sanitized.ends_with('.') && !sanitized.ends_with(' '),
            "Falha em {}: termina com ponto ou espaço",
            desc
        );
        // Não pode ser reservado do Windows
        assert!(
            !savevault_core::path::is_windows_reserved_name(&sanitized),
            "Falha em {}: nome reservado não tratado",
            desc
        );

        // Cria diretório no storage
        let game_paths = storage
            .ensure_game_structure(&Platform::Generic, raw_name, Some(stable_id))
            .unwrap_or_else(|e| panic!("Falha ao criar estrutura para {}: {:?}", desc, e));
        assert!(game_paths.snapshots_dir.is_dir());
        assert!(game_paths.metadata_dir.is_dir());
    }

    // Testa nomes ultra-longos (> 80 caracteres)
    let long_name = "A".repeat(120);
    let long_id = Uuid::new_v4();
    let sanitized_long = disambiguated_game_folder_name(&long_name, long_id);
    assert!(
        sanitized_long.len() <= 90,
        "Nome longo deveria ser truncado"
    );
    assert!(
        sanitized_long.contains(&long_id.to_string()[..4]),
        "Deveria conter id estável"
    );
}

/// 3. Teste de Backup Atômico Seguro, Hash BLAKE3 e Metadados
#[tokio::test]
async fn test_drive_backup_atomic_safety_and_hash_verification() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let db_path = temp_root.path().join("vault.db");
    let db = Database::open(&db_path).expect("Falha ao abrir banco");

    let backup_dir = temp_root.path().join("local_backups");
    let save_dir = temp_root.path().join("saves");
    let drive_root = temp_root.path().join("Google Drive").join("SaveVault");

    std::fs::create_dir_all(&save_dir).unwrap();
    std::fs::create_dir_all(&backup_dir).unwrap();
    std::fs::create_dir_all(&drive_root).unwrap();

    // 1. Cadastra Game e SaveLocation
    let game = Game::new(
        "Baldur's Gate 3",
        Platform::Steam,
        Some("1086940".into()),
        Confidence::High,
    );
    GamesRepo::new(&db.conn).insert(&game).unwrap();

    let save_loc = SaveLocation::new(game.id, save_dir.clone(), SaveLocationType::Directory);
    let saved_loc = SaveLocationsRepo::new(&db.conn).upsert(&save_loc).unwrap();

    // 2. Cria arquivo de save e gera snapshot local via pipeline
    let save_file = save_dir.join("quicksave.sav");
    let save_content = b"CONTEUDO_DO_SAVE_BG3_ULTRA_IMPORTANTE";
    std::fs::write(&save_file, save_content).unwrap();

    let event = StableFileEvent {
        save_location_id: saved_loc.id,
        path: save_file.clone(),
        kind: StableFileEventKind::Created,
        size_bytes: save_content.len() as u64,
        observed_at: SystemTime::now(),
    };
    handle_stable_event(&db.conn, &event, &backup_dir, 5)
        .await
        .unwrap();

    // Busca o arquivo e o backup gerado
    let file_entry = FilesRepo::new(&db.conn)
        .find_by_path(&save_file)
        .unwrap()
        .expect("Arquivo deve existir no banco");
    let backups = BackupsRepo::new(&db.conn)
        .list_by_file(file_entry.id)
        .unwrap();
    assert_eq!(backups.len(), 1);
    let backup = &backups[0];
    assert_eq!(backup.sync_status, SyncStatus::Pending);

    // 3. Sincroniza o snapshot para o Google Drive Filesystem
    let copied_ok =
        sync_backup_to_drive_folder(&db.conn, backup.id, &backup_dir, &drive_root).unwrap();
    assert!(copied_ok);

    let storage = FilesystemBackupStorage::new(drive_root.clone());
    let paths = storage.get_game_paths(&game.platform, &game.name, Some(game.id));
    let file_name = save_file.file_name().and_then(|n| n.to_str()).unwrap();
    let copied_path = paths
        .snapshots_dir
        .join(format!("v{}", backup.version))
        .join(file_name);
    assert!(copied_path.exists());
    assert!(!copied_path.to_string_lossy().ends_with(".tmp"));

    // Verifica que o hash no destino bate com o hash original do backup
    let drive_bytes = std::fs::read(&copied_path).unwrap();
    let drive_hash = blake3::hash(&drive_bytes).to_hex().to_string();
    assert_eq!(drive_hash, file_entry.hash);
    assert_eq!(drive_bytes.len() as u64, file_entry.size_bytes);

    // Verifica que o metadata JSON foi gerado no Drive
    let meta_file = paths
        .metadata_dir
        .join(format!("v{}_{}.json", backup.version, file_name));
    assert!(meta_file.exists(), "Metadata JSON deveria ter sido gravado");

    let meta_content = std::fs::read_to_string(&meta_file).unwrap();
    assert!(meta_content.contains(&file_entry.hash));
    assert!(meta_content.contains(&game.name));

    // Verifica que nenhum arquivo temporário (.tmp) restou
    let tmp_count = walkdir::WalkDir::new(&drive_root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
        .count();
    assert_eq!(tmp_count, 0, "Nenhum .tmp deve permanecer");

    // Verifica status no banco de dados atualizado para CopiedToDriveFolder
    let updated_backup = BackupsRepo::new(&db.conn)
        .find_by_id(backup.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        updated_backup.sync_status,
        SyncStatus::CopiedToDriveFolder,
        "Status deve ser CopiedToDriveFolder"
    );

    // 4. Teste de rejeição de hash divergente / corrupção
    let wrong_hash = "0000000000000000000000000000000000000000000000000000000000000000";
    let corrupt_params = BackupSnapshotParams {
        local_snapshot_path: copied_path.clone(),
        platform: Platform::Steam,
        game_name: game.name.clone(),
        game_id: Some(game.id),
        file_id: Some(backup.file_id),
        backup_id: Some(backup.id),
        version: backup.version + 1,
        file_name: "corrupt.sav".to_string(),
        expected_hash: Some(wrong_hash.to_string()),
        expected_size: Some(drive_bytes.len() as u64),
    };
    let corrupt_result = storage.backup_snapshot(corrupt_params);

    assert!(
        corrupt_result.is_err(),
        "Upload com hash divergente deve ser rejeitado"
    );

    // Confirma que nenhum arquivo temporário ficou órfão após o erro
    let tmp_after_err = walkdir::WalkDir::new(&drive_root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
        .count();
    assert_eq!(
        tmp_after_err, 0,
        "Arquivo temporário deve ter sido limpo no erro"
    );
}

/// 4. Teste de Resiliência Offline (Drive Indisponível)
#[tokio::test]
async fn test_drive_offline_resilience() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let db_path = temp_root.path().join("vault.db");
    let db = Database::open(&db_path).expect("Falha ao abrir banco");

    let backup_dir = temp_root.path().join("local_backups");
    let save_dir = temp_root.path().join("saves");
    // Pasta do Drive propositalmente inexistente/desconectada
    let offline_drive_root = temp_root.path().join("X_Drive_Offline").join("SaveVault");

    std::fs::create_dir_all(&save_dir).unwrap();
    std::fs::create_dir_all(&backup_dir).unwrap();

    let game = Game::new(
        "Elden Ring",
        Platform::Steam,
        Some("1245620".into()),
        Confidence::High,
    );
    GamesRepo::new(&db.conn).insert(&game).unwrap();

    let save_loc = SaveLocation::new(game.id, save_dir.clone(), SaveLocationType::Directory);
    let saved_loc = SaveLocationsRepo::new(&db.conn).upsert(&save_loc).unwrap();

    let save_file = save_dir.join("ER0000.sl2");
    std::fs::write(&save_file, "ELDEN_RING_SAVE_OFFLINE_TEST").unwrap();

    // Criação do snapshot local deve ter sucesso mesmo com Drive offline
    let event = StableFileEvent {
        save_location_id: saved_loc.id,
        path: save_file.clone(),
        kind: StableFileEventKind::Created,
        size_bytes: "ELDEN_RING_SAVE_OFFLINE_TEST".len() as u64,
        observed_at: SystemTime::now(),
    };
    handle_stable_event(&db.conn, &event, &backup_dir, 5)
        .await
        .unwrap();

    let file_entry = FilesRepo::new(&db.conn)
        .find_by_path(&save_file)
        .unwrap()
        .expect("Arquivo deve existir no banco");
    let backups = BackupsRepo::new(&db.conn)
        .list_by_file(file_entry.id)
        .unwrap();
    assert_eq!(backups.len(), 1);
    let backup = &backups[0];
    let file_name = save_file.file_name().and_then(|n| n.to_str()).unwrap();
    let local_snapshot_path = backup_dir
        .join(backup.file_id.to_string())
        .join(format!("v{}", backup.version))
        .join(file_name);
    assert!(
        local_snapshot_path.exists(),
        "Snapshot local deve existir fisicamente: {}",
        local_snapshot_path.display()
    );

    // Tentativa de sincronizar para o Drive offline retorna false sem quebrar
    let sync_res =
        sync_backup_to_drive_folder(&db.conn, backup.id, &backup_dir, &offline_drive_root);

    assert!(
        sync_res.is_ok() && !sync_res.unwrap(),
        "Deve retornar Ok(false) gracioso quando o Drive está inacessível"
    );

    // Invariante de ouro: O snapshot local e o registro no banco DEVEM permanecer 100% intactos!
    assert!(
        local_snapshot_path.exists(),
        "Snapshot local NUNCA deve ser apagado em caso de erro de Drive"
    );
    let backup_check = BackupsRepo::new(&db.conn)
        .find_by_id(backup.id)
        .unwrap()
        .unwrap();
    assert_eq!(backup_check.sync_status, SyncStatus::PendingOffline);

    // Agora simula o Drive voltando a ficar online
    std::fs::create_dir_all(&offline_drive_root).unwrap();

    // Retry com sucesso
    let sync_ok =
        sync_backup_to_drive_folder(&db.conn, backup.id, &backup_dir, &offline_drive_root);
    assert!(
        sync_ok.is_ok() && sync_ok.unwrap(),
        "Retry deve ter sucesso após reconexão"
    );

    let backup_synced = BackupsRepo::new(&db.conn)
        .find_by_id(backup.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        backup_synced.sync_status,
        SyncStatus::CopiedToDriveFolder,
        "Status atualizado para CopiedToDriveFolder"
    );
}

/// 5. Teste de Restauração a partir do Drive (com Staging, Safety Backup e Transação Segura)
#[tokio::test]
async fn test_drive_restore_with_staging_and_safety_backup() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let db_path = temp_root.path().join("vault.db");
    let db = Database::open(&db_path).expect("Falha ao abrir banco");

    let backup_dir = temp_root.path().join("local_backups");
    let save_dir = temp_root.path().join("saves");
    let drive_root = temp_root.path().join("Google Drive").join("SaveVault");

    std::fs::create_dir_all(&save_dir).unwrap();
    std::fs::create_dir_all(&backup_dir).unwrap();
    std::fs::create_dir_all(&drive_root).unwrap();

    let game = Game::new(
        "Celeste",
        Platform::Steam,
        Some("504230".into()),
        Confidence::High,
    );
    GamesRepo::new(&db.conn).insert(&game).unwrap();

    let save_loc = SaveLocation::new(game.id, save_dir.clone(), SaveLocationType::Directory);
    let saved_loc = SaveLocationsRepo::new(&db.conn).upsert(&save_loc).unwrap();

    let save_file = save_dir.join("0.celeste");
    let original_save_bytes = b"CELESTE_CHAPTER_9_FAREWELL_COMPLETED";
    std::fs::write(&save_file, original_save_bytes).unwrap();

    // Cria snapshot
    let event = StableFileEvent {
        save_location_id: saved_loc.id,
        path: save_file.clone(),
        kind: StableFileEventKind::Created,
        size_bytes: original_save_bytes.len() as u64,
        observed_at: SystemTime::now(),
    };
    handle_stable_event(&db.conn, &event, &backup_dir, 5)
        .await
        .unwrap();

    let file_entry = FilesRepo::new(&db.conn)
        .find_by_path(&save_file)
        .unwrap()
        .expect("Arquivo deve existir no banco");
    let backups = BackupsRepo::new(&db.conn)
        .list_by_file(file_entry.id)
        .unwrap();
    let backup = &backups[0];

    // Copia para o Drive
    let drive_copy_ok =
        sync_backup_to_drive_folder(&db.conn, backup.id, &backup_dir, &drive_root).unwrap();
    assert!(drive_copy_ok);

    // SIMULAÇÃO CRÍTICA: Apaga o snapshot local do cofre!
    // Agora o snapshot existe APENAS no Google Drive!
    let file_name = save_file.file_name().and_then(|n| n.to_str()).unwrap();
    let local_snapshot_file = backup_dir
        .join(backup.file_id.to_string())
        .join(format!("v{}", backup.version))
        .join(file_name);
    std::fs::remove_file(&local_snapshot_file).unwrap();
    assert!(
        !local_snapshot_file.exists(),
        "Snapshot local deve ter sido removido"
    );

    // Modifica o save original para simular dano / corrupção no jogo
    let corrupted_save_bytes = b"CORRUPTED_OR_OVERWRITTEN_SAVE_DATA";
    std::fs::write(&save_file, corrupted_save_bytes).unwrap();

    // Executa Restore via restore_snapshot_with_db_and_drive
    let db_mutex = Mutex::new(db.conn);
    let request = RestoreRequest::new(backup.id);
    let report =
        restore_snapshot_with_db_and_drive(&db_mutex, &backup_dir, Some(&drive_root), &request)
            .await
            .expect("Restore via Drive deve ter sucesso");

    // Verifica que a restauração completou
    assert!(report.dest_path.exists());
    let restored_bytes = std::fs::read(&report.dest_path).unwrap();
    assert_eq!(
        restored_bytes, original_save_bytes,
        "Conteúdo restaurado deve ser idêntico ao original"
    );

    // Verifica que o Safety Backup foi criado compulsoriamente antes do overwrite
    assert!(
        report.safety_backup_path.is_some(),
        "Safety backup deve ter sido gerado"
    );
    let safety_path = report.safety_backup_path.unwrap();
    assert!(
        safety_path.exists(),
        "Arquivo de safety backup deve existir"
    );
    let safety_bytes = std::fs::read(&safety_path).unwrap();
    assert_eq!(
        safety_bytes, corrupted_save_bytes,
        "Safety backup deve guardar o estado anterior (corrompido)"
    );

    // Verifica que o staging local foi preenchido e verificado
    assert!(
        local_snapshot_file.exists(),
        "Snapshot deve ter sido restaurado para o staging local com verificação de hash"
    );
}

/// 6. Teste de Concorrência: Watcher, Drive Sync e Reconciliação
#[tokio::test]
async fn test_concurrency_drive_sync_and_reconciliation() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let db_path = temp_root.path().join("vault.db");
    let db = Database::open(&db_path).expect("Falha ao abrir banco");

    let backup_dir = temp_root.path().join("local_backups");
    let save_dir = temp_root.path().join("saves");
    let drive_root = temp_root.path().join("Google Drive").join("SaveVault");

    std::fs::create_dir_all(&save_dir).unwrap();
    std::fs::create_dir_all(&backup_dir).unwrap();
    std::fs::create_dir_all(&drive_root).unwrap();

    let game = Game::new(
        "Hades II",
        Platform::Steam,
        Some("1145350".into()),
        Confidence::High,
    );
    GamesRepo::new(&db.conn).insert(&game).unwrap();

    let save_loc = SaveLocation::new(game.id, save_dir.clone(), SaveLocationType::Directory);
    let _saved_loc = SaveLocationsRepo::new(&db.conn).upsert(&save_loc).unwrap();

    // Cria arquivos iniciais
    for i in 1..=5 {
        let fpath = save_dir.join(format!("Profile{}.sav", i));
        std::fs::write(&fpath, format!("HADES_DATA_SLOT_{}", i)).unwrap();
    }
    let extra_file = save_dir.join("Profile_Extra.sav");
    std::fs::write(&extra_file, "HADES_EXTRA_DATA").unwrap();

    let db_mutex = Arc::new(Mutex::new(db.conn));
    let reconciler = Arc::new(
        Reconciler::new(db_mutex.clone(), backup_dir.clone(), 5)
            .with_drive_dir(Some(drive_root.clone())),
    );

    // Executa reconciliação e sync concorrentemente para estressar transações e I/O
    let recon_clone = reconciler.clone();
    let recon_handle = tokio::spawn(async move { recon_clone.reconcile().await });

    let db_clone = db_mutex.clone();
    let b_dir = backup_dir.clone();
    let d_root = drive_root.clone();
    let sync_handle = tokio::spawn(async move {
        let conn = db_clone.lock().await;
        sync_all_pending_to_drive_folder(&conn, &b_dir, &d_root)
    });

    let (recon_res, sync_res) = tokio::join!(recon_handle, sync_handle);
    assert!(recon_res.unwrap().is_ok());
    assert!(sync_res.unwrap().is_ok());

    // Ciclo final para garantir que qualquer save processado pela outra thread seja sincronizado
    let _ = reconciler.reconcile().await;
    {
        let conn = db_mutex.lock().await;
        let _ = sync_all_pending_to_drive_folder(&conn, &backup_dir, &drive_root);
    }

    // Confirma que todos os 6 snapshots foram copiados para a pasta do Drive
    let storage = FilesystemBackupStorage::new(drive_root.clone());
    let drive_snapshots = storage
        .list_snapshots(&Platform::Steam, &game.name, Some(game.id))
        .unwrap();
    assert_eq!(drive_snapshots.len(), 6);
}

/// 7. Teste de Sincronização de Múltiplas Versões e Idempotência Rigorosa
#[tokio::test]
async fn test_drive_multi_version_sync_and_idempotency() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let db_path = temp_root.path().join("vault.db");
    let db = Database::open(&db_path).expect("Falha ao abrir banco");

    let backup_dir = temp_root.path().join("local_backups");
    let save_dir = temp_root.path().join("saves");
    let drive_root = temp_root.path().join("Google Drive").join("SaveVault");

    std::fs::create_dir_all(&save_dir).unwrap();
    std::fs::create_dir_all(&backup_dir).unwrap();
    std::fs::create_dir_all(&drive_root).unwrap();

    let game = Game::new(
        "Dark Souls",
        Platform::Steam,
        Some("211420".into()),
        Confidence::High,
    );
    GamesRepo::new(&db.conn).insert(&game).unwrap();

    let save_loc = SaveLocation::new(game.id, save_dir.clone(), SaveLocationType::Directory);
    let saved_loc = SaveLocationsRepo::new(&db.conn).upsert(&save_loc).unwrap();

    let save_file = save_dir.join("DRAKS0005.sl2");

    // Versão 1: Conteúdo curto (10 bytes)
    let v1_data = b"V1_CONTENT";
    std::fs::write(&save_file, v1_data).unwrap();
    let ev1 = StableFileEvent {
        save_location_id: saved_loc.id,
        path: save_file.clone(),
        kind: StableFileEventKind::Created,
        size_bytes: v1_data.len() as u64,
        observed_at: SystemTime::now(),
    };
    handle_stable_event(&db.conn, &ev1, &backup_dir, 5)
        .await
        .unwrap();

    // Versão 2: Conteúdo mais longo (26 bytes)
    let v2_data = b"V2_MUCH_LONGER_CONTENT_NOW";
    std::fs::write(&save_file, v2_data).unwrap();
    let ev2 = StableFileEvent {
        save_location_id: saved_loc.id,
        path: save_file.clone(),
        kind: StableFileEventKind::Modified,
        size_bytes: v2_data.len() as u64,
        observed_at: SystemTime::now(),
    };
    handle_stable_event(&db.conn, &ev2, &backup_dir, 5)
        .await
        .unwrap();

    // Confirma que temos 2 versões no banco
    let file_entry = FilesRepo::new(&db.conn)
        .find_by_path(&save_file)
        .unwrap()
        .unwrap();
    let backups = BackupsRepo::new(&db.conn)
        .list_by_file(file_entry.id)
        .unwrap();
    assert_eq!(backups.len(), 2, "Devem existir 2 backups locais");

    // Sincroniza todos os backups pendentes para o Drive
    let copied_count =
        sync_all_pending_to_drive_folder(&db.conn, &backup_dir, &drive_root).unwrap();
    assert_eq!(
        copied_count, 2,
        "Ambas as versões (v1 e v2) devem ser copiadas para o Drive"
    );

    // Verifica que cada versão física no Drive tem o seu tamanho exato
    let storage = FilesystemBackupStorage::new(drive_root.clone());
    let paths = storage.get_game_paths(&game.platform, &game.name, Some(game.id));

    let drive_v1 = paths.snapshots_dir.join("v1").join("DRAKS0005.sl2");
    let drive_v2 = paths.snapshots_dir.join("v2").join("DRAKS0005.sl2");

    assert!(drive_v1.exists(), "v1 deve existir no Drive");
    assert!(drive_v2.exists(), "v2 deve existir no Drive");

    assert_eq!(std::fs::metadata(&drive_v1).unwrap().len(), 10);
    assert_eq!(std::fs::metadata(&drive_v2).unwrap().len(), 26);

    // Confirma idempotência estrita: 10 execuções seguidas sem modificações realizam ZERO cópias
    for _ in 1..=10 {
        let no_op_count =
            sync_all_pending_to_drive_folder(&db.conn, &backup_dir, &drive_root).unwrap();
        assert_eq!(
            no_op_count, 0,
            "Reconciliação sem alterações não deve realizar novas cópias"
        );
    }
}

/// 8. Teste de Supressão de Eventos Concorrentes do Watcher durante Restauração
#[tokio::test]
async fn test_restore_active_suppression_prevents_watcher_loop() {
    let temp_root = tempdir().expect("Falha ao criar tempdir");
    let db_path = temp_root.path().join("vault.db");
    let db = Database::open(&db_path).expect("Falha ao abrir banco");

    let backup_dir = temp_root.path().join("local_backups");
    let save_dir = temp_root.path().join("saves");

    std::fs::create_dir_all(&save_dir).unwrap();
    std::fs::create_dir_all(&backup_dir).unwrap();

    let game = Game::new(
        "Celeste",
        Platform::Steam,
        Some("504230".into()),
        Confidence::High,
    );
    GamesRepo::new(&db.conn).insert(&game).unwrap();

    let save_loc = SaveLocation::new(game.id, save_dir.clone(), SaveLocationType::Directory);
    let saved_loc = SaveLocationsRepo::new(&db.conn).upsert(&save_loc).unwrap();

    let save_file = save_dir.join("0.celeste");
    std::fs::write(&save_file, "CELESTE_ORIGINAL_SAVE").unwrap();

    let ev = StableFileEvent {
        save_location_id: saved_loc.id,
        path: save_file.clone(),
        kind: StableFileEventKind::Created,
        size_bytes: "CELESTE_ORIGINAL_SAVE".len() as u64,
        observed_at: SystemTime::now(),
    };
    handle_stable_event(&db.conn, &ev, &backup_dir, 5)
        .await
        .unwrap();

    // Modifica o save
    std::fs::write(&save_file, "CELESTE_MODIFIED_STATE").unwrap();

    // Obtém o backup original
    let file_entry = FilesRepo::new(&db.conn)
        .find_by_path(&save_file)
        .unwrap()
        .unwrap();
    let backups = BackupsRepo::new(&db.conn)
        .list_by_file(file_entry.id)
        .unwrap();
    let orig_backup = &backups[0];

    // Simula restauração e verifica supressão de evento no pipeline
    let db_mutex = Arc::new(Mutex::new(db.conn));
    let restore_req = RestoreRequest::new(orig_backup.id);

    let res = restore_snapshot_with_db_and_drive(&db_mutex, &backup_dir, None, &restore_req).await;
    assert!(res.is_ok(), "Restauração deve ter sucesso");

    // Verifica que o arquivo foi restaurado para o conteúdo original
    let restored_content = std::fs::read_to_string(&save_file).unwrap();
    assert_eq!(restored_content, "CELESTE_ORIGINAL_SAVE");

    // Testa que enquanto um caminho estiver sob ActiveRestoreGuard, process_file suprime o evento
    {
        let _guard =
            savevault_app::restore::ActiveRestoreGuard::new(std::slice::from_ref(&save_file));
        assert!(savevault_app::restore::is_path_under_active_restore(
            &save_file
        ));

        let suppressed_event = StableFileEvent {
            save_location_id: saved_loc.id,
            path: save_file.clone(),
            kind: StableFileEventKind::Modified,
            size_bytes: 20,
            observed_at: SystemTime::now(),
        };

        let process_res =
            savevault_app::pipeline::process_file(&db_mutex, &suppressed_event, &backup_dir, 5)
                .await
                .unwrap();

        assert!(
            process_res.is_none(),
            "Evento gerado durante restore deve ser suprimido para evitar loops"
        );
    }

    // Após o drop do guard, o caminho não está mais sob restauração ativa
    assert!(!savevault_app::restore::is_path_under_active_restore(
        &save_file
    ));
}
