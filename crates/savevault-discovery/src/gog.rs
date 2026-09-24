//! Detector GOG: verifica instalações via pasta padrão do GOG Galaxy.
//! TODO v0.4: ler registry HKLM\SOFTWARE\GOG.com\Games para lista completa.

use anyhow::Result;
use savevault_core::models::Game;
use std::path::PathBuf;
use tracing::{debug, warn};

pub struct GogDetector {
    gog_root: Option<PathBuf>,
}

impl GogDetector {
    pub fn new() -> Self {
        Self {
            gog_root: find_gog_root(),
        }
    }

    pub async fn detect(&self) -> Result<Vec<Game>> {
        let root = match &self.gog_root {
            Some(r) => r,
            None => {
                warn!("GOG Galaxy não encontrado no sistema.");
                return Ok(vec![]);
            }
        };

        debug!("GOG root: {}", root.display());
        // Placeholder — implementação completa via registry no v0.4
        Ok(vec![])
    }
}

fn find_gog_root() -> Option<PathBuf> {
    let default = PathBuf::from("C:\\Program Files (x86)\\GOG Galaxy");
    if default.exists() {
        return Some(default);
    }
    None
}
