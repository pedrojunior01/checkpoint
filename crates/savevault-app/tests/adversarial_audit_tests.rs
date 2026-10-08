//! Testes Adversariais de Segurança, Integridade e Concorrência.
//!
//! Cobre:
//! 1. Prefixos estendidos do Windows (\\?\ e \\?\UNC\), cross-drive e caminhos protegidos do macOS.
//! 2. Rejeição de symlinks/junções direcionados para pastas protegidas do sistema.
//! 3. Bloqueio de sobrescrita de symlinks durante a restauração (TOCTOU / Symlink Escape).
//! 4. Bloqueio de subdiretórios com junções que escapam do SaveLocation.
//! 5. Disaster recovery com múltiplos Save Locations para o mesmo jogo (ausência de hijacking).
//! 6. Ausência de colisão com operador SQL LIKE (ex: autosave.dat vs save.dat).
//! 7. Atomicidade e rollback de SaveLocationsRepo::update_mapping via savepoint.

use chrono::Utc;
use std::path::{Path, PathBuf};
use tempfile::tempdir;
use uuid::Uuid;

use savevault_app::cloud_recovery::reconstruct_catalog_from_cloud;
use savevault_app::remapping::validate_candidate_path;
use savevault_app::restore::{validate_destination_security, RestoreError};
use savevault_cloud::{
    CloudCatalogManifest, CloudManifestEntry, CloudProvider, LocalFolderProvider,
};
use savevault_core::models::{
    Confidence, FileEntry, Game, MappingSource, MappingStatus, Platform, SaveLocation,
    SaveLocationType,
};
use savevault_core::path::{is_system_protected_path, validate_remapping_path_security};
use savevault_db::repositories::{FilesRepo, GamesRepo, SaveLocationsRepo};
use savevault_db::Database;

// =============================================================================
// 1. AUDITORIA DE PATHS E PREFIXOS ESTENDIDOS
// =============================================================================

#[test]
fn test_extended_prefix_and_unc_system_protected_paths() {
    // Windows: Prefixo estendido em C: e outros drives (D:, E:, Z:)
    assert!(is_system_protected_path(Path::new(r"C:\Windows")));
    assert!(is_system_protected_path(Path::new(r"\\?\C:\Windows")));
    assert!(is_system_protected_path(Path::new(
        r"\\?\C:\Windows\System32"
    )));
    assert!(is_system_protected_path(Path::new(r"D:\Windows\System32")));
    assert!(is_system_protected_path(Path::new(
        r"E:\Program Files\Common Files"
    )));
    assert!(is_system_protected_path(Path::new(
        r"\\?\E:\Program Files (x86)\App"
    )));
    assert!(is_system_protected_path(Path::new(r"C:\pagefile.sys")));
    assert!(is_system_protected_path(Path::new(r"D:\swapfile.sys")));
    assert!(is_system_protected_path(Path::new(
        r"C:\System Volume Information"
    )));

    // Windows: UNC Admin Shares
    assert!(is_system_protected_path(Path::new(
        r"\\fileserver\c$\windows"
    )));
    assert!(is_system_protected_path(Path::new(
        r"\\fileserver\admin$\system32"
    )));
    assert!(is_system_protected_path(Path::new(
        r"\\?\UNC\fileserver\c$\boot"
    )));

    // Unix / Linux / macOS
    assert!(is_system_protected_path(Path::new("/bin/sh")));
    assert!(is_system_protected_path(Path::new("/sbin/init")));
    assert!(is_system_protected_path(Path::new("/etc/sudoers")));
    assert!(is_system_protected_path(Path::new("/usr/bin/python3")));
    assert!(is_system_protected_path(Path::new(
        "/System/Library/CoreServices"
    )));
    assert!(is_system_protected_path(Path::new("/Library/Preferences")));
    assert!(is_system_protected_path(Path::new(
        "/Applications/TextEdit.app"
    )));
    assert!(is_system_protected_path(Path::new("/private/etc/hosts")));

    // Pastas legítimas de usuário NÃO devem ser bloqueadas
    assert!(!is_system_protected_path(Path::new(
        r"C:\Users\Alice\Saved Games\EldenRing"
    )));
    assert!(!is_system_protected_path(Path::new(
        r"D:\Games\Saves\MyGame"
    )));
    assert!(!is_system_protected_path(Path::new(
        "/home/alice/.local/share/mygame"
    )));
    assert!(!is_system_protected_path(Path::new(
        "/Users/alice/Library/Application Support/mygame"
    )));
}

// =============================================================================
// 2. AUDITORIA DE SYMLINK / JUNCTION EM REMAPPING
// =============================================================================

#[test]
fn test_symlink_or_junction_to_protected_directory_blocked() {
    let tmp = tempdir().unwrap();
    let link_target = if cfg!(windows) {
        PathBuf::from(r"C:\Windows")
    } else {
        PathBuf::from("/bin")
    };

    let link_path = tmp.path().join("evil_system_link");

    #[cfg(windows)]
    let symlink_created = std::os::windows::fs::symlink_dir(&link_target, &link_path).is_ok();
    #[cfg(not(windows))]
    let symlink_created = std::os::unix::fs::symlink(&link_target, &link_path).is_ok();

    if symlink_created {
        // O caminho aponta para o link aparentemente inocente dentro do tempdir,
        // mas sua resolução física aponta para a pasta protegida
        let sec_res = validate_remapping_path_security(&link_path);
        assert!(
            sec_res.is_err(),
            "Link simbólico para pasta protegida deveria ser rejeitado!"
        );
        let err_msg = sec_res.err().unwrap();
        assert!(err_msg.contains("protegido"));

        let val_res = validate_candidate_path(&link_path);
        assert!(!val_res.valid);

        // Subpasta dentro do link também deve ser rejeitada
        let sub_link = link_path.join("System32").join("cmd.exe");
        assert!(validate_remapping_path_security(&sub_link).is_err());
    }
}

// =============================================================================
// 3. AUDITORIA DE RESTORE CONTRA SYMLINKS DE DESTINO
// =============================================================================

#[test]
fn test_restore_rejects_destination_symlink() {
    let tmp = tempdir().unwrap();
    let save_loc_path = tmp.path().join("SafeGameSaves");
    std::fs::create_dir_all(&save_loc_path).unwrap();

    let save_location = SaveLocation {
        id: Uuid::new_v4(),
        game_id: Uuid::new_v4(),
        path: save_loc_path.clone(),
        location_type: SaveLocationType::Directory,
        original_path: None,
        mapping_status: MappingStatus::Resolved,
        mapping_source: MappingSource::Original,
        resolution_reason: None,
    };

    let target_file = save_loc_path.join("actual_target.txt");
    std::fs::write(&target_file, b"secret data").unwrap();

    let dest_file_symlink = save_loc_path.join("save.dat");

    #[cfg(windows)]
    let link_created = std::os::windows::fs::symlink_file(&target_file, &dest_file_symlink).is_ok();
    #[cfg(not(windows))]
    let link_created = std::os::unix::fs::symlink(&target_file, &dest_file_symlink).is_ok();

    if link_created {
        let res = validate_destination_security(&dest_file_symlink, &save_location);
        match res {
            Err(RestoreError::PathSecurityViolation(msg)) => {
                assert!(
                    msg.contains("symlink não autorizado"),
                    "Mensagem esperada sobre symlink: {}",
                    msg
                );
            }
            other => panic!(
                "Esperava rejeição de destino como symlink, obteve: {:?}",
                other
            ),
        }
    }
}

#[test]
fn test_restore_rejects_escaping_via_parent_junction() {
    let tmp = tempdir().unwrap();
    let outside_protected = tmp.path().join("outside_folder");
    std::fs::create_dir_all(&outside_protected).unwrap();

    let save_loc_path = tmp.path().join("AuthorizedSaveLocation");
    std::fs::create_dir_all(&save_loc_path).unwrap();

    let save_location = SaveLocation {
        id: Uuid::new_v4(),
        game_id: Uuid::new_v4(),
        path: save_loc_path.clone(),
        location_type: SaveLocationType::Directory,
        original_path: None,
        mapping_status: MappingStatus::Resolved,
        mapping_source: MappingSource::Original,
        resolution_reason: None,
    };

    let link_in_saves = save_loc_path.join("junction_out");

    #[cfg(windows)]
    let link_created =
        std::os::windows::fs::symlink_dir(&outside_protected, &link_in_saves).is_ok();
    #[cfg(not(windows))]
    let link_created = std::os::unix::fs::symlink(&outside_protected, &link_in_saves).is_ok();

    if link_created {
        let dest_escaping = link_in_saves.join("escaped_file.dat");
        let res = validate_destination_security(&dest_escaping, &save_location);
        match res {
            Err(RestoreError::PathSecurityViolation(msg)) => {
                assert!(
                    msg.contains("escapa"),
                    "Mensagem esperada sobre escape: {}",
                    msg
                );
            }
            other => panic!(
                "Esperava rejeição de escape via junction, obteve: {:?}",
                other
            ),
        }
    }
}

// =============================================================================
// 4. AUDITORIA DE RECONSTRUÇÃO: MÚLTIPLOS LOCAIS DE SAVE POR JOGO
// =============================================================================

#[tokio::test]
async fn test_multi_location_game_disaster_recovery() {
    let db = Database::open_in_memory().unwrap();
    let cloud_tmp = tempdir().unwrap();
    let provider = LocalFolderProvider::new(cloud_tmp.path().to_path_buf());

    // Cria manifest com 1 jogo tendo 2 Save Locations distintas
    let entry_docs = CloudManifestEntry {
        game_name: "Baldurs Gate 3".to_string(),
        platform: "pc".to_string(),
        save_location_path: r"C:\Users\Alice\Documents\Larian Studios\Baldurs Gate 3\Savegames"
            .to_string(),
        relative_file_path: "save1.sav".to_string(),
        version: 1,
        cloud_path: "baldurs_gate_3/v1/save1.sav".to_string(),
        file_hash: "blake3_hash_docs_12345".to_string(),
        created_at: Utc::now().to_rfc3339(),
        cloud_file_id: "remote_id_docs_001".to_string(),
    };

    let entry_appdata = CloudManifestEntry {
        game_name: "Baldurs Gate 3".to_string(),
        platform: "pc".to_string(),
        save_location_path:
            r"C:\Users\Alice\AppData\Local\Larian Studios\Baldurs Gate 3\PlayerProfiles".to_string(),
        relative_file_path: "profile.json".to_string(),
        version: 1,
        cloud_path: "baldurs_gate_3/v1/profile.json".to_string(),
        file_hash: "blake3_hash_appdata_67890".to_string(),
        created_at: Utc::now().to_rfc3339(),
        cloud_file_id: "remote_id_appdata_002".to_string(),
    };

    let manifest = CloudCatalogManifest {
        schema_version: 1,
        updated_at: Utc::now().to_rfc3339(),
        entries: vec![entry_docs, entry_appdata],
    };

    provider.save_manifest(&manifest).await.unwrap();

    // Executa a reconstrução do catálogo
    let imported = reconstruct_catalog_from_cloud(&db.conn, &provider)
        .await
        .unwrap();
    assert_eq!(imported, 2);

    let games_repo = GamesRepo::new(&db.conn);
    let locs_repo = SaveLocationsRepo::new(&db.conn);

    let games = games_repo.list_all().unwrap();
    assert_eq!(games.len(), 1, "Deve existir exatamente 1 jogo");
    let game = &games[0];
    assert_eq!(game.name, "Baldurs Gate 3");

    let locs = locs_repo.list_by_game(game.id).unwrap();
    assert_eq!(
        locs.len(),
        2,
        "O jogo deve possuir EXATAMENTE 2 Save Locations independentes!"
    );

    let loc1 = &locs[0];
    let loc2 = &locs[1];
    assert_ne!(loc1.id, loc2.id);

    // Consulta arquivos registrados
    let mut stmt = db
        .conn
        .prepare("SELECT save_location_id, path FROM files")
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            let loc_id: String = r.get(0)?;
            let p: String = r.get(1)?;
            Ok((loc_id, p))
        })
        .unwrap();
    let files: Vec<(String, String)> = rows.flatten().collect();
    assert_eq!(files.len(), 2);

    assert!(files
        .iter()
        .any(|(lid, _p)| lid == &loc1.id.to_string() || lid == &loc2.id.to_string()));

    // Segunda execução da recuperação (idempotência): não pode duplicar nem corromper
    let second_imported = reconstruct_catalog_from_cloud(&db.conn, &provider)
        .await
        .unwrap();
    assert_eq!(second_imported, 0);

    let locs_after = locs_repo.list_by_game(game.id).unwrap();
    assert_eq!(locs_after.len(), 2);
}

// =============================================================================
// 5. AUDITORIA DE RECONSTRUÇÃO: AUSÊNCIA DE COLISÃO SQL LIKE
// =============================================================================

#[tokio::test]
async fn test_reconstruct_catalog_no_fuzzy_like_file_collision() {
    let db = Database::open_in_memory().unwrap();
    let cloud_tmp = tempdir().unwrap();
    let provider = LocalFolderProvider::new(cloud_tmp.path().to_path_buf());

    // Dois arquivos onde o segundo é sufixo do primeiro ("autosave.dat" e "save.dat")
    let entry_autosave = CloudManifestEntry {
        game_name: "Witcher 3".to_string(),
        platform: "pc".to_string(),
        save_location_path: r"C:\Games\Witcher3\Saves".to_string(),
        relative_file_path: "autosave.dat".to_string(),
        version: 1,
        cloud_path: "witcher_3/v1/autosave.dat".to_string(),
        file_hash: "hash_autosave_1111".to_string(),
        created_at: Utc::now().to_rfc3339(),
        cloud_file_id: "cloud_autosave_01".to_string(),
    };

    let entry_save = CloudManifestEntry {
        game_name: "Witcher 3".to_string(),
        platform: "pc".to_string(),
        save_location_path: r"C:\Games\Witcher3\Saves".to_string(),
        relative_file_path: "save.dat".to_string(),
        version: 1,
        cloud_path: "witcher_3/v1/save.dat".to_string(),
        file_hash: "hash_save_2222".to_string(),
        created_at: Utc::now().to_rfc3339(),
        cloud_file_id: "cloud_save_02".to_string(),
    };

    let manifest = CloudCatalogManifest {
        schema_version: 1,
        updated_at: Utc::now().to_rfc3339(),
        entries: vec![entry_autosave, entry_save],
    };

    provider.save_manifest(&manifest).await.unwrap();

    let count = reconstruct_catalog_from_cloud(&db.conn, &provider)
        .await
        .unwrap();
    assert_eq!(count, 2);

    let mut stmt = db.conn.prepare("SELECT id, path FROM files").unwrap();
    let rows = stmt
        .query_map([], |r| {
            let id: String = r.get(0)?;
            let p: String = r.get(1)?;
            Ok((id, p))
        })
        .unwrap();
    let all_files: Vec<(String, String)> = rows.flatten().collect();

    assert_eq!(
        all_files.len(),
        2,
        "autosave.dat e save.dat não devem colidir em um único registro!"
    );

    let has_autosave = all_files.iter().any(|(_, p)| p.ends_with("autosave.dat"));
    let has_save = all_files
        .iter()
        .any(|(_, p)| p.ends_with("save.dat") && !p.ends_with("autosave.dat"));
    assert!(has_autosave);
    assert!(has_save);
}

// =============================================================================
// 6. AUDITORIA DO SQLITE: ATOMICIDADE DE UPDATE_MAPPING VIA SAVEPOINT
// =============================================================================

#[test]
fn test_atomic_update_mapping_rollback_on_failure() {
    let db = Database::open_in_memory().unwrap();
    let games_repo = GamesRepo::new(&db.conn);
    let locs_repo = SaveLocationsRepo::new(&db.conn);
    let files_repo = FilesRepo::new(&db.conn);

    let game_id = Uuid::new_v4();
    games_repo
        .insert(&Game {
            id: game_id,
            name: "Hollow Knight".to_string(),
            platform: Platform::Steam,
            platform_id: None,
            confidence: Confidence::High,
            created_at: Utc::now(),
        })
        .unwrap();

    let initial_path = PathBuf::from(r"C:\Users\Alice\AppData\LocalLow\Team Cherry\Hollow Knight");
    let loc1 = locs_repo
        .upsert(&SaveLocation {
            id: Uuid::new_v4(),
            game_id,
            path: initial_path.clone(),
            location_type: SaveLocationType::Directory,
            original_path: Some(initial_path.clone()),
            mapping_status: MappingStatus::NeedsUserMapping,
            mapping_source: MappingSource::Original,
            resolution_reason: None,
        })
        .unwrap();

    let other_path = PathBuf::from(r"C:\Users\Alice\Documents\OtherFolder");
    let _loc2 = locs_repo
        .upsert(&SaveLocation {
            id: Uuid::new_v4(),
            game_id,
            path: other_path.clone(),
            location_type: SaveLocationType::Directory,
            original_path: Some(other_path.clone()),
            mapping_status: MappingStatus::Resolved,
            mapping_source: MappingSource::Original,
            resolution_reason: None,
        })
        .unwrap();

    // Insere arquivo na loc1
    let file1 = files_repo
        .upsert(&FileEntry {
            id: Uuid::new_v4(),
            save_location_id: loc1.id,
            path: initial_path.join("user1.dat"),
            hash: "blake3_hash_test".to_string(),
            size_bytes: 1024,
            modified_at: Utc::now(),
        })
        .unwrap();

    // Tentativa de remapear loc1 para other_path -> DEVE FALHAR devido à UNIQUE constraint (game_id, path)
    let res = locs_repo.update_mapping(
        loc1.id,
        &other_path,
        MappingStatus::UserMapped,
        MappingSource::User,
        None,
    );

    assert!(
        res.is_err(),
        "Deve falhar por colisão de caminho único para o mesmo jogo"
    );

    // Verifica que loc1 e seu arquivo NÃO foram corrompidos ou alterados parcialmente
    let loc1_after = locs_repo.find_by_id(loc1.id).unwrap().unwrap();
    assert_eq!(loc1_after.path, initial_path);
    assert_eq!(loc1_after.mapping_status, MappingStatus::NeedsUserMapping);

    let file1_after = files_repo.find_by_id(file1.id).unwrap().unwrap();
    assert_eq!(file1_after.path, initial_path.join("user1.dat"));
}
