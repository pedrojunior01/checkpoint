//! Generic detector: heurística segura para jogos no Windows.
//! Detecta jogos através do diretório oficial do Windows `%USERPROFILE%\Saved Games`.
//! Princípio: preferir falsos negativos a falsos positivos (seção 10 do spec).

use anyhow::Result;
use savevault_core::models::{Confidence, Game, Platform};
use std::path::{Path, PathBuf};

/// Informações de um jogo genérico detectado e sua pasta de save.
#[derive(Debug, Clone)]
pub struct GenericGameInfo {
    pub game: Game,
    pub save_path: PathBuf,
}

#[derive(Default)]
pub struct GenericDetector {
    custom_root: Option<PathBuf>,
}

impl GenericDetector {
    pub fn new() -> Self {
        Self { custom_root: None }
    }

    /// Permite configurar pasta raiz customizada para testes.
    pub fn with_root(root: Option<PathBuf>) -> Self {
        Self { custom_root: root }
    }

    /// Detecta jogos em `%USERPROFILE%\Saved Games` retornando seus metadados e caminhos de save.
    pub async fn detect_with_paths(&self) -> Result<Vec<GenericGameInfo>> {
        let saved_games_dir = if let Some(ref root) = self.custom_root {
            root.clone()
        } else {
            let userprofile = std::env::var("USERPROFILE").unwrap_or_default();
            if userprofile.is_empty() {
                return Ok(vec![]);
            }
            PathBuf::from(userprofile).join("Saved Games")
        };

        if !saved_games_dir.exists() {
            return Ok(vec![]);
        }

        let mut results = Vec::new();
        let mut entries = match tokio::fs::read_dir(&saved_games_dir).await {
            Ok(e) => e,
            Err(_) => return Ok(vec![]),
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir() && has_any_files(&path).await {
                let game_name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .trim()
                    .to_string();

                if !game_name.is_empty() {
                    let game = Game::new(game_name, Platform::Generic, None, Confidence::Medium);
                    results.push(GenericGameInfo {
                        game,
                        save_path: path,
                    });
                }
            }
        }

        Ok(results)
    }

    /// Varre `%USERPROFILE%\Saved Games` e raízes configuradas buscando pastas de jogos que contenham dados de save.
    pub async fn detect(&self) -> Result<Vec<Game>> {
        Ok(self
            .detect_with_paths()
            .await?
            .into_iter()
            .map(|info| info.game)
            .collect())
    }
}

async fn has_any_files(dir: &Path) -> bool {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return false,
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.path().is_file() {
            return true;
        }
        if entry.path().is_dir() {
            if let Ok(mut sub) = tokio::fs::read_dir(entry.path()).await {
                while let Ok(Some(sub_entry)) = sub.next_entry().await {
                    if sub_entry.path().is_file() {
                        return true;
                    }
                }
            }
        }
    }
    false
}
