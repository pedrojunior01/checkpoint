//! savevault-discovery
//!
//! Detecção de jogos instalados (Steam, GOG, emuladores) e localização de seus saves.

pub mod steam;
pub mod gog;
pub mod emulators;
pub mod generic;

use savevault_core::models::Game;
use anyhow::Result;

/// Resultado de uma operação de discovery.
#[derive(Debug)]
pub struct DiscoveryResult {
    pub games: Vec<Game>,
}

/// Executa todos os detectors disponíveis e retorna os jogos encontrados.
pub async fn run_all() -> Result<DiscoveryResult> {
    let mut games = Vec::new();

    // Steam
    match steam::SteamDetector::new().detect().await {
        Ok(mut found) => {
            tracing::info!("Steam: {} jogo(s) encontrado(s)", found.len());
            games.append(&mut found);
        }
        Err(e) => tracing::warn!("Steam detector falhou: {}", e),
    }

    // GOG
    match gog::GogDetector::new().detect().await {
        Ok(mut found) => {
            tracing::info!("GOG: {} jogo(s) encontrado(s)", found.len());
            games.append(&mut found);
        }
        Err(e) => tracing::warn!("GOG detector falhou: {}", e),
    }

    // Emuladores
    match emulators::EmulatorDetector::new().detect().await {
        Ok(mut found) => {
            tracing::info!("Emuladores: {} entrada(s) encontrada(s)", found.len());
            games.append(&mut found);
        }
        Err(e) => tracing::warn!("Emulator detector falhou: {}", e),
    }

    Ok(DiscoveryResult { games })
}

#[cfg(test)]
mod tests;
