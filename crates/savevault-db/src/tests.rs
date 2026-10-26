#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use crate::repositories::{
        BackupsRepo, FilesRepo, GamesRepo, SaveLocationsRepo, SyncQueueRepo,
    };
    use crate::Database;
    use chrono::Utc;
    use rusqlite::params;
    use savevault_core::models::{
        Backup, Confidence, FileEntry, Game, Platform, SaveLocation, SaveLocationType,
        SyncOperation, SyncQueueItem, SyncStatus,
    };
    use std::path::PathBuf;
    use uuid::Uuid;

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

        let g1 = Game::new(
            "Celeste",
            Platform::Steam,
            Some("504230".into()),
            Confidence::High,
        );
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

        let game = Game::new(
            "Stardew Valley",
            Platform::Steam,
            Some("413150".into()),
            Confidence::High,
        );
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

        let platforms = [
            Platform::Steam,
            Platform::Gog,
            Platform::Dolphin,
            Platform::Pcsx2,
            Platform::Ppsspp,
            Platform::RetroArch,
            Platform::Generic,
        ];

        for (i, platform) in platforms.iter().enumerate() {
            let game = Game::new(
                format!("Jogo {}", i),
                platform.clone(),
                None,
                Confidence::Medium,
            );
            let id = game.id;
            repo.insert(&game).unwrap();
            let found = repo.find_by_id(id).unwrap().unwrap();
            assert_eq!(
                &found.platform, platform,
                "Platform {:?} não sobreviveu ao roundtrip",
                platform
            );
        }
    }

    #[test]
    fn games_confidence_roundtrip_all_variants() {
        let db = open_test_db();
        let repo = GamesRepo::new(&db.conn);

        for (i, confidence) in [Confidence::High, Confidence::Medium, Confidence::Low]
            .iter()
            .enumerate()
        {
            let game = Game::new(
                format!("Conf {}", i),
                Platform::Generic,
                None,
                confidence.clone(),
            );
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
        assert!(
            remaining.is_empty(),
            "ON DELETE CASCADE deve remover save locations"
        );
    }

    #[test]
    fn save_locations_find_by_id() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);

        let game = Game::new("FindById Test", Platform::Steam, None, Confidence::High);
        games_repo.insert(&game).unwrap();

        let loc = SaveLocation::new(game.id, r"C:\Saves\FindMe", SaveLocationType::Directory);
        let loc_id = loc.id;
        loc_repo.insert(&loc).unwrap();

        let found = loc_repo.find_by_id(loc_id).unwrap();
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.id, loc_id);
        assert_eq!(found.game_id, game.id);
        assert_eq!(found.path, PathBuf::from(r"C:\Saves\FindMe"));
        assert!(found.is_directory());

        let missing = loc_repo.find_by_id(Uuid::new_v4()).unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn save_locations_find_by_game_and_path() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);

        let game = Game::new("GameAndPath Test", Platform::Steam, None, Confidence::High);
        games_repo.insert(&game).unwrap();

        let path = PathBuf::from(r"C:\Users\User\AppData\Local\Game\save.dat");
        let loc = SaveLocation::new(game.id, path.clone(), SaveLocationType::File);
        loc_repo.insert(&loc).unwrap();

        let found = loc_repo.find_by_game_and_path(game.id, &path).unwrap();
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.id, loc.id);
        assert!(found.is_file());

        let nonexistent = loc_repo
            .find_by_game_and_path(game.id, &PathBuf::from(r"C:\Other\path"))
            .unwrap();
        assert!(nonexistent.is_none());
    }

    #[test]
    fn save_locations_supports_multiple_locations_per_game() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);

        let game = Game::new(
            "Multi-Loc Game",
            Platform::Steam,
            Some("12345".into()),
            Confidence::High,
        );
        let game_id = game.id;
        games_repo.insert(&game).unwrap();

        // Um jogo pode ter um diretório principal de saves e um arquivo de configuração/save separado
        let loc1 = SaveLocation::new(
            game_id,
            r"C:\AppData\Local\Game\Saves",
            SaveLocationType::Directory,
        );
        let loc2 = SaveLocation::new(
            game_id,
            r"C:\Documents\My Games\Game\profile.sav",
            SaveLocationType::File,
        );

        loc_repo.insert(&loc1).unwrap();
        loc_repo.insert(&loc2).unwrap();

        let locs = loc_repo.list_by_game(game_id).unwrap();
        assert_eq!(locs.len(), 2, "Game deve suportar múltiplos SaveLocations");

        let has_dir = locs.iter().any(|l| {
            l.is_directory() && l.path == std::path::Path::new(r"C:\AppData\Local\Game\Saves")
        });
        let has_file = locs.iter().any(|l| {
            l.is_file() && l.path == std::path::Path::new(r"C:\Documents\My Games\Game\profile.sav")
        });
        assert!(has_dir);
        assert!(has_file);
    }

    #[test]
    fn save_locations_unique_constraint_rejects_duplicate_path() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);

        let game = Game::new("Unique Test", Platform::Generic, None, Confidence::Medium);
        games_repo.insert(&game).unwrap();

        let loc1 = SaveLocation::new(game.id, r"C:\Saves\SamePath", SaveLocationType::Directory);
        loc_repo.insert(&loc1).unwrap();

        // Tentar inserir outro SaveLocation com o mesmo (game_id, path) deve falhar pelo índice único
        let loc2 = SaveLocation::new(game.id, r"C:\Saves\SamePath", SaveLocationType::Directory);
        let res = loc_repo.insert(&loc2);
        assert!(res.is_err(), "Índice único idx_save_locations_game_path deve rejeitar caminho duplicado para o mesmo jogo");

        // Tentar inserir com case diferente também deve falhar pelo índice COLLATE NOCASE
        let loc_case =
            SaveLocation::new(game.id, r"c:\saves\samepath", SaveLocationType::Directory);
        let res_case = loc_repo.insert(&loc_case);
        assert!(
            res_case.is_err(),
            "Índice único deve rejeitar caminho duplicado mesmo com casing diferente"
        );
    }

    #[test]
    fn save_locations_upsert_is_idempotent() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);

        let game = Game::new("Idempotent Test", Platform::Steam, None, Confidence::High);
        games_repo.insert(&game).unwrap();

        let loc = SaveLocation::new(game.id, r"C:\Games\Save", SaveLocationType::Directory);

        // Executar upsert 1 vez
        let saved1 = loc_repo.upsert(&loc).unwrap();
        assert_eq!(saved1.id, loc.id);

        // Executar upsert 10 vezes com a mesma localização lógica
        for _ in 0..10 {
            let res = loc_repo.upsert(&loc).unwrap();
            assert_eq!(
                res.id, loc.id,
                "ID original deve ser mantido em chamadas subsequentes"
            );
        }

        // Deve existir apenas 1 registro no banco
        let locs = loc_repo.list_by_game(game.id).unwrap();
        assert_eq!(
            locs.len(),
            1,
            "Persistência repetida não pode criar duplicatas"
        );
    }

    #[test]
    fn save_locations_upsert_preserves_existing_uuid() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);

        let game = Game::new("Preserve UUID Test", Platform::Gog, None, Confidence::High);
        games_repo.insert(&game).unwrap();

        let original_id = Uuid::new_v4();
        let loc1 = SaveLocation {
            id: original_id,
            game_id: game.id,
            path: PathBuf::from(r"C:\Games\GogGame\Save"),
            location_type: SaveLocationType::Directory,
        };
        let res1 = loc_repo.upsert(&loc1).unwrap();
        assert_eq!(res1.id, original_id);

        // Uma segunda descoberta gera um SaveLocation com novo UUID em memória
        let new_generated_id = Uuid::new_v4();
        assert_ne!(new_generated_id, original_id);

        let loc2 = SaveLocation {
            id: new_generated_id,
            game_id: game.id,
            path: PathBuf::from(r"c:\games\goggame\save\"),
            location_type: SaveLocationType::Directory,
        };

        // O upsert deve reconhecer a mesma identidade lógica e PRESERVAR o UUID do banco
        let res2 = loc_repo.upsert(&loc2).unwrap();
        assert_eq!(
            res2.id, original_id,
            "O UUID retornado pelo upsert DEVE ser o original gravado no banco"
        );

        let in_db = loc_repo.find_by_id(original_id).unwrap();
        assert!(
            in_db.is_some(),
            "Registro com o UUID original deve permanecer no banco"
        );

        let in_db_new = loc_repo.find_by_id(new_generated_id).unwrap();
        assert!(
            in_db_new.is_none(),
            "Novo UUID efêmero não deve ser gravado"
        );
    }

    #[test]
    fn save_locations_case_insensitivity_prevents_duplicates() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);

        let game = Game::new("Case Test", Platform::Generic, None, Confidence::Medium);
        games_repo.insert(&game).unwrap();

        let loc1 = SaveLocation::new(game.id, r"C:\Games\Save", SaveLocationType::Directory);
        let loc2 = SaveLocation::new(game.id, r"c:\games\save", SaveLocationType::Directory);

        loc_repo.upsert(&loc1).unwrap();
        loc_repo.upsert(&loc2).unwrap();

        let locs = loc_repo.list_by_game(game.id).unwrap();
        assert_eq!(
            locs.len(),
            1,
            "Variação de casing no Windows não deve produzir duas linhas"
        );
    }

    #[test]
    fn save_locations_separators_and_slashes_prevent_duplicates() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);

        let game = Game::new(
            "Separator Test",
            Platform::Generic,
            None,
            Confidence::Medium,
        );
        games_repo.insert(&game).unwrap();

        let loc1 = SaveLocation::new(game.id, r"C:\Games\Save", SaveLocationType::Directory);
        let loc2 = SaveLocation::new(game.id, "C:/Games/Save", SaveLocationType::Directory);
        let loc3 = SaveLocation::new(game.id, r"C:\Games\Save\", SaveLocationType::Directory);
        let loc4 = SaveLocation::new(game.id, r"C:\Games\\Save", SaveLocationType::Directory);

        loc_repo.upsert(&loc1).unwrap();
        loc_repo.upsert(&loc2).unwrap();
        loc_repo.upsert(&loc3).unwrap();
        loc_repo.upsert(&loc4).unwrap();

        let locs = loc_repo.list_by_game(game.id).unwrap();
        assert_eq!(
            locs.len(),
            1,
            "Variação de separadores e trailing slashes não pode duplicar registro"
        );
    }

    #[test]
    fn save_locations_multiple_locations_persisted_individually() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);

        let game = Game::new(
            "Multi Loc Test",
            Platform::Generic,
            None,
            Confidence::Medium,
        );
        games_repo.insert(&game).unwrap();

        let loc_a = SaveLocation::new(game.id, r"C:\SaveA", SaveLocationType::Directory);
        let loc_b = SaveLocation::new(game.id, r"C:\SaveB", SaveLocationType::File);
        let loc_c = SaveLocation::new(game.id, r"C:\SaveC", SaveLocationType::Directory);

        loc_repo.upsert(&loc_a).unwrap();
        loc_repo.upsert(&loc_b).unwrap();
        loc_repo.upsert(&loc_c).unwrap();

        let locs = loc_repo.list_by_game(game.id).unwrap();
        assert_eq!(
            locs.len(),
            3,
            "Três locais distintos para o mesmo jogo devem produzir 3 registros"
        );

        // Segunda execução completa
        loc_repo.upsert(&loc_a).unwrap();
        loc_repo.upsert(&loc_b).unwrap();
        loc_repo.upsert(&loc_c).unwrap();

        let locs_after = loc_repo.list_by_game(game.id).unwrap();
        assert_eq!(
            locs_after.len(),
            3,
            "Segunda execução deve continuar com 3 registros"
        );
    }

    #[test]
    fn save_locations_nonexistent_path_persisted_successfully() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);

        let game = Game::new(
            "NonExistent Path Test",
            Platform::Generic,
            None,
            Confidence::Medium,
        );
        games_repo.insert(&game).unwrap();

        let non_existing = PathBuf::from(r"C:\DefinitelyNonExistentGame12345\Save");
        assert!(
            !non_existing.exists(),
            "Pré-condição: caminho não existe no disco"
        );

        let loc = SaveLocation::new(game.id, non_existing.clone(), SaveLocationType::Directory);
        let saved = loc_repo.upsert(&loc).unwrap();

        let found = loc_repo
            .find_by_game_and_path(game.id, &non_existing)
            .unwrap();
        assert!(
            found.is_some(),
            "Caminho inexistente fisicamente deve poder ser persistido e consultado com sucesso"
        );
        assert_eq!(saved.id, found.unwrap().id);
    }

    #[test]
    fn save_locations_glob_persisted_with_semantics() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);

        let game = Game::new(
            "Glob Persist Test",
            Platform::Generic,
            None,
            Confidence::Medium,
        );
        games_repo.insert(&game).unwrap();

        let glob_path = PathBuf::from(r"C:\MyGame\*.sav");
        let loc = SaveLocation::new(game.id, glob_path.clone(), SaveLocationType::Glob);
        assert!(loc.is_glob());

        let saved = loc_repo.upsert(&loc).unwrap();
        assert_eq!(saved.location_type, SaveLocationType::Glob);
        assert!(saved.is_glob());

        let found = loc_repo.find_by_game_and_path(game.id, &glob_path).unwrap();
        assert!(found.is_some());
        let found = found.unwrap();
        assert!(found.is_glob());
        assert_eq!(found.location_type, SaveLocationType::Glob);
    }

    #[test]
    fn save_locations_corrupted_uuid_returns_error() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);

        let game = Game::new(
            "Corrupt UUID Test",
            Platform::Generic,
            None,
            Confidence::Low,
        );
        games_repo.insert(&game).unwrap();

        // Inserir manualmente linha com UUID corrompido
        db.conn.execute(
            "INSERT INTO save_locations (id, game_id, path, location_type) VALUES (?1, ?2, ?3, ?4)",
            params!["not-a-valid-uuid", game.id.to_string(), r"C:\Saves\Bad", "directory"],
        ).unwrap();

        let res = loc_repo.list_by_game(game.id);
        assert!(
            res.is_err(),
            "UUID corrompido deve propagar erro de conversão SQL em vez de fabricar UUID aleatório"
        );
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

        let game = Game::new(
            "HashUpdateTest",
            Platform::Generic,
            None,
            Confidence::Medium,
        );
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
            id: Uuid::new_v4(),
            game_id: game.id,
            path: PathBuf::from(r"C:\S1"),
            location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();
        let file_id = Uuid::new_v4();
        let entry = FileEntry {
            id: file_id,
            save_location_id: loc.id,
            path: PathBuf::from(r"C:\S1\s.dat"),
            hash: "h".into(),
            size_bytes: 1,
            modified_at: Utc::now(),
        };
        files_repo.upsert(&entry).unwrap();

        for v in 1..=3u32 {
            let b = Backup {
                id: Uuid::new_v4(),
                file_id,
                version: v,
                cloud_path: None,
                drive_file_id: None,
                sync_status: SyncStatus::Pending,
                created_at: Utc::now(),
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
            id: Uuid::new_v4(),
            game_id: game.id,
            path: PathBuf::from(r"C:\P1"),
            location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();
        let file_id = Uuid::new_v4();
        let entry = FileEntry {
            id: file_id,
            save_location_id: loc.id,
            path: PathBuf::from(r"C:\P1\s.dat"),
            hash: "h".into(),
            size_bytes: 1,
            modified_at: Utc::now(),
        };
        files_repo.upsert(&entry).unwrap();

        for v in 1..=10u32 {
            let b = Backup {
                id: Uuid::new_v4(),
                file_id,
                version: v,
                cloud_path: None,
                drive_file_id: None,
                sync_status: SyncStatus::Pending,
                created_at: Utc::now(),
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
    fn test_prune_caso_a_backup_sem_drive_file_id_removido_sem_delete_remoto() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);
        let files_repo = FilesRepo::new(&db.conn);
        let backups_repo = BackupsRepo::new(&db.conn);
        let queue_repo = SyncQueueRepo::new(&db.conn);

        let game = Game::new("PruneCaseA", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();
        let loc = SaveLocation {
            id: Uuid::new_v4(),
            game_id: game.id,
            path: PathBuf::from(r"C:\PruneA"),
            location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();
        let file_id = Uuid::new_v4();
        let entry = FileEntry {
            id: file_id,
            save_location_id: loc.id,
            path: PathBuf::from(r"C:\PruneA\s.dat"),
            hash: "hash".into(),
            size_bytes: 100,
            modified_at: Utc::now(),
        };
        files_repo.upsert(&entry).unwrap();

        // 4 backups sem drive_file_id
        let mut ids = Vec::new();
        for v in 1..=4u32 {
            let b = Backup {
                id: Uuid::new_v4(),
                file_id,
                version: v,
                cloud_path: None,
                drive_file_id: None,
                sync_status: SyncStatus::Pending,
                created_at: Utc::now(),
            };
            backups_repo.insert(&b).unwrap();
            ids.push(b.id);
        }

        // Pruning mantendo 2 versões (deve manter v4 e v3; remover v1 e v2)
        backups_repo.prune_old_versions(file_id, 2).unwrap();

        let remaining = backups_repo.list_by_file(file_id).unwrap();
        assert_eq!(
            remaining.len(),
            2,
            "Apenas as 2 mais recentes devem permanecer"
        );
        assert_eq!(remaining[0].version, 4);
        assert_eq!(remaining[1].version, 3);

        // v1 e v2 foram removidos do SQLite
        assert!(backups_repo.find_by_id(ids[0]).unwrap().is_none());
        assert!(backups_repo.find_by_id(ids[1]).unwrap().is_none());

        // Nenhum delete remoto enfileirado
        let queue = queue_repo.list_pending().unwrap();
        assert_eq!(
            queue.len(),
            0,
            "Nenhum delete remoto necessário para backups locais sem drive_file_id"
        );
    }

    #[test]
    fn test_prune_caso_b_backup_com_drive_file_id_preserva_referencia_e_enfileira_delete() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);
        let files_repo = FilesRepo::new(&db.conn);
        let backups_repo = BackupsRepo::new(&db.conn);
        let queue_repo = SyncQueueRepo::new(&db.conn);

        let game = Game::new("PruneCaseB", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();
        let loc = SaveLocation {
            id: Uuid::new_v4(),
            game_id: game.id,
            path: PathBuf::from(r"C:\PruneB"),
            location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();
        let file_id = Uuid::new_v4();
        let entry = FileEntry {
            id: file_id,
            save_location_id: loc.id,
            path: PathBuf::from(r"C:\PruneB\s.dat"),
            hash: "hash".into(),
            size_bytes: 100,
            modified_at: Utc::now(),
        };
        files_repo.upsert(&entry).unwrap();

        // v1: sincronizado com drive_file_id remoto
        let b1 = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 1,
            cloud_path: Some("cloud/v1.sav".into()),
            drive_file_id: Some("drive_file_id_alpha_999".into()),
            sync_status: SyncStatus::Synced,
            created_at: Utc::now(),
        };
        backups_repo.insert(&b1).unwrap();

        // v2: local sem cloud
        let b2 = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 2,
            cloud_path: None,
            drive_file_id: None,
            sync_status: SyncStatus::Pending,
            created_at: Utc::now(),
        };
        backups_repo.insert(&b2).unwrap();

        // v3: mais recente
        let b3 = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 3,
            cloud_path: Some("cloud/v3.sav".into()),
            drive_file_id: Some("drive_file_id_beta_888".into()),
            sync_status: SyncStatus::Synced,
            created_at: Utc::now(),
        };
        backups_repo.insert(&b3).unwrap();

        // Pruning mantendo apenas 1 versão mais recente (v3 mantido, v1 e v2 podados)
        backups_repo.prune_old_versions(file_id, 1).unwrap();

        // Invariante ARCH-01:
        // v2 (sem drive_file_id) foi deletado do SQLite
        assert!(backups_repo.find_by_id(b2.id).unwrap().is_none());

        // v1 (com drive_file_id) Teve sua referência remota PRESERVADA!
        let b1_persisted = backups_repo
            .find_by_id(b1.id)
            .unwrap()
            .expect("b1 deve ser preservado para deleção remota");
        assert_eq!(
            b1_persisted.drive_file_id,
            Some("drive_file_id_alpha_999".to_string())
        );

        // Fila possui exatamente a operação Delete com o backup_id correto
        let queue = queue_repo.list_pending().unwrap();
        assert_eq!(
            queue.len(),
            1,
            "Operação de delete remoto deve estar enfileirada"
        );
        assert_eq!(queue[0].backup_id, b1.id);
        assert_eq!(queue[0].operation, SyncOperation::Delete);
        assert_eq!(queue[0].status, SyncStatus::Pending);
    }

    #[test]
    fn test_prune_caso_c_pruning_em_modo_offline_mantem_consistencia_e_pendencia() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);
        let files_repo = FilesRepo::new(&db.conn);
        let backups_repo = BackupsRepo::new(&db.conn);
        let queue_repo = SyncQueueRepo::new(&db.conn);

        let game = Game::new("PruneCaseC", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();
        let loc = SaveLocation {
            id: Uuid::new_v4(),
            game_id: game.id,
            path: PathBuf::from(r"C:\PruneC"),
            location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();
        let file_id = Uuid::new_v4();
        let entry = FileEntry {
            id: file_id,
            save_location_id: loc.id,
            path: PathBuf::from(r"C:\PruneC\s.dat"),
            hash: "hash".into(),
            size_bytes: 100,
            modified_at: Utc::now(),
        };
        files_repo.upsert(&entry).unwrap();

        let b1 = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 1,
            cloud_path: Some("cloud/offline.sav".into()),
            drive_file_id: Some("drive_offline_token_123".into()),
            sync_status: SyncStatus::Synced,
            created_at: Utc::now(),
        };
        backups_repo.insert(&b1).unwrap();

        let b2 = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 2,
            cloud_path: None,
            drive_file_id: None,
            sync_status: SyncStatus::Pending,
            created_at: Utc::now(),
        };
        backups_repo.insert(&b2).unwrap();

        // Pruning executado offline (nenhuma conexão ou provider ativo)
        backups_repo.prune_old_versions(file_id, 1).unwrap();

        // Aplicação continua consistente:
        // 1. Integridade de chave estrangeira válida (PRAGMA foreign_keys = ON)
        let queue_item = queue_repo.list_pending().unwrap();
        assert_eq!(queue_item.len(), 1);
        assert_eq!(queue_item[0].backup_id, b1.id);

        // 2. Não perdeu a identidade remota
        let b1_check = backups_repo.find_by_id(b1.id).unwrap().unwrap();
        assert_eq!(
            b1_check.drive_file_id.as_deref(),
            Some("drive_offline_token_123")
        );

        // 3. Chamadas subsequentes de prune são idempotentes e não duplicam itens na fila
        backups_repo.prune_old_versions(file_id, 1).unwrap();
        let queue_after = queue_repo.list_pending().unwrap();
        assert_eq!(
            queue_after.len(),
            1,
            "Idempotência: não deve duplicar item de deleção"
        );
    }

    #[test]
    fn test_prune_caso_d_atomicidade_transacional_e_resistencia_a_falha() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);
        let files_repo = FilesRepo::new(&db.conn);
        let backups_repo = BackupsRepo::new(&db.conn);

        let game = Game::new("PruneCaseD", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();
        let loc = SaveLocation {
            id: Uuid::new_v4(),
            game_id: game.id,
            path: PathBuf::from(r"C:\PruneD"),
            location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();
        let file_id = Uuid::new_v4();
        let entry = FileEntry {
            id: file_id,
            save_location_id: loc.id,
            path: PathBuf::from(r"C:\PruneD\s.dat"),
            hash: "hash".into(),
            size_bytes: 100,
            modified_at: Utc::now(),
        };
        files_repo.upsert(&entry).unwrap();

        let b1 = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 1,
            cloud_path: Some("cloud/crash_test.sav".into()),
            drive_file_id: Some("drive_crash_protection_id".into()),
            sync_status: SyncStatus::Synced,
            created_at: Utc::now(),
        };
        backups_repo.insert(&b1).unwrap();

        // Simula falha abortando se sync_queue sofrer INSERT
        db.conn
            .execute_batch(
                "CREATE TRIGGER abort_sync_queue BEFORE INSERT ON sync_queue \
             BEGIN \
                 SELECT RAISE(FAIL, 'Simulated crash before local removal'); \
             END;",
            )
            .unwrap();

        // Execução do prune deve falhar atomicamente
        let res = backups_repo.prune_old_versions(file_id, 0);
        assert!(res.is_err(), "Falha simulada deve retornar Err");

        // Remove trigger para verificar consistência
        db.conn
            .execute("DROP TRIGGER abort_sync_queue", [])
            .unwrap();

        // O backup original permanece intocado: a referência remota NÃO foi silenciosamente perdida
        let b1_recovered = backups_repo.find_by_id(b1.id).unwrap().unwrap();
        assert_eq!(
            b1_recovered.drive_file_id.as_deref(),
            Some("drive_crash_protection_id")
        );
    }

    #[test]
    fn test_backups_clear_cloud_metadata() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);
        let files_repo = FilesRepo::new(&db.conn);
        let backups_repo = BackupsRepo::new(&db.conn);

        let game = Game::new("ClearMeta", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();
        let loc = SaveLocation {
            id: Uuid::new_v4(),
            game_id: game.id,
            path: PathBuf::from(r"C:\ClearMeta"),
            location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();
        let file_id = Uuid::new_v4();
        let entry = FileEntry {
            id: file_id,
            save_location_id: loc.id,
            path: PathBuf::from(r"C:\ClearMeta\s.dat"),
            hash: "hash".into(),
            size_bytes: 100,
            modified_at: Utc::now(),
        };
        files_repo.upsert(&entry).unwrap();

        let backup = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 1,
            cloud_path: Some("cloud/active.sav".into()),
            drive_file_id: Some("drive_id_active".into()),
            sync_status: SyncStatus::Synced,
            created_at: Utc::now(),
        };
        backups_repo.insert(&backup).unwrap();

        // Limpa metadados de nuvem mantendo registro local
        backups_repo
            .clear_cloud_metadata(backup.id, &SyncStatus::PendingOffline)
            .unwrap();

        let updated = backups_repo.find_by_id(backup.id).unwrap().unwrap();
        assert!(updated.cloud_path.is_none());
        assert!(updated.drive_file_id.is_none());
        assert_eq!(updated.sync_status, SyncStatus::PendingOffline);
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
            id: Uuid::new_v4(),
            game_id: game.id,
            path: PathBuf::from(r"C:\ST"),
            location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();
        let file_id = Uuid::new_v4();
        let entry = FileEntry {
            id: file_id,
            save_location_id: loc.id,
            path: PathBuf::from(r"C:\ST\s.dat"),
            hash: "h".into(),
            size_bytes: 1,
            modified_at: Utc::now(),
        };
        files_repo.upsert(&entry).unwrap();

        let backup = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 1,
            cloud_path: None,
            drive_file_id: None,
            sync_status: SyncStatus::Pending,
            created_at: Utc::now(),
        };
        let backup_id = backup.id;
        backups_repo.insert(&backup).unwrap();

        backups_repo
            .update_sync_status(backup_id, &SyncStatus::Synced)
            .unwrap();
        let updated = backups_repo.list_by_file(file_id).unwrap();
        assert_eq!(updated[0].sync_status, SyncStatus::Synced);
    }

    #[test]
    fn backups_find_by_id() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);
        let files_repo = FilesRepo::new(&db.conn);
        let backups_repo = BackupsRepo::new(&db.conn);

        let game = Game::new("FindBackupTest", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();
        let loc = SaveLocation {
            id: Uuid::new_v4(),
            game_id: game.id,
            path: PathBuf::from(r"C:\FB"),
            location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();
        let file_id = Uuid::new_v4();
        let entry = FileEntry {
            id: file_id,
            save_location_id: loc.id,
            path: PathBuf::from(r"C:\FB\s.dat"),
            hash: "h".into(),
            size_bytes: 1,
            modified_at: Utc::now(),
        };
        files_repo.upsert(&entry).unwrap();

        let backup = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 1,
            cloud_path: Some("cloud/path/v1".into()),
            drive_file_id: Some("drive_id_abc123".into()),
            sync_status: SyncStatus::Pending,
            created_at: Utc::now(),
        };
        backups_repo.insert(&backup).unwrap();

        let found = backups_repo.find_by_id(backup.id).unwrap();
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.id, backup.id);
        assert_eq!(found.file_id, file_id);
        assert_eq!(found.version, 1);
        assert_eq!(found.cloud_path, Some("cloud/path/v1".into()));
        assert_eq!(found.drive_file_id, Some("drive_id_abc123".into()));

        let missing = backups_repo.find_by_id(Uuid::new_v4()).unwrap();
        assert!(missing.is_none());
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
            id: Uuid::new_v4(),
            game_id: game.id,
            path: PathBuf::from(r"C:\Q1"),
            location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();
        let file_id = Uuid::new_v4();
        let entry = FileEntry {
            id: file_id,
            save_location_id: loc.id,
            path: PathBuf::from(r"C:\Q1\s.dat"),
            hash: "h".into(),
            size_bytes: 1,
            modified_at: Utc::now(),
        };
        files_repo.upsert(&entry).unwrap();
        let backup = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 1,
            cloud_path: None,
            drive_file_id: None,
            sync_status: SyncStatus::Pending,
            created_at: Utc::now(),
        };
        let backup_id = backup.id;
        backups_repo.insert(&backup).unwrap();

        let item = SyncQueueItem {
            id: Uuid::new_v4(),
            backup_id,
            operation: SyncOperation::Upload,
            status: SyncStatus::Pending,
            attempts: 0,
            created_at: Utc::now(),
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
            id: Uuid::new_v4(),
            game_id: game.id,
            path: PathBuf::from(r"C:\DQ"),
            location_type: SaveLocationType::Directory,
        };
        loc_repo.insert(&loc).unwrap();
        let file_id = Uuid::new_v4();
        files_repo
            .upsert(&FileEntry {
                id: file_id,
                save_location_id: loc.id,
                path: PathBuf::from(r"C:\DQ\s.dat"),
                hash: "h".into(),
                size_bytes: 1,
                modified_at: Utc::now(),
            })
            .unwrap();
        let backup = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 1,
            cloud_path: None,
            drive_file_id: None,
            sync_status: SyncStatus::Pending,
            created_at: Utc::now(),
        };
        backups_repo.insert(&backup).unwrap();

        let item_id = Uuid::new_v4();
        queue_repo
            .enqueue(&SyncQueueItem {
                id: item_id,
                backup_id: backup.id,
                operation: SyncOperation::Upload,
                status: SyncStatus::Pending,
                attempts: 0,
                created_at: Utc::now(),
            })
            .unwrap();

        assert_eq!(queue_repo.list_pending().unwrap().len(), 1);
        queue_repo.delete(item_id).unwrap();
        assert_eq!(queue_repo.list_pending().unwrap().len(), 0);
    }

    #[test]
    fn save_locations_list_all_returns_all_locations() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);
        let loc_repo = SaveLocationsRepo::new(&db.conn);

        let g1 = Game::new("Game 1", Platform::Steam, None, Confidence::High);
        let g2 = Game::new("Game 2", Platform::Gog, None, Confidence::High);
        games_repo.insert(&g1).unwrap();
        games_repo.insert(&g2).unwrap();

        let l1 = SaveLocation::new(g1.id, r"C:\Saves\G1", SaveLocationType::Directory);
        let l2 = SaveLocation::new(g2.id, r"C:\Saves\G2", SaveLocationType::Directory);
        loc_repo.insert(&l1).unwrap();
        loc_repo.insert(&l2).unwrap();

        let all = loc_repo.list_all().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn games_find_by_name_and_platform_case_insensitive() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);

        let g = Game::new(
            "Super Mario World",
            Platform::RetroArch,
            None,
            Confidence::High,
        );
        games_repo.insert(&g).unwrap();

        // Exact match
        let found = games_repo
            .find_by_name_and_platform("Super Mario World", &Platform::RetroArch)
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, g.id);

        // Case-insensitive match
        let found_lower = games_repo
            .find_by_name_and_platform("super mario world", &Platform::RetroArch)
            .unwrap();
        assert!(found_lower.is_some());
        assert_eq!(found_lower.unwrap().id, g.id);

        // Different platform returns None
        let found_diff_platform = games_repo
            .find_by_name_and_platform("Super Mario World", &Platform::Dolphin)
            .unwrap();
        assert!(found_diff_platform.is_none());
    }

    #[test]
    fn games_unique_name_platform_constraint_prevents_duplicate() {
        let db = open_test_db();
        let games_repo = GamesRepo::new(&db.conn);

        let g1 = Game::new("Unique Game", Platform::Generic, None, Confidence::High);
        let g2 = Game::new("unique game", Platform::Generic, None, Confidence::Medium);

        games_repo.insert(&g1).unwrap();
        // Devido ao INSERT OR IGNORE e ao índice único, a duplicata é ignorada e não duplica no banco
        games_repo.insert(&g2).unwrap();

        let all = games_repo.list_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, g1.id);
    }
}
