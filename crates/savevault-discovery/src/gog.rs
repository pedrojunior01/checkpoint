//! Detector GOG: descobre jogos instalados via Registro do Windows e arquivos de manifesto goggame-*.info.

use anyhow::Result;
use savevault_core::models::{Confidence, Game, Platform};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Informações de um jogo GOG detectado.
#[derive(Debug, Clone)]
pub struct GogGameInfo {
    pub game: Game,
    pub install_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct GogGameInfoJson {
    #[serde(rename = "gameId", default)]
    game_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

pub struct GogDetector {
    custom_root: Option<PathBuf>,
}

impl Default for GogDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl GogDetector {
    pub fn new() -> Self {
        Self { custom_root: None }
    }

    /// Permite especificar uma pasta raiz customizada para testes ou bibliotecas secundárias.
    pub fn with_root(root: Option<PathBuf>) -> Self {
        Self { custom_root: root }
    }

    /// Detecta jogos GOG com seus caminhos de instalação.
    pub async fn detect_with_paths(&self) -> Result<Vec<GogGameInfo>> {
        let mut results = Vec::new();
        let mut seen_ids = HashSet::new();

        // 1. Se um root customizado foi fornecido (ex: em testes), faz a varredura exclusiva dele
        if let Some(ref root) = self.custom_root {
            if root.exists() {
                scan_directory_for_gog_games(root, &mut results, &mut seen_ids).await;
            }
            return Ok(results);
        }

        // 2. Consulta o Registro do Windows
        #[cfg(windows)]
        {
            scan_registry_gog_games(&mut results, &mut seen_ids);
        }

        // 3. Varredura em pastas padrão de instalação de jogos GOG
        let candidate_dirs = get_gog_candidate_dirs();
        for dir in candidate_dirs {
            if dir.exists() {
                scan_directory_for_gog_games(&dir, &mut results, &mut seen_ids).await;
            }
        }

        debug!("GOG: Total de jogos encontrados: {}", results.len());
        Ok(results)
    }

    /// Detecta apenas a lista de `Game` (compatibilidade).
    pub async fn detect(&self) -> Result<Vec<Game>> {
        Ok(self
            .detect_with_paths()
            .await?
            .into_iter()
            .map(|i| i.game)
            .collect())
    }
}

/// Diretórios padrão onde jogos GOG costumam ser instalados
fn get_gog_candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Ok(env_root) = std::env::var("GOG_ROOT") {
        dirs.push(PathBuf::from(env_root));
    }

    let defaults = [
        "C:\\GOG Games",
        "D:\\GOG Games",
        "E:\\GOG Games",
        "C:\\Program Files (x86)\\GOG Galaxy\\Games",
        "C:\\Program Files\\GOG Galaxy\\Games",
    ];

    for path in defaults {
        let p = PathBuf::from(path);
        if p.exists() {
            dirs.push(p);
        }
    }

    dirs
}

/// Varre uma pasta de biblioteca procurando subpastas contendo `goggame-*.info`
async fn scan_directory_for_gog_games(
    dir: &Path,
    results: &mut Vec<GogGameInfo>,
    seen_ids: &mut HashSet<String>,
) {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let game_dir = entry.path();
        if !game_dir.is_dir() {
            continue;
        }

        if let Ok(mut sub_entries) = tokio::fs::read_dir(&game_dir).await {
            while let Ok(Some(sub_entry)) = sub_entries.next_entry().await {
                let filename = sub_entry.file_name().to_string_lossy().to_string();
                if filename.starts_with("goggame-") && filename.ends_with(".info") {
                    if let Ok(content) = tokio::fs::read_to_string(sub_entry.path()).await {
                        if let Ok(parsed) = serde_json::from_str::<GogGameInfoJson>(&content) {
                            let name = parsed.name.unwrap_or_else(|| {
                                game_dir
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .to_string()
                            });
                            let game_id = parsed.game_id.unwrap_or_else(|| {
                                filename
                                    .trim_start_matches("goggame-")
                                    .trim_end_matches(".info")
                                    .to_string()
                            });

                            if !seen_ids.contains(&game_id) {
                                seen_ids.insert(game_id.clone());
                                debug!(
                                    "GOG jogo encontrado via arquivo .info: {} ({})",
                                    name, game_id
                                );
                                results.push(GogGameInfo {
                                    game: Game::new(
                                        name,
                                        Platform::Gog,
                                        Some(game_id),
                                        Confidence::High,
                                    ),
                                    install_path: Some(game_dir.clone()),
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Consulta chaves de registro do GOG no Windows
#[cfg(windows)]
fn scan_registry_gog_games(results: &mut Vec<GogGameInfo>, seen_ids: &mut HashSet<String>) {
    use std::process::Command;

    let registry_keys = [
        "HKLM\\SOFTWARE\\WOW6432Node\\GOG.com\\Games",
        "HKLM\\SOFTWARE\\GOG.com\\Games",
    ];

    for reg_key in registry_keys {
        let output = match Command::new("reg").args(["query", reg_key]).output() {
            Ok(o) => o,
            Err(_) => continue,
        };

        if !output.status.success() {
            continue;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let subkey = line.trim();
            if !subkey.starts_with(reg_key) || subkey == reg_key {
                continue;
            }

            // Cada subchave representa um jogo instalado
            let game_id = subkey
                .split('\\')
                .next_back()
                .unwrap_or_default()
                .to_string();
            if game_id.is_empty() || seen_ids.contains(&game_id) {
                continue;
            }

            let name = crate::steam::query_registry_value(subkey, "gameName")
                .or_else(|| crate::steam::query_registry_value(subkey, "GAMENAME"));
            let path_str = crate::steam::query_registry_value(subkey, "path")
                .or_else(|| crate::steam::query_registry_value(subkey, "PATH"));

            if let Some(game_name) = name {
                seen_ids.insert(game_id.clone());
                let install_path = path_str.map(PathBuf::from);
                debug!(
                    "GOG jogo encontrado via Registro: {} (ID: {}, path: {:?})",
                    game_name, game_id, install_path
                );
                results.push(GogGameInfo {
                    game: Game::new(game_name, Platform::Gog, Some(game_id), Confidence::High),
                    install_path,
                });
            }
        }
    }
}
