pub mod emulators;
pub mod generic;
pub mod gog;
pub mod ludusavi;
pub mod manifest;
pub mod save_detector;
pub mod steam;

pub use ludusavi::LudusaviSaveDetector;
pub use save_detector::{SaveDetector, SaveDiscoveryError, SaveDiscoveryResult};

use anyhow::Result;
use savevault_core::models::{Game, SaveLocation};

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
    // Genéricos (%USERPROFILE%\Saved Games)
    // ------------------------------------------------------------------
    let generic_infos = match generic::GenericDetector::new().detect_with_paths().await {
        Ok(infos) => {
            tracing::info!("Generic: {} jogo(s) encontrado(s)", infos.len());
            infos
        }
        Err(e) => {
            tracing::warn!("Generic detector falhou: {}", e);
            vec![]
        }
    };

    for info in generic_infos {
        save_locations.push(SaveLocation::new(
            info.game.id,
            info.save_path,
            savevault_core::models::SaveLocationType::Directory,
        ));
        games.push(info.game);
    }

    for info in steam_infos {
        games.push(info.game);
    }

    // ------------------------------------------------------------------
    // Ludusavi Manifest — detecção de saves para todos os jogos descobertos
    // (Steam, GOG, Emuladores, Genéricos) com SaveLocationType correto
    // ------------------------------------------------------------------
    match manifest::get_manifest_path(cache_dir).await {
        Ok(manifest_path) => match LudusaviSaveDetector::from_file(&manifest_path) {
            Ok(detector) => {
                for game in &games {
                    match detector.detect_save_locations(game) {
                        Ok(res) => {
                            let locs = res.into_locations();
                            if !locs.is_empty() {
                                tracing::debug!(
                                    "Ludusavi: {} local(is) de save para '{}' ({:?})",
                                    locs.len(),
                                    game.name,
                                    game.platform
                                );
                                save_locations.extend(locs);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Erro ao detectar saves para '{}': {}", game.name, e);
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Falha ao carregar manifest: {}. Saves não serão detectados.",
                    e
                );
            }
        },
        Err(e) => {
            tracing::warn!("Manifest indisponível: {}. Saves não serão detectados.", e);
        }
    }

    // Deduplica save_locations por (game_id, path normalizado case-insensitive)
    let mut seen_locs = std::collections::HashSet::new();
    save_locations.retain(|loc| {
        let norm = savevault_core::path::normalize_windows_path(&loc.path)
            .unwrap_or_else(|_| loc.path.clone());
        let key = (loc.game_id, norm.to_string_lossy().to_lowercase());
        seen_locs.insert(key)
    });

    tracing::info!(
        "Discovery concluído: {} jogo(s), {} caminho(s) de save detectado(s).",
        games.len(),
        save_locations.len()
    );

    Ok(DiscoveryResult {
        games,
        save_locations,
    })
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
