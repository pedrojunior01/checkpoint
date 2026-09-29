//! Detector de emuladores: Dolphin, PCSX2, PPSSPP, RetroArch.
//! Verifica as pastas padrão de dados de cada emulador.

use anyhow::Result;
use savevault_core::models::{Confidence, Game, Platform, SaveLocation, SaveLocationType};
use std::path::PathBuf;
use tracing::debug;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct DiscoveredEmulator {
    pub game: Game,
    pub save_locations: Vec<SaveLocation>,
}

#[derive(Default)]
pub struct EmulatorDetector;

impl EmulatorDetector {
    pub fn new() -> Self {
        Self
    }

    pub async fn detect(&self) -> Result<Vec<Game>> {
        let emus = self.detect_with_locations().await?;
        Ok(emus.into_iter().map(|e| e.game).collect())
    }

    pub async fn detect_with_locations(&self) -> Result<Vec<DiscoveredEmulator>> {
        let mut results = Vec::new();

        for (platform, name, paths) in emulator_profiles() {
            for path in paths {
                if path.exists() {
                    debug!("{} encontrado em: {}", name, path.display());
                    let game = Game::new(
                        format!("{} (Emulador)", name),
                        platform.clone(),
                        None,
                        Confidence::High,
                    );
                    let location = SaveLocation {
                        id: Uuid::new_v4(),
                        game_id: game.id,
                        path: path.clone(),
                        location_type: SaveLocationType::Directory,
                    };
                    results.push(DiscoveredEmulator {
                        game,
                        save_locations: vec![location],
                    });
                    break; // Uma entrada por emulador
                }
            }
        }

        Ok(results)
    }
}

/// Retorna os perfis de emuladores conhecidos:
/// (Platform, nome display, lista de possíveis caminhos de dados)
fn emulator_profiles() -> Vec<(Platform, &'static str, Vec<PathBuf>)> {
    let appdata = std::env::var("APPDATA").unwrap_or_default();
    let localappdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let docs = get_documents_dir();

    vec![
        (
            Platform::Dolphin,
            "Dolphin",
            vec![
                PathBuf::from(&appdata).join("Dolphin Emulator"),
                PathBuf::from("C:\\Users\\Public\\Documents\\Dolphin Emulator"),
            ],
        ),
        (
            Platform::Pcsx2,
            "PCSX2",
            vec![
                PathBuf::from(&docs).join("PCSX2"),
                PathBuf::from(&appdata).join("PCSX2"),
            ],
        ),
        (
            Platform::Ppsspp,
            "PPSSPP",
            vec![
                PathBuf::from(&appdata).join("PPSSPP"),
                PathBuf::from(&docs).join("PPSSPP"),
            ],
        ),
        (
            Platform::RetroArch,
            "RetroArch",
            vec![
                PathBuf::from(&appdata).join("RetroArch"),
                PathBuf::from(&localappdata).join("RetroArch"),
                PathBuf::from("C:\\RetroArch-Win64"),
            ],
        ),
    ]
}

fn get_documents_dir() -> String {
    // Tenta via USERPROFILE\Documents (fallback simples)
    let userprofile = std::env::var("USERPROFILE").unwrap_or_default();
    format!("{}\\Documents", userprofile)
}
