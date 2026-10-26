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
    /// Índice: gog_id → nome do jogo no manifest.
    gog_index: HashMap<u64, String>,
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
        let mut gog_index: HashMap<u64, String> = HashMap::new();
        let mut skipped = 0usize;

        for (name, value) in raw {
            match serde_yaml::from_value::<GameEntry>(value) {
                Ok(entry) => {
                    // Indexar pelo Steam AppID se disponível
                    if let Some(steam_id) = entry.steam.as_ref().and_then(|s| s.id) {
                        steam_index.insert(steam_id, name.clone());
                    }
                    // Indexar pelo GOG ID se disponível
                    if let Some(gog_id) = entry.gog.as_ref().and_then(|g| g.id) {
                        gog_index.insert(gog_id, name.clone());
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
            "Manifest carregado: {} jogos indexados ({} Steam, {} GOG, {} ignorados).",
            games.len(),
            steam_index.len(),
            gog_index.len(),
            skipped
        );

        Ok(Self {
            steam_index,
            gog_index,
            games,
        })
    }

    /// Retorna os caminhos de save para um jogo Steam, resolvendo placeholders.
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

    /// Retorna os caminhos de save para um jogo GOG, resolvendo placeholders.
    pub fn find_save_paths_gog(
        &self,
        gog_id: Option<u64>,
        title: &str,
        ctx: &PathContext,
    ) -> Vec<PathBuf> {
        // Tenta achar por GOG ID primeiro
        let game_entry = gog_id
            .and_then(|id| self.gog_index.get(&id))
            .and_then(|name| self.games.get(name))
            .or_else(|| self.find_entry_by_title(title));

        let entry = match game_entry {
            Some(e) => e,
            None => return vec![],
        };

        let mut paths = Vec::new();

        for (template, file_entry) in &entry.files {
            if !file_entry.has_save_tag() {
                continue;
            }
            if !file_entry.is_applicable_windows_gog() && !file_entry.is_applicable_windows() {
                continue;
            }
            let resolved_list = resolve_paths(template, ctx);
            for resolved in resolved_list {
                debug!("[GOG: {}] {} → {}", title, template, resolved.display());
                paths.push(resolved);
            }
        }

        if !paths.is_empty() {
            info!(
                "Manifest: {} caminho(s) de save para GOG '{}'.",
                paths.len(),
                title
            );
        }

        paths
    }

    /// Busca saves no manifest pelo título do jogo (para jogos genéricos ou sem AppID).
    pub fn find_save_paths_by_title(&self, title: &str, ctx: &PathContext) -> Vec<PathBuf> {
        let entry = match self.find_entry_by_title(title) {
            Some(e) => e,
            None => return vec![],
        };

        let mut paths = Vec::new();

        for (template, file_entry) in &entry.files {
            if !file_entry.has_save_tag() || !file_entry.is_applicable_windows() {
                continue;
            }
            let resolved_list = resolve_paths(template, ctx);
            for resolved in resolved_list {
                paths.push(resolved);
            }
        }

        paths
    }

    fn find_entry_by_title(&self, query: &str) -> Option<&GameEntry> {
        if let Some(e) = self.games.get(query) {
            return Some(e);
        }
        self.games.iter().find_map(|(name, entry)| {
            if name.eq_ignore_ascii_case(query) {
                Some(entry)
            } else {
                None
            }
        })
    }

    /// Retorna o número de jogos com Steam AppID indexados.
    pub fn indexed_count(&self) -> usize {
        self.steam_index.len()
    }
}
