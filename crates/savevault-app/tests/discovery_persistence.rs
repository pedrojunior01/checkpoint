//! Testes de integração da Etapa 2B:
//! Game Discovery → Save Discovery → Canonicalização → Persistência SQLite.

use savevault_app::orchestrator::discover_and_persist_saves;
use savevault_core::models::{Confidence, Game, Platform, SaveLocationType};
use savevault_db::repositories::SaveLocationsRepo;
use savevault_db::Database;
use savevault_discovery::ludusavi::manifest::LudusaviManifest;
use savevault_discovery::ludusavi::resolver::PathResolver;
use savevault_discovery::LudusaviSaveDetector;
use std::path::{Path, PathBuf};

fn open_test_db() -> Database {
    Database::open_in_memory().expect("Falha ao abrir banco em memória")
}

#[test]
fn test_integration_discovery_pipeline_idempotency() {
    let db = open_test_db();
    let yaml = r#"
Stardew Valley:
  steam:
    id: 413150
  files:
    <winAppData>/StardewValley/Saves:
      tags: [save]
      when:
        - os: windows
"#;
    let manifest = LudusaviManifest::from_yaml_str(yaml).unwrap();
    let detector = LudusaviSaveDetector::new(
        manifest,
        PathResolver::empty().with_variable("<winAppData>", r"C:\Users\TestUser\AppData\Roaming"),
    );

    let game = Game::new(
        "Stardew Valley",
        Platform::Steam,
        Some("413150".into()),
        Confidence::High,
    );
    savevault_db::repositories::GamesRepo::new(&db.conn)
        .insert(&game)
        .unwrap();

    // 1ª Execução do pipeline
    let report1 =
        discover_and_persist_saves(&db.conn, std::slice::from_ref(&game), &detector).unwrap();
    assert_eq!(report1.games_processed, 1);
    assert_eq!(report1.persisted_locations.len(), 1);
    assert_eq!(report1.conflicts.len(), 0);

    let loc_repo = SaveLocationsRepo::new(&db.conn);
    let in_db = loc_repo.list_by_game(game.id).unwrap();
    assert_eq!(in_db.len(), 1);
    let original_uuid = in_db[0].id;
    assert_eq!(
        in_db[0].path,
        PathBuf::from(r"C:\Users\TestUser\AppData\Roaming\StardewValley\Saves")
    );

    // 2ª, 3ª e 4ª Execuções consecutivas
    for _ in 0..3 {
        let report =
            discover_and_persist_saves(&db.conn, std::slice::from_ref(&game), &detector).unwrap();
        assert_eq!(report.persisted_locations.len(), 1);
        assert_eq!(
            report.persisted_locations[0].id, original_uuid,
            "UUID original deve ser preservado"
        );
    }

    // Continua exatamente com 1 registro no banco
    let in_db_after = loc_repo.list_by_game(game.id).unwrap();
    assert_eq!(
        in_db_after.len(),
        1,
        "Idempotência garantida: persistência múltipla não cria duplicatas"
    );
    assert_eq!(in_db_after[0].id, original_uuid);
}

#[test]
fn test_integration_pipeline_multiple_locations_and_nonexistent_paths() {
    let db = open_test_db();
    let yaml = r#"
Cyberpunk 2077:
  gog:
    id: 1423049311
  files:
    <winSavedGames>/CD Projekt Red/Cyberpunk 2077:
      tags: [save]
      when:
        - os: windows
    <winAppData>/Cyberpunk 2077/profiles:
      tags: [save]
      when:
        - os: windows
"#;
    let manifest = LudusaviManifest::from_yaml_str(yaml).unwrap();
    let detector = LudusaviSaveDetector::new(
        manifest,
        PathResolver::empty()
            .with_variable("<winSavedGames>", r"C:\Users\User\Saved Games")
            .with_variable("<winAppData>", r"C:\Users\User\AppData\Local"),
    );

    let game = Game::new(
        "Cyberpunk 2077",
        Platform::Gog,
        Some("1423049311".into()),
        Confidence::High,
    );
    savevault_db::repositories::GamesRepo::new(&db.conn)
        .insert(&game)
        .unwrap();

    let report =
        discover_and_persist_saves(&db.conn, std::slice::from_ref(&game), &detector).unwrap();
    assert_eq!(
        report.persisted_locations.len(),
        2,
        "Dois locais de save distintos devem ser persistidos"
    );

    let loc_repo = SaveLocationsRepo::new(&db.conn);
    let in_db = loc_repo.list_by_game(game.id).unwrap();
    assert_eq!(in_db.len(), 2);

    // Reexecutar pipeline não duplica nenhum dos 2 locais
    let report2 =
        discover_and_persist_saves(&db.conn, std::slice::from_ref(&game), &detector).unwrap();
    assert_eq!(report2.persisted_locations.len(), 2);
    let in_db2 = loc_repo.list_by_game(game.id).unwrap();
    assert_eq!(in_db2.len(), 2);
}

#[test]
fn test_integration_pipeline_globs_are_persisted_as_globs_without_fs_access() {
    let db = open_test_db();
    let yaml = r#"
OldSchoolGame:
  files:
    <winAppData>/OldSchool/*.sav:
      tags: [save]
      when:
        - os: windows
"#;
    let manifest = LudusaviManifest::from_yaml_str(yaml).unwrap();
    let detector = LudusaviSaveDetector::new(
        manifest,
        PathResolver::empty().with_variable("<winAppData>", r"C:\AppData"),
    );

    let game = Game::new("OldSchoolGame", Platform::Generic, None, Confidence::Medium);
    savevault_db::repositories::GamesRepo::new(&db.conn)
        .insert(&game)
        .unwrap();

    let report =
        discover_and_persist_saves(&db.conn, std::slice::from_ref(&game), &detector).unwrap();
    assert_eq!(report.persisted_locations.len(), 1);

    let saved = &report.persisted_locations[0];
    assert_eq!(saved.location_type, SaveLocationType::Glob);
    assert!(saved.is_glob());
    assert_eq!(saved.path, PathBuf::from(r"C:\AppData\OldSchool\*.sav"));

    // Consulta no banco confirma persistência da semântica de glob
    let loc_repo = SaveLocationsRepo::new(&db.conn);
    let in_db = loc_repo
        .find_by_game_and_path(game.id, Path::new(r"C:\AppData\OldSchool\*.sav"))
        .unwrap();
    assert!(in_db.is_some());
    let in_db = in_db.unwrap();
    assert_eq!(in_db.location_type, SaveLocationType::Glob);
    assert!(in_db.is_glob());
}

#[test]
fn test_integration_pipeline_conflicting_game_identity_is_safely_blocked() {
    let db = open_test_db();
    let yaml = r#"
DOOM:
  steam:
    id: 999999
  files:
    <winAppData>/DOOM/Saves:
      tags: [save]
      when:
        - os: windows
"#;
    let manifest = LudusaviManifest::from_yaml_str(yaml).unwrap();
    let detector = LudusaviSaveDetector::new(
        manifest,
        PathResolver::empty().with_variable("<winAppData>", r"C:\AppData"),
    );

    // Jogo local tem nome "DOOM", plataforma Steam, mas AppID 123456 (conflitante com 999999 do manifest)
    let game = Game::new(
        "DOOM",
        Platform::Steam,
        Some("123456".into()),
        Confidence::High,
    );
    savevault_db::repositories::GamesRepo::new(&db.conn)
        .insert(&game)
        .unwrap();

    let report =
        discover_and_persist_saves(&db.conn, std::slice::from_ref(&game), &detector).unwrap();

    // Deve registrar conflito e NÃO persistir nenhum local de save para evitar associação errada
    assert_eq!(report.conflicts.len(), 1);
    assert_eq!(report.persisted_locations.len(), 0);

    let loc_repo = SaveLocationsRepo::new(&db.conn);
    let in_db = loc_repo.list_by_game(game.id).unwrap();
    assert_eq!(
        in_db.len(),
        0,
        "Nenhum save de jogo conflitante pode ser persistido no banco"
    );
}

#[test]
fn test_integration_pipeline_sqlite_error_propagates() {
    let db = open_test_db();
    let yaml = r#"
ValidGame:
  files:
    <winAppData>/ValidGame/Saves:
      tags: [save]
      when:
        - os: windows
"#;
    let manifest = LudusaviManifest::from_yaml_str(yaml).unwrap();
    let detector = LudusaviSaveDetector::new(
        manifest,
        PathResolver::empty().with_variable("<winAppData>", r"C:\AppData"),
    );

    // Jogo NÃO inserido na tabela games: causará violação de Foreign Key no SQLite
    let game = Game::new("ValidGame", Platform::Generic, None, Confidence::Medium);

    let res = discover_and_persist_saves(&db.conn, &[game], &detector);
    assert!(
        res.is_err(),
        "Erros do banco de dados (ex.: Foreign Key) DEVEM se propagar para a camada superior"
    );
}
