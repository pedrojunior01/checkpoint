use super::resolver::{resolve_paths, PathContext};
use super::types::GameEntry;
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Detector de saves via Ludusavi Manifest.
pub struct ManifestDetector {
    /// Índice: steam_app_id → nome do jogo no manifest.
    steam_index: HashMap<u64, String>,
    /// Jogos do manifest, indexados pelo nome.
    games: HashMap<String, GameEntry>,
}

impl ManifestDetector {
    /// Carrega e indexa o manifest a partir de um arquivo YAML no disco.
    pub fn load(manifest_path: &Path) -> Result<Self> {
        let size_mb = std::fs::metadata(manifest_path)?.len() as f64 / 1_048_576.0;
        info!("Carregando manifest ({:.1} MB)...", size_mb);

        let content = std::fs::read_to_string(manifest_path)?;

        // Parsear primeiro como mapa de Values para ter fallback por jogo
        let raw: HashMap<String, serde_yaml::Value> = serde_yaml::from_str(&content)?;

        let mut games: HashMap<String, GameEntry> = HashMap::new();
        let mut steam_index: HashMap<u64, String> = HashMap::new();
        let mut skipped = 0usize;

        for (name, value) in raw {
            match serde_yaml::from_value::<GameEntry>(value) {
                Ok(entry) => {
                    // Indexar pelo Steam AppID se disponível
                    if let Some(steam_id) = entry.steam.as_ref().and_then(|s| s.id) {
                        steam_index.insert(steam_id, name.clone());
                    }
                    games.insert(name, entry);
                }
                Err(_) => {
                    skipped += 1;
                    debug!("Entrada ignorada no manifest: {}", name);
                }
            }
        }

        info!(
            "Manifest carregado: {} jogos indexados ({} com Steam AppID, {} ignorados).",
            games.len(),
            steam_index.len(),
            skipped
        );

        Ok(Self { steam_index, games })
    }

    /// Retorna os caminhos de save para um jogo Steam, resolvendo placeholders.
    ///
    /// Apenas inclui caminhos com tag "save" que se aplicam a Windows + Steam.
    /// Caminhos que não existem no sistema são incluídos mesmo assim (o jogo
    /// pode não ter saves ainda).
    pub fn find_save_paths(&self, steam_app_id: u64, ctx: &PathContext) -> Vec<PathBuf> {
        let game_name = match self.steam_index.get(&steam_app_id) {
            Some(n) => n,
            None => return vec![],
        };

        let entry = match self.games.get(game_name) {
            Some(e) => e,
            None => return vec![],
        };

        let mut paths = Vec::new();

        for (template, file_entry) in &entry.files {
            if !file_entry.has_save_tag() {
                continue;
            }
            if !file_entry.is_applicable_windows_steam() {
                continue;
            }
            let resolved_list = resolve_paths(template, ctx);
            for resolved in resolved_list {
                debug!("[{}] {} → {}", game_name, template, resolved.display());
                paths.push(resolved);
            }
        }

        if !paths.is_empty() {
            info!(
                "Manifest: {} caminho(s) de save para '{}' (AppID {}).",
                paths.len(),
                game_name,
                steam_app_id
            );
        }

        paths
    }

    /// Retorna o número de jogos com Steam AppID indexados.
    pub fn indexed_count(&self) -> usize {
        self.steam_index.len()
    }
}
