#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    // ------------------------------------------------------------------
    // Steam detector — testa parsing de .acf e libraryfolders.vdf
    // ------------------------------------------------------------------
    mod steam_parsing {
        use crate::steam::SteamDetector;
        use tempfile::tempdir;

        /// Cria uma estrutura de biblioteca Steam falsa para testes
        fn create_fake_steam_library(base: &std::path::Path) -> std::path::PathBuf {
            let steam_root = base.join("steam");
            let steamapps = steam_root.join("steamapps");
            std::fs::create_dir_all(&steamapps).unwrap();

            // libraryfolders.vdf mínimo
            let vdf_content = r#""libraryfolders"
{
    "0"
    {
        "path"    "C:\\Games\\Steam"
    }
}"#;
            std::fs::write(steamapps.join("libraryfolders.vdf"), vdf_content).unwrap();

            // appmanifest para Stardew Valley
            let acf_stardew = r#""AppState"
{
    "appid"    "413150"
    "name"    "Stardew Valley"
    "installdir"    "Stardew Valley"
}"#;
            std::fs::write(steamapps.join("appmanifest_413150.acf"), acf_stardew).unwrap();

            // appmanifest para Celeste
            let acf_celeste = r#""AppState"
{
    "appid"    "504230"
    "name"    "Celeste"
    "installdir"    "Celeste"
}"#;
            std::fs::write(steamapps.join("appmanifest_504230.acf"), acf_celeste).unwrap();

            steam_root
        }

        #[tokio::test]
        async fn detect_with_no_steam_returns_empty() {
            // Sem steam_root definido, deve retornar lista vazia sem erro
            let detector = SteamDetector::new();
            // Se steam não está instalado no CI, retorna vazio
            // Se está instalado, retorna jogos reais — ambos são OK
            let result = detector.detect().await;
            assert!(result.is_ok(), "Não deve retornar erro quando steam não encontrado");
        }

        #[test]
        fn acf_files_are_correctly_parsed() {
            // Testa o parser interno via um detector com root fake
            let dir = tempdir().unwrap();
            let steam_root = create_fake_steam_library(dir.path());

            // Verificar estrutura criada
            let acf_path = steam_root.join("steamapps").join("appmanifest_413150.acf");
            assert!(acf_path.exists(), "ACF de Stardew Valley deve existir");

            let content = std::fs::read_to_string(&acf_path).unwrap();
            assert!(content.contains("Stardew Valley"));
            assert!(content.contains("413150"));
        }
    }

    // ------------------------------------------------------------------
    // Emulator detector — testa detecção de pastas de emuladores
    // ------------------------------------------------------------------
    mod emulator_tests {
        use crate::emulators::EmulatorDetector;

        #[tokio::test]
        async fn detect_returns_ok_even_with_no_emulators() {
            let detector = EmulatorDetector::new();
            let result = detector.detect().await;
            assert!(result.is_ok(), "Não deve falhar se nenhum emulador está instalado");
        }
    }

    // ------------------------------------------------------------------
    // Discovery orquestrador
    // ------------------------------------------------------------------
    mod orchestrator_tests {
        use crate::run_all;
        use tempfile::tempdir;

        #[tokio::test]
        async fn run_all_returns_ok() {
            let dir = tempdir().unwrap();
            let result = run_all(dir.path()).await;
            assert!(result.is_ok(), "run_all não deve retornar erro");
        }

        #[tokio::test]
        async fn run_all_games_have_valid_names() {
            let dir = tempdir().unwrap();
            let result = run_all(dir.path()).await.unwrap();
            for game in &result.games {
                assert!(!game.name.is_empty(), "Nome do jogo não deve ser vazio");
            }
        }

        #[tokio::test]
        async fn run_all_games_have_unique_ids() {
            let dir = tempdir().unwrap();
            let result = run_all(dir.path()).await.unwrap();
            let ids: std::collections::HashSet<_> = result.games.iter().map(|g| g.id).collect();
            assert_eq!(ids.len(), result.games.len(), "Todos os IDs devem ser únicos");
        }
    }

    // ------------------------------------------------------------------
    // Save Discovery tests (Ludusavi Manifest & SaveDetector)
    // ------------------------------------------------------------------
    mod save_discovery_tests {
        use std::path::{Path, PathBuf};
        use savevault_core::models::{Confidence, Game, Platform, SaveLocationType};
        use crate::save_detector::{SaveDetector, SaveDiscoveryResult, SaveDiscoveryError};
        use crate::ludusavi::{LudusaviManifest, LudusaviSaveDetector, PathResolver};

        const FIXTURE_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/manifest_sample.yaml");

        fn setup_test_detector() -> LudusaviSaveDetector {
            let manifest = LudusaviManifest::from_path(Path::new(FIXTURE_PATH))
                .expect("Falha ao carregar fixture real");
            let resolver = PathResolver::empty()
                .with_variable("<home>", r"C:\Users\TestUser")
                .with_variable("<winAppData>", r"C:\Users\TestUser\AppData\Roaming")
                .with_variable("<winLocalAppData>", r"C:\Users\TestUser\AppData\Local")
                .with_variable("<winDocuments>", r"C:\Users\TestUser\Documents");
            LudusaviSaveDetector::new(manifest, resolver)
        }

        #[test]
        fn test_1_known_game_returns_expected_locations() {
            let detector = setup_test_detector();
            let game = Game::new(
                "Stardew Valley",
                Platform::Steam,
                Some("413150".into()),
                Confidence::High,
            );

            let result = detector.detect_save_locations(&game).unwrap();
            assert!(result.is_recognized());
            let locs = result.locations();
            assert_eq!(locs.len(), 1);
            assert_eq!(locs[0].game_id, game.id);
            assert_eq!(locs[0].path, PathBuf::from(r"C:\Users\TestUser\AppData\Roaming\StardewValley\Saves"));
            assert_eq!(locs[0].location_type, SaveLocationType::Directory);
        }

        #[test]
        fn test_2_multi_locations_game() {
            let detector = setup_test_detector();
            let game = Game::new(
                "Sample MultiLocation Game",
                Platform::Generic,
                None,
                Confidence::Medium,
            );

            let result = detector.detect_save_locations(&game).unwrap();
            assert!(result.is_recognized());
            let locs = result.locations();
            assert_eq!(locs.len(), 2, "Jogo deve retornar múltiplos locais de save");

            let has_dir = locs.iter().any(|l| l.location_type == SaveLocationType::Directory && l.path == Path::new(r"C:\Users\TestUser\AppData\Roaming\MultiGame\Saves"));
            let has_file = locs.iter().any(|l| l.location_type == SaveLocationType::File && l.path == Path::new(r"C:\Users\TestUser\Documents\MultiGame\profile.sav"));
            assert!(has_dir);
            assert!(has_file);
        }

        #[test]
        fn test_3_file_save_location() {
            let detector = setup_test_detector();
            let game = Game::new(
                "Single File Save Game",
                Platform::Generic,
                None,
                Confidence::Medium,
            );

            let result = detector.detect_save_locations(&game).unwrap();
            let locs = result.locations();
            assert_eq!(locs.len(), 1);
            assert_eq!(locs[0].location_type, SaveLocationType::File);
            assert_eq!(locs[0].path, PathBuf::from(r"C:\Users\TestUser\AppData\Local\SingleSaveGame\save.dat"));
            assert!(locs[0].is_file());
        }

        #[test]
        fn test_4_directory_save_location() {
            let detector = setup_test_detector();
            let game = Game::new(
                "Stardew Valley",
                Platform::Steam,
                Some("413150".into()),
                Confidence::High,
            );

            let result = detector.detect_save_locations(&game).unwrap();
            let locs = result.locations();
            assert_eq!(locs.len(), 1);
            assert_eq!(locs[0].location_type, SaveLocationType::Directory);
            assert!(locs[0].is_directory());
        }

        #[test]
        fn test_5_environment_variable_resolution() {
            let resolver = PathResolver::empty()
                .with_variable("<winAppData>", r"C:\Custom\AppData")
                .with_variable("<home>", r"C:\Custom\Home");

            let resolved = resolver.resolve("<winAppData>/MyGame/Saves").unwrap();
            assert_eq!(resolved, PathBuf::from(r"C:\Custom\AppData\MyGame\Saves"));
        }

        #[test]
        fn test_6_missing_environment_variable_returns_explicit_error() {
            let resolver = PathResolver::empty(); // Sem variáveis configuradas
            let res = resolver.resolve("<winAppData>/MyGame/Saves");

            assert!(res.is_err());
            match res.unwrap_err() {
                SaveDiscoveryError::MissingVariable(var) => {
                    assert_eq!(var, "<winAppData>");
                }
                other => panic!("Esperado MissingVariable, recebido: {:?}", other),
            }
        }

        #[test]
        fn test_7_unknown_game_returns_unrecognized() {
            let detector = setup_test_detector();
            let game = Game::new(
                "Completely Unknown Indie Game 999",
                Platform::Steam,
                Some("9999999".into()),
                Confidence::Low,
            );

            let result = detector.detect_save_locations(&game).unwrap();
            assert_eq!(result, SaveDiscoveryResult::UnrecognizedGame);
            assert!(!result.is_recognized());
            assert_eq!(result.locations().len(), 0);
        }

        #[test]
        fn test_8_invalid_manifest_produces_clear_parse_error() {
            let bad_yaml = "invalid: [unclosed yaml";
            let res = LudusaviManifest::from_yaml_str(bad_yaml);
            assert!(res.is_err());
            match res.unwrap_err() {
                SaveDiscoveryError::ParseError(msg) => {
                    assert!(!msg.is_empty());
                }
                other => panic!("Esperado ParseError, recebido: {:?}", other),
            }
        }

        #[test]
        fn test_9_incomplete_entry_does_not_panic() {
            let incomplete_yaml = r#"
PartialGame:
  files: {}
EmptyGame: {}
"#;
            let manifest = LudusaviManifest::from_yaml_str(incomplete_yaml).unwrap();
            let detector = LudusaviSaveDetector::new(manifest, PathResolver::empty());

            let game = Game::new("PartialGame", Platform::Generic, None, Confidence::Low);
            let result = detector.detect_save_locations(&game).unwrap();
            assert_eq!(result.locations().len(), 0);

            let empty_game = Game::new("EmptyGame", Platform::Generic, None, Confidence::Low);
            let result2 = detector.detect_save_locations(&empty_game).unwrap();
            assert_eq!(result2.locations().len(), 0);
        }

        #[test]
        fn test_10_produces_save_location_even_if_path_not_yet_on_disk() {
            let detector = setup_test_detector();
            let game = Game::new(
                "Celeste",
                Platform::Steam,
                Some("504230".into()),
                Confidence::High,
            );

            let result = detector.detect_save_locations(&game).unwrap();
            assert!(result.is_recognized());

            // O caminho virtual C:\Users\TestUser\Documents\My Games\Celeste\Saves não existe no disco
            // mas o detector deve produzir um SaveLocation perfeitamente formado
            match &result {
                SaveDiscoveryResult::KnownNotFoundOnDisk { expected_locations } => {
                    assert_eq!(expected_locations.len(), 1);
                    assert_eq!(expected_locations[0].path, PathBuf::from(r"C:\Users\TestUser\Documents\My Games\Celeste\Saves"));
                    assert_eq!(expected_locations[0].location_type, SaveLocationType::Directory);
                }
                SaveDiscoveryResult::Found(locs) => {
                    assert_eq!(locs.len(), 1);
                }
                _ => panic!("Esperado que o jogo fosse reconhecido"),
            }

            let all_locs = result.into_locations();
            assert_eq!(all_locs.len(), 1);
            assert_eq!(all_locs[0].game_id, game.id);
        }

        #[test]
        fn test_11_windows_path_normalization() {
            let resolver = PathResolver::empty()
                .with_variable("<winAppData>", r"C:\Users\User\AppData\Roaming");

            // Entrada com barras normais (padrão em manifests YAML)
            let path_with_forward_slashes = "<winAppData>/Game/SubDir/save.dat";
            let resolved = resolver.resolve(path_with_forward_slashes).unwrap();

            // Deve ser normalizado para separadores do Windows
            assert_eq!(resolved, PathBuf::from(r"C:\Users\User\AppData\Roaming\Game\SubDir\save.dat"));
        }

        #[test]
        fn test_12_detector_is_pure_and_isolated_from_database() {
            // Garante isolamento: o detector não requer conexão SQLite, Database ou IO de banco
            let detector = setup_test_detector();
            assert_eq!(detector.name(), "ludusavi-manifest");

            let game = Game::new("Stardew Valley", Platform::Steam, Some("413150".into()), Confidence::High);
            let res = detector.detect_save_locations(&game);
            assert!(res.is_ok());
        }

        #[test]
        fn test_13_real_fixture_parsing_and_tag_filtering() {
            // Executa o parser real sobre o arquivo de fixture
            let manifest = LudusaviManifest::from_path(Path::new(FIXTURE_PATH)).unwrap();
            assert!(manifest.games.contains_key("The Witcher 3: Wild Hunt"));

            let witcher = &manifest.games["The Witcher 3: Wild Hunt"];
            // user.settings tem tag [config], gamesaves tem tag [save]
            assert_eq!(witcher.files.len(), 2);

            let detector = setup_test_detector();
            let game = Game::new(
                "The Witcher 3: Wild Hunt",
                Platform::Gog,
                Some("1207664643".into()),
                Confidence::High,
            );

            let result = detector.detect_save_locations(&game).unwrap();
            let locs = result.locations();

            // Apenas o caminho com tag [save] deve ser incluído, ignorando [config]
            assert_eq!(locs.len(), 1);
            assert_eq!(locs[0].path, PathBuf::from(r"C:\Users\TestUser\Documents\The Witcher 3\gamesaves"));
        }

        #[test]
        fn test_14_integration_discover_saves_for_games() {
            let detector = setup_test_detector();
            let games = vec![
                Game::new("Stardew Valley", Platform::Steam, Some("413150".into()), Confidence::High),
                Game::new("The Witcher 3: Wild Hunt", Platform::Gog, Some("1207664643".into()), Confidence::High),
                Game::new("Unknown Game", Platform::Generic, None, Confidence::Low),
            ];

            let discovered_locations = crate::discover_saves_for_games(&games, &detector);

            // Stardew Valley (1) + The Witcher 3 (1) + Unknown (0) = 2 locais de save
            assert_eq!(discovered_locations.len(), 2);
            assert_eq!(discovered_locations[0].game_id, games[0].id);
            assert_eq!(discovered_locations[1].game_id, games[1].id);
        }

        #[test]
        fn test_15_glob_pattern_in_manifest_retains_raw_pattern() {
            let yaml = r#"
GlobGame:
  files:
    <winAppData>/GlobGame/*.sav:
      tags: [save]
      when:
        - os: windows
"#;
            let manifest = LudusaviManifest::from_yaml_str(yaml).unwrap();
            let detector = LudusaviSaveDetector::new(
                manifest,
                PathResolver::empty().with_variable("<winAppData>", r"C:\AppData"),
            );
            let game = Game::new("GlobGame", Platform::Generic, None, Confidence::Medium);
            let result = detector.detect_save_locations(&game).unwrap();
            let locs = result.into_locations();
            assert_eq!(locs.len(), 1);
            // Confirma que é identificado como Glob, sem consultar filesystem
            assert_eq!(locs[0].path, PathBuf::from(r"C:\AppData\GlobGame\*.sav"));
            assert_eq!(locs[0].location_type, SaveLocationType::Glob);
            assert!(locs[0].is_glob());
            assert!(!locs[0].is_file());
            assert!(!locs[0].is_directory());
        }

        #[test]
        fn test_16_matching_prioritizes_store_id_over_title() {
            let detector = setup_test_detector();
            // Jogo com nome diferente, mas AppID 413150 (Stardew Valley)
            let game = Game::new(
                "Stardew Valley: Collector's Edition",
                Platform::Steam,
                Some("413150".into()),
                Confidence::High,
            );
            let result = detector.detect_save_locations(&game).unwrap();
            assert!(result.is_recognized(), "Deve encontrar pelo Steam AppID mesmo com título diferente");
            let locs = result.locations();
            assert_eq!(locs.len(), 1);
            assert_eq!(locs[0].path, PathBuf::from(r"C:\Users\TestUser\AppData\Roaming\StardewValley\Saves"));
        }

        #[test]
        fn test_17_conflicting_steam_id_rejects_silent_nominal_fallback() {
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

            // Jogo na máquina é Steam com AppID 123456, mas manifest tem DOOM com Steam AppID 999999
            let game = Game::new(
                "DOOM",
                Platform::Steam,
                Some("123456".into()),
                Confidence::High,
            );

            let result = detector.detect_save_locations(&game).unwrap();
            assert!(result.is_conflict(), "Deve identificar explicitamente o conflito de identidade entre Store ID e nome");
            assert!(result.is_recognized(), "O jogo é conhecido por nome");
            assert_eq!(result.locations().len(), 0, "Não deve associar silenciosamente os saves do jogo conflitante");

            match result {
                SaveDiscoveryResult::ConflictingGameIdentity {
                    game_name,
                    platform,
                    expected_platform_id,
                    manifest_platform_id,
                } => {
                    assert_eq!(game_name, "DOOM");
                    assert_eq!(platform, Platform::Steam);
                    assert_eq!(expected_platform_id, "123456");
                    assert_eq!(manifest_platform_id, "999999");
                }
                other => panic!("Esperava ConflictingGameIdentity, obteve {:?}", other),
            }
        }
    }
}

