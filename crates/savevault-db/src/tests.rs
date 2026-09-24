#[cfg(test)]
mod tests {
    use crate::Database;
    use crate::repositories::{GamesRepo, SaveLocationsRepo, FilesRepo, BackupsRepo, SyncQueueRepo};
    use savevault_core::models::{
        Confidence, Game, Platform, SaveLocation, SaveLocationType,
        FileEntry, Backup, SyncQueueItem, SyncOperation, SyncStatus,
    };
    use uuid::Uuid;
    use chrono::Utc;
    use std::path::PathBuf;

    fn open_test_db() -> Database {
        Database::open_in_memory().expect("banco em memória deve abrir")
    }

    // ------------------------------------------------------------------
    // Migrations
    // ------------------------------------------------------------------
    #[test]
    fn migrations_run_idempotently() {
        // Abrir o mesmo banco duas vezes — migrações não devem falhar
        let db = open_test_db();
        // Rodar migrations novamente diretamente
        crate::migrations::run(&db.conn).expect("segunda execução deve ser idempotente");
    }

    // ------------------------------------------------------------------
    // GamesRepo
    // ------------------------------------------------------------------
    #[test]
    fn games_insert_and_list() {
        let db = open_test_db();
        let repo = GamesRepo::new(&db.conn);

        let g1 = Game::new("Celeste", Platform::Steam, Some("504230".into()), Confidence::High);
        let g2 = Game::new("Hollow Knight", Platform::Gog, None, Confidence::Medium);

        repo.insert(&g1).unwrap();
        repo.insert(&g2).unwrap();

        let all = repo.list_all().unwrap();
        assert_eq!(all.len(), 2);

        // list_all retorna em ordem alfabética
        assert_eq!(all[0].name, "Celeste");
        assert_eq!(all[1].name, "Hollow Knight");
    }

    #[test]
    fn games_find_by_id() {
        let db = open_test_db();
        let repo = GamesRepo::new(&db.conn);

        let game = Game::new("Stardew Valley", Platform::Steam, Some("413150".into()), Confidence::High);
        let id = game.id;
        repo.insert(&game).unwrap();

        let found = repo.find_by_id(id).unwrap();
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.name, "Stardew Valley");
        assert_eq!(found.platform_id, Some("413150".into()));
    }

    #[test]
    fn games_find_by_id_missing_returns_none() {
        let db = open_test_db();
        let repo = GamesRepo::new(&db.conn);
        let result = repo.find_by_id(Uuid::new_v4()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn games_delete_removes_entry() {
        let db = open_test_db();
        let repo = GamesRepo::new(&db.conn);

        let game = Game::new("To Delete", Platform::Generic, None, Confidence::Low);
        let id = game.id;
        repo.insert(&game).unwrap();

        assert_eq!(repo.list_all().unwrap().len(), 1);
        repo.delete(id).unwrap();
        assert_eq!(repo.list_all().unwrap().len(), 0);
    }

    #[test]
    fn games_platform_roundtrip_all_variants() {
        let db = open_test_db();
        let repo = GamesRepo::new(&db.conn);

        let platforms = vec![
            Platform::Steam, Platform::Gog, Platform::Dolphin,
            Platform::Pcsx2, Platform::Ppsspp, Platform::RetroArch, Platform::Generic,
        ];

        for (i, platform) in platforms.iter().enumerate() {
            let game = Game::new(format!("Jogo {}", i), platform.clone(), None, Confidence::Medium);
            let id = game.id;
            repo.insert(&game).unwrap();
            let found = repo.find_by_id(id).unwrap().unwrap();
            assert_eq!(&found.platform, platform, "Platform {:?} não sobreviveu ao roundtrip", platform);
        }
    }

    #[test]
    fn games_confidence_roundtrip_all_variants() {
        let db = open_test_db();
        let repo = GamesRepo::new(&db.conn);

        for (i, confidence) in [Confidence::High, Confidence::Medium, Confidence::Low].iter().enumerate() {
            let game = Game::new(format!("Conf {}", i), Platform::Generic, None, confidence.clone());
            let id = game.id;
            repo.insert(&game).unwrap();
            let found = repo.find_by_id(id).unwrap().unwrap();
            assert_eq!(&found.confidence, confidence);
        }
    }

    // ------------------------------------------------------------------
    // SaveLocationsRepo
    // ------------------------------------------------------------------
    #[test]
    fn save_locations_insert_and_list_by_game() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);

        let game = Game::new("Celeste", Platform::Steam, None, Confidence::High);
        let game_id = game.id;
        games_repo.insert(&game).unwrap();

        let loc = SaveLocation {
            id: Uuid::new_v4(),
            game_id,
            path: PathBuf::from(r"C:\Users\pedro\AppData\Local\Celeste\Saves"),
            location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();

        let found = loc_repo.list_by_game(game_id).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, loc.path);
    }

    #[test]
    fn save_locations_cascade_delete_with_game() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);

        let game = Game::new("Cascade Test", Platform::Generic, None, Confidence::Low);
        let game_id = game.id;
        games_repo.insert(&game).unwrap();

        let loc = SaveLocation {
            id: Uuid::new_v4(),
            game_id,
            path: PathBuf::from(r"C:\Saves\CascadeTest"),
            location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();

        // Ao deletar o jogo, SaveLocation deve ser removida por CASCADE
        games_repo.delete(game_id).unwrap();
        let remaining = loc_repo.list_by_game(game_id).unwrap();
        assert!(remaining.is_empty(), "ON DELETE CASCADE deve remover save locations");
    }

    // ------------------------------------------------------------------
    // FilesRepo
    // ------------------------------------------------------------------
    #[test]
    fn files_upsert_and_find_by_path() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);
        let files_repo = FilesRepo::new(&db.conn);

        let game = Game::new("FilesTest", Platform::Steam, None, Confidence::High);
        games_repo.insert(&game).unwrap();

        let loc = SaveLocation {
            id: Uuid::new_v4(),
            game_id: game.id,
            path: PathBuf::from(r"C:\Saves"),
            location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();

        let path = PathBuf::from(r"C:\Saves\save.dat");
        let entry = FileEntry {
            id: Uuid::new_v4(),
            save_location_id: loc.id,
            path: path.clone(),
            hash: "abc123".into(),
            size_bytes: 1024,
            modified_at: Utc::now(),
        };
        files_repo.upsert(&entry).unwrap();

        let found = files_repo.find_by_path(&path).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().hash, "abc123");
    }

    #[test]
    fn files_upsert_updates_hash() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);
        let files_repo = FilesRepo::new(&db.conn);

        let game = Game::new("HashUpdateTest", Platform::Generic, None, Confidence::Medium);
        games_repo.insert(&game).unwrap();
        let loc = SaveLocation {
            id: Uuid::new_v4(),
            game_id: game.id,
            path: PathBuf::from(r"C:\Saves2"),
            location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();

        let file_id = Uuid::new_v4();
        let path = PathBuf::from(r"C:\Saves2\save.dat");

        let entry_v1 = FileEntry {
            id: file_id,
            save_location_id: loc.id,
            path: path.clone(),
            hash: "hash_v1".into(),
            size_bytes: 100,
            modified_at: Utc::now(),
        };
        files_repo.upsert(&entry_v1).unwrap();

        let entry_v2 = FileEntry {
            id: file_id,
            save_location_id: loc.id,
            path: path.clone(),
            hash: "hash_v2".into(),
            size_bytes: 200,
            modified_at: Utc::now(),
        };
        files_repo.upsert(&entry_v2).unwrap();

        let found = files_repo.find_by_path(&path).unwrap().unwrap();
        assert_eq!(found.hash, "hash_v2", "Hash deve ser atualizado no upsert");
        assert_eq!(found.size_bytes, 200);
    }

    // ------------------------------------------------------------------
    // BackupsRepo
    // ------------------------------------------------------------------
    #[test]
    fn backups_insert_and_list_by_file_descending() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);
        let files_repo = FilesRepo::new(&db.conn);
        let backups_repo = BackupsRepo::new(&db.conn);

        let game = Game::new("BackupTest", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();
        let loc = SaveLocation {
            id: Uuid::new_v4(), game_id: game.id,
            path: PathBuf::from(r"C:\S1"), location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();
        let file_id = Uuid::new_v4();
        let entry = FileEntry {
            id: file_id, save_location_id: loc.id,
            path: PathBuf::from(r"C:\S1\s.dat"), hash: "h".into(),
            size_bytes: 1, modified_at: Utc::now(),
        };
        files_repo.upsert(&entry).unwrap();

        for v in 1..=3u32 {
            let b = Backup {
                id: Uuid::new_v4(), file_id, version: v,
                cloud_path: None, sync_status: SyncStatus::Pending, created_at: Utc::now(),
            };
            backups_repo.insert(&b).unwrap();
        }

        let backups = backups_repo.list_by_file(file_id).unwrap();
        assert_eq!(backups.len(), 3);
        // Deve vir em ordem decrescente de versão
        assert_eq!(backups[0].version, 3);
        assert_eq!(backups[1].version, 2);
        assert_eq!(backups[2].version, 1);
    }

    #[test]
    fn backups_prune_keeps_only_n_most_recent() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);
        let files_repo = FilesRepo::new(&db.conn);
        let backups_repo = BackupsRepo::new(&db.conn);

        let game = Game::new("PruneTest", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();
        let loc = SaveLocation {
            id: Uuid::new_v4(), game_id: game.id,
            path: PathBuf::from(r"C:\P1"), location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();
        let file_id = Uuid::new_v4();
        let entry = FileEntry {
            id: file_id, save_location_id: loc.id,
            path: PathBuf::from(r"C:\P1\s.dat"), hash: "h".into(),
            size_bytes: 1, modified_at: Utc::now(),
        };
        files_repo.upsert(&entry).unwrap();

        for v in 1..=10u32 {
            let b = Backup {
                id: Uuid::new_v4(), file_id, version: v,
                cloud_path: None, sync_status: SyncStatus::Pending, created_at: Utc::now(),
            };
            backups_repo.insert(&b).unwrap();
        }

        backups_repo.prune_old_versions(file_id, 3).unwrap();
        let remaining = backups_repo.list_by_file(file_id).unwrap();
        assert_eq!(remaining.len(), 3, "Deve manter apenas 3 versões");
        // Deve ter mantido as mais recentes (versões 8, 9, 10)
        let versions: Vec<u32> = remaining.iter().map(|b| b.version).collect();
        assert!(versions.contains(&10));
        assert!(versions.contains(&9));
        assert!(versions.contains(&8));
    }

    #[test]
    fn backups_update_sync_status() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);
        let files_repo = FilesRepo::new(&db.conn);
        let backups_repo = BackupsRepo::new(&db.conn);

        let game = Game::new("StatusTest", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();
        let loc = SaveLocation {
            id: Uuid::new_v4(), game_id: game.id,
            path: PathBuf::from(r"C:\ST"), location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();
        let file_id = Uuid::new_v4();
        let entry = FileEntry {
            id: file_id, save_location_id: loc.id,
            path: PathBuf::from(r"C:\ST\s.dat"), hash: "h".into(),
            size_bytes: 1, modified_at: Utc::now(),
        };
        files_repo.upsert(&entry).unwrap();

        let backup = Backup {
            id: Uuid::new_v4(), file_id, version: 1,
            cloud_path: None, sync_status: SyncStatus::Pending, created_at: Utc::now(),
        };
        let backup_id = backup.id;
        backups_repo.insert(&backup).unwrap();

        backups_repo.update_sync_status(backup_id, &SyncStatus::Synced).unwrap();
        let updated = backups_repo.list_by_file(file_id).unwrap();
        assert_eq!(updated[0].sync_status, SyncStatus::Synced);
    }

    // ------------------------------------------------------------------
    // SyncQueueRepo
    // ------------------------------------------------------------------
    #[test]
    fn sync_queue_enqueue_and_list_pending() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);
        let files_repo = FilesRepo::new(&db.conn);
        let backups_repo = BackupsRepo::new(&db.conn);
        let queue_repo = SyncQueueRepo::new(&db.conn);

        let game = Game::new("QueueTest", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();
        let loc = SaveLocation {
            id: Uuid::new_v4(), game_id: game.id,
            path: PathBuf::from(r"C:\Q1"), location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();
        let file_id = Uuid::new_v4();
        let entry = FileEntry {
            id: file_id, save_location_id: loc.id,
            path: PathBuf::from(r"C:\Q1\s.dat"), hash: "h".into(),
            size_bytes: 1, modified_at: Utc::now(),
        };
        files_repo.upsert(&entry).unwrap();
        let backup = Backup {
            id: Uuid::new_v4(), file_id, version: 1,
            cloud_path: None, sync_status: SyncStatus::Pending, created_at: Utc::now(),
        };
        let backup_id = backup.id;
        backups_repo.insert(&backup).unwrap();

        let item = SyncQueueItem {
            id: Uuid::new_v4(), backup_id,
            operation: SyncOperation::Upload,
            status: SyncStatus::Pending,
            attempts: 0, created_at: Utc::now(),
        };
        queue_repo.enqueue(&item).unwrap();

        let pending = queue_repo.list_pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].operation, SyncOperation::Upload);
        assert_eq!(pending[0].attempts, 0);
    }

    #[test]
    fn sync_queue_delete_removes_item() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);
        let files_repo = FilesRepo::new(&db.conn);
        let backups_repo = BackupsRepo::new(&db.conn);
        let queue_repo = SyncQueueRepo::new(&db.conn);

        let game = Game::new("DelQueueTest", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();
        let loc = SaveLocation {
            id: Uuid::new_v4(), game_id: game.id,
            path: PathBuf::from(r"C:\DQ"), location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();
        let file_id = Uuid::new_v4();
        files_repo.upsert(&FileEntry {
            id: file_id, save_location_id: loc.id,
            path: PathBuf::from(r"C:\DQ\s.dat"), hash: "h".into(),
            size_bytes: 1, modified_at: Utc::now(),
        }).unwrap();
        let backup = Backup {
            id: Uuid::new_v4(), file_id, version: 1,
            cloud_path: None, sync_status: SyncStatus::Pending, created_at: Utc::now(),
        };
        backups_repo.insert(&backup).unwrap();

        let item_id = Uuid::new_v4();
        queue_repo.enqueue(&SyncQueueItem {
            id: item_id, backup_id: backup.id,
            operation: SyncOperation::Upload,
            status: SyncStatus::Pending,
            attempts: 0, created_at: Utc::now(),
        }).unwrap();

        assert_eq!(queue_repo.list_pending().unwrap().len(), 1);
        queue_repo.delete(item_id).unwrap();
        assert_eq!(queue_repo.list_pending().unwrap().len(), 0);
    }
}
