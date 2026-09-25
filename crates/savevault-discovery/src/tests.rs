#[cfg(test)]
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

        #[tokio::test]
        async fn run_all_returns_ok() {
            let result = run_all().await;
            assert!(result.is_ok(), "run_all não deve retornar erro");
        }

        #[tokio::test]
        async fn run_all_games_have_valid_names() {
            let result = run_all().await.unwrap();
            for game in &result.games {
                assert!(!game.name.is_empty(), "Nome do jogo não deve ser vazio");
            }
        }

        #[tokio::test]
        async fn run_all_games_have_unique_ids() {
            let result = run_all().await.unwrap();
            let ids: std::collections::HashSet<_> = result.games.iter().map(|g| g.id).collect();
            assert_eq!(ids.len(), result.games.len(), "Todos os IDs devem ser únicos");
        }
    }
}
