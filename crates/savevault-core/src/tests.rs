#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {

    // ------------------------------------------------------------------
    // Config
    // ------------------------------------------------------------------
    mod config_tests {
        use crate::config::Config;
        use tempfile::tempdir;

        #[test]
        fn default_config_has_sensible_values() {
            let cfg = Config::default();
            assert_eq!(cfg.general.max_versions, 10);
            assert_eq!(cfg.general.log_level, "info");
            assert!(cfg.general.debounce_ms > 0);
            assert_eq!(cfg.cloud.provider, "local_folder");
        }

        #[test]
        fn config_roundtrip_toml() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("config.toml");

            let original = Config::default();
            original.save(&path).expect("salvar config");

            let loaded = Config::load(&path).expect("carregar config");
            assert_eq!(loaded.general.max_versions, original.general.max_versions);
            assert_eq!(loaded.general.log_level, original.general.log_level);
            assert_eq!(loaded.cloud.provider, original.cloud.provider);
        }

        #[test]
        fn config_load_nonexistent_returns_default() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("nonexistent.toml");
            let cfg = Config::load(&path).expect("deve retornar default sem erro");
            assert_eq!(cfg.general.max_versions, 10);
        }

        #[test]
        fn config_save_creates_parent_dirs() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("nested").join("deep").join("config.toml");
            let cfg = Config::default();
            cfg.save(&path).expect("deve criar diretórios e salvar");
            assert!(path.exists());
        }
    }

    // ------------------------------------------------------------------
    // Models
    // ------------------------------------------------------------------
    mod model_tests {
        use crate::models::{
            Confidence, Game, Platform, SaveLocation, SaveLocationType,
            SyncStatus,
        };
        use std::path::PathBuf;
        use uuid::Uuid;

        #[test]
        fn game_new_generates_unique_ids() {
            let g1 = Game::new("Jogo A", Platform::Steam, Some("123".into()), Confidence::High);
            let g2 = Game::new("Jogo A", Platform::Steam, Some("123".into()), Confidence::High);
            assert_ne!(g1.id, g2.id, "IDs devem ser únicos");
        }

        #[test]
        fn game_fields_are_set_correctly() {
            let game = Game::new("Stardew Valley", Platform::Steam, Some("413150".into()), Confidence::High);
            assert_eq!(game.name, "Stardew Valley");
            assert_eq!(game.platform, Platform::Steam);
            assert_eq!(game.platform_id, Some("413150".into()));
            assert_eq!(game.confidence, Confidence::High);
        }

        #[test]
        fn platform_variants_are_distinct() {
            assert_ne!(Platform::Steam, Platform::Gog);
            assert_ne!(Platform::Dolphin, Platform::RetroArch);
        }

        #[test]
        fn confidence_variants_are_distinct() {
            assert_ne!(Confidence::High, Confidence::Low);
            assert_ne!(Confidence::Medium, Confidence::High);
        }

        #[test]
        fn sync_status_variants_are_distinct() {
            assert_ne!(SyncStatus::Pending, SyncStatus::Synced);
            assert_ne!(SyncStatus::Failed, SyncStatus::PendingOffline);
        }

        #[test]
        fn save_location_with_directory_type() {
            let loc = SaveLocation {
                id: Uuid::new_v4(),
                game_id: Uuid::new_v4(),
                path: PathBuf::from(r"C:\Users\pedro\AppData\Roaming\StardewValley\Saves"),
                location_type: SaveLocationType::Directory,
            };
            assert_eq!(loc.location_type, SaveLocationType::Directory);
            assert!(loc.is_directory());
            assert!(!loc.is_file());
        }

        #[test]
        fn save_location_new_constructor_sets_fields() {
            let game_id = Uuid::new_v4();
            let path = PathBuf::from(r"C:\Games\Save\file.sav");
            let loc = SaveLocation::new(game_id, path.clone(), SaveLocationType::File);

            assert_eq!(loc.game_id, game_id);
            assert_eq!(loc.path, path);
            assert_eq!(loc.location_type, SaveLocationType::File);
            assert!(loc.is_file());
            assert!(!loc.is_directory());
        }
    }
}
