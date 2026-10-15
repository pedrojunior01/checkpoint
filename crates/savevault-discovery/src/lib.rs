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
/// Normalmente é `%APPDATA%\checkpoint\` (ou `%APPDATA%\SaveVault\`).
pub async fn run_all(cache_dir: &std::path::Path) -> Result<DiscoveryResult> {
    let mut games: Vec<Game> = Vec::new();
    let mut save_locations: Vec<SaveLocation> = Vec::new();
    let mut known_install_paths: Vec<std::path::PathBuf> = Vec::new();

    // ------------------------------------------------------------------
    // 1. Steam — com caminhos de instalação para resolução de saves
    // ------------------------------------------------------------------
    let steam_infos = match steam::SteamDetector::new().detect_with_paths().await {
        Ok(infos) => {
            tracing::info!("Steam: {} jogo(s) encontrado(s)", infos.len());
            for info in &infos {
                if let Some(ref p) = info.install_path {
                    known_install_paths.push(p.clone());
                }
            }
            infos
        }
        Err(e) => {
            tracing::warn!("Steam detector falhou: {}", e);
            vec![]
        }
    };

    // ------------------------------------------------------------------
    // 2. GOG Galaxy — via Registro do Windows ou fallback em pasta
    // ------------------------------------------------------------------
    let gog_infos = match gog::GogDetector::new().detect_with_paths().await {
        Ok(infos) => {
            tracing::info!("GOG: {} jogo(s) encontrado(s)", infos.len());
            for info in &infos {
                if let Some(ref p) = info.install_path {
                    known_install_paths.push(p.clone());
                }
            }
            infos
        }
        Err(e) => {
            tracing::warn!("GOG detector falhou: {}", e);
            vec![]
        }
    };

    // ------------------------------------------------------------------
    // 3. Emuladores (Dolphin, PCSX2, PPSSPP, RetroArch)
    // ------------------------------------------------------------------
    match emulators::EmulatorDetector::new().detect().await {
        Ok(mut found) => {
            tracing::info!("Emuladores: {} entrada(s) encontrada(s)", found.len());
            games.append(&mut found);
        }
        Err(e) => tracing::warn!("Emulator detector falhou: {}", e),
    }

    // ------------------------------------------------------------------
    // 4. Detector Genérico / Heurístico (jogos fora de lojas ou DRM-free)
    // ------------------------------------------------------------------
    let generic_detector = generic::GenericDetector::with_known_paths(known_install_paths);
    let generic_infos = match generic_detector.detect_with_locations().await {
        Ok(infos) => {
            tracing::info!("Generic: {} jogo(s)/pasta(s) encontrado(s)", infos.len());
            infos
        }
        Err(e) => {
            tracing::warn!("Generic detector falhou: {}", e);
            vec![]
        }
    };

    // ------------------------------------------------------------------
    // 5. Ludusavi Manifest — detecção e resolução de saves
    // ------------------------------------------------------------------
    match manifest::get_manifest_path(cache_dir).await {
        Ok(manifest_path) => {
            match manifest::ManifestDetector::load(&manifest_path) {
                Ok(detector) => {
                    // Resolução de saves para Steam
                    for info in &steam_infos {
                        let game = &info.game;
                        let app_id = game.platform_id.as_ref().and_then(|id| id.parse::<u64>().ok());
                        let ctx = manifest::PathContext::from_env(info.install_path.as_ref());

                        let mut paths = if let Some(id) = app_id {
                            detector.find_save_paths(id, &ctx)
                        } else {
                            detector.find_save_paths_by_title(&game.name, &ctx)
                        };

                        // Fallback: se o manifest não mapeou, procura subpastas de save no diretório de instalação
                        // ou no diretório de userdata da Steam. NUNCA adiciona a pasta raiz de instalação
                        // para evitar monitorar DLLs e arquivos de assets gigantes (ex: .pak de 18GB).
                        if paths.is_empty() {
                            if let Some(ref install_path) = info.install_path {
                                for sub in &["saves", "save", "saved", "savegames", "SaveData", "savedgames"] {
                                    let sub_p = install_path.join(sub);
                                    if sub_p.exists() {
                                        paths.push(sub_p);
                                        break;
                                    }
                                }
                            }
                            if paths.is_empty() {
                                if let Some(id) = app_id {
                                    let steam_root = std::env::var("ProgramFiles(x86)")
                                        .map(|p| std::path::PathBuf::from(p).join("Steam"))
                                        .unwrap_or_else(|_| std::path::PathBuf::from(r"C:\Program Files (x86)\Steam"));
                                    let userdata = steam_root.join("userdata");
                                    if userdata.exists() {
                                        if let Ok(entries) = std::fs::read_dir(&userdata) {
                                            for e in entries.flatten() {
                                                let save_folder = e.path().join(id.to_string());
                                                if save_folder.exists() {
                                                    let remote_sub = save_folder.join("remote");
                                                    if remote_sub.exists() {
                                                        paths.push(remote_sub);
                                                    } else {
                                                        paths.push(save_folder);
                                                    }
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        for path in paths {
                            let location_type = if savevault_core::path::is_glob_pattern(&path.to_string_lossy()) {
                                SaveLocationType::Glob
                            } else if path.is_file() || path.extension().is_some() {
                                SaveLocationType::File
                            } else {
                                SaveLocationType::Directory
                            };

                            save_locations.push(SaveLocation {
                                id: Uuid::new_v4(),
                                game_id: game.id,
                                path,
                                location_type,
                            });
                        }

                        games.push(game.clone());
                    }

                    // Resolução de saves para GOG
                    for info in &gog_infos {
                        let game = &info.game;
                        let gog_id = game.platform_id.as_ref().and_then(|id| id.parse::<u64>().ok());
                        let ctx = manifest::PathContext::from_env(info.install_path.as_ref());

                        let mut paths = detector.find_save_paths_gog(gog_id, &game.name, &ctx);

                        // Se o manifest não encontrou, usa os saves locais detectados pelo registry/pasta do GOG
                        if paths.is_empty() {
                            for loc in &info.save_paths {
                                paths.push(loc.path.clone());
                            }
                        }

                        for path in paths {
                            let location_type = if savevault_core::path::is_glob_pattern(&path.to_string_lossy()) {
                                SaveLocationType::Glob
                            } else if path.is_file() || path.extension().is_some() {
                                SaveLocationType::File
                            } else {
                                SaveLocationType::Directory
                            };

                            save_locations.push(SaveLocation {
                                id: Uuid::new_v4(),
                                game_id: game.id,
                                path,
                                location_type,
                            });
                        }

                        games.push(game.clone());
                    }
                }
                Err(e) => {
                    tracing::warn!("Falha ao carregar manifest: {}. Saves serão detectados apenas localmente.", e);
                    for info in &steam_infos {
                        if let Some(ref install_path) = info.install_path {
                            save_locations.push(SaveLocation {
                                id: Uuid::new_v4(),
                                game_id: info.game.id,
                                path: install_path.clone(),
                                location_type: SaveLocationType::Directory,
                            });
                        }
                        games.push(info.game.clone());
                    }
                    for info in &gog_infos {
                        save_locations.extend(info.save_paths.clone());
                        games.push(info.game.clone());
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!("Manifest indisponível: {}. Saves serão detectados apenas localmente.", e);
            for info in &steam_infos {
                if let Some(ref install_path) = info.install_path {
                    save_locations.push(SaveLocation {
                        id: Uuid::new_v4(),
                        game_id: info.game.id,
                        path: install_path.clone(),
                        location_type: SaveLocationType::Directory,
                    });
                }
                games.push(info.game.clone());
            }
            for info in &gog_infos {
                save_locations.extend(info.save_paths.clone());
                games.push(info.game.clone());
            }
        }
    }

    // ------------------------------------------------------------------
    // 6. Adiciona jogos e locais do Detector Genérico (Com Deduplicação)
    // ------------------------------------------------------------------
    for info in generic_infos {
        let mut is_duplicate = false;
        for loc in &info.save_locations {
            for existing in &save_locations {
                let existing_str = existing.path.to_string_lossy().to_lowercase();
                let loc_str = loc.path.to_string_lossy().to_lowercase();
                if existing_str.contains(&loc_str) || loc_str.contains(&existing_str) {
                    is_duplicate = true;
                    break;
                }
            }
            if is_duplicate { break; }
        }
        
        if !is_duplicate {
            save_locations.extend(info.save_locations);
            games.push(info.game);
        } else {
            tracing::info!("Jogo genérico '{}' ignorado por sobrepor caminho de jogo oficial.", info.game.name);
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
