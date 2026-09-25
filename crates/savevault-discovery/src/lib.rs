pub mod emulators;
pub mod generic;
pub mod gog;
pub mod manifest;
pub mod steam;
pub mod save_detector;
pub mod ludusavi;

pub use save_detector::{SaveDetector, SaveDiscoveryResult, SaveDiscoveryError};
pub use ludusavi::LudusaviSaveDetector;

use anyhow::Result;
use savevault_core::models::{Game, SaveLocation, SaveLocationType};
use uuid::Uuid;

/// Resultado de uma operação de discovery.
#[derive(Debug)]
pub struct DiscoveryResult {
    pub games: Vec<Game>,
    /// Caminhos de save encontrados via Ludusavi Manifest, associados a cada jogo.
    pub save_locations: Vec<SaveLocation>,
}

/// Executa todos os detectors disponíveis e retorna os jogos e saves encontrados.
///
/// O `cache_dir` é usado para armazenar o manifest.yaml do Ludusavi.
/// Normalmente é `%APPDATA%\SaveVault\`.
pub async fn run_all(cache_dir: &std::path::Path) -> Result<DiscoveryResult> {
    let mut games: Vec<Game> = Vec::new();
    let mut save_locations: Vec<SaveLocation> = Vec::new();

    // ------------------------------------------------------------------
    // Steam — com caminhos de instalação para resolução de saves
    // ------------------------------------------------------------------
    let steam_infos = match steam::SteamDetector::new().detect_with_paths().await {
        Ok(infos) => {
            tracing::info!("Steam: {} jogo(s) encontrado(s)", infos.len());
            infos
        }
        Err(e) => {
            tracing::warn!("Steam detector falhou: {}", e);
            vec![]
        }
    };

    // ------------------------------------------------------------------
    // GOG (stub — v0.4 completo via registry)
    // ------------------------------------------------------------------
    match gog::GogDetector::new().detect().await {
        Ok(mut found) => {
            tracing::info!("GOG: {} jogo(s) encontrado(s)", found.len());
            games.append(&mut found);
        }
        Err(e) => tracing::warn!("GOG detector falhou: {}", e),
    }

    // ------------------------------------------------------------------
    // Emuladores
    // ------------------------------------------------------------------
    match emulators::EmulatorDetector::new().detect().await {
        Ok(mut found) => {
            tracing::info!("Emuladores: {} entrada(s) encontrada(s)", found.len());
            games.append(&mut found);
        }
        Err(e) => tracing::warn!("Emulator detector falhou: {}", e),
    }

    // ------------------------------------------------------------------
    // Ludusavi Manifest — detecção de saves para jogos Steam
    // ------------------------------------------------------------------
    match manifest::get_manifest_path(cache_dir).await {
        Ok(manifest_path) => {
            match manifest::ManifestDetector::load(&manifest_path) {
                Ok(detector) => {
                    for info in &steam_infos {
                        let game = &info.game;
                        let app_id = match game.platform_id.as_ref().and_then(|id| id.parse::<u64>().ok()) {
                            Some(id) => id,
                            None => {
                                games.push(game.clone());
                                continue;
                            }
                        };

                        let ctx = manifest::PathContext::from_env(info.install_path.as_ref());
                        let paths = detector.find_save_paths(app_id, &ctx);

                        for path in paths {
                            save_locations.push(SaveLocation {
                                id: Uuid::new_v4(),
                                game_id: game.id,
                                path,
                                location_type: SaveLocationType::Directory,
                            });
                        }

                        games.push(game.clone());
                    }
                }
                Err(e) => {
                    tracing::warn!("Falha ao carregar manifest: {}. Saves não serão detectados.", e);
                    for info in &steam_infos {
                        games.push(info.game.clone());
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!("Manifest indisponível: {}. Saves não serão detectados.", e);
            for info in &steam_infos {
                games.push(info.game.clone());
            }
        }
    }

    tracing::info!(
        "Discovery concluído: {} jogo(s), {} caminho(s) de save detectado(s).",
        games.len(),
        save_locations.len()
    );

    Ok(DiscoveryResult { games, save_locations })
}

/// Conecta os jogos encontrados pelo Game Discovery ao Save Discovery,
/// retornando todos os locais de save descobertos pelo detector fornecido.
pub fn discover_saves_for_games<D: SaveDetector>(
    games: &[Game],
    detector: &D,
) -> Vec<savevault_core::models::SaveLocation> {
    let mut all_locations = Vec::new();
    for game in games {
        match detector.detect_save_locations(game) {
            Ok(result) => {
                all_locations.extend(result.into_locations());
            }
            Err(e) => {
                tracing::warn!("Erro ao descobrir saves para '{}': {}", game.name, e);
            }
        }
    }
    all_locations
}

#[cfg(test)]
mod tests;
