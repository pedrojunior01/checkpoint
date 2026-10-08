//! Detector GOG Galaxy: detecta jogos instalados via Registro do Windows.
//!
//! ## Estratégia de detecção (em ordem de prioridade):
//!
//! 1. **Registry principal** (`HKLM\SOFTWARE\GOG.com\Games`):
//!    Cada subchave é um jogo com entradas como `GAMENAME`, `INSTALLPATH`,
//!    `GAMEEXECUTABLE`, `productId`, etc.
//!
//! 2. **Registry WoW64** (`HKLM\SOFTWARE\WOW6432Node\GOG.com\Games`):
//!    Usado quando o GOG Galaxy (32 bits) está instalado em Windows 64 bits.
//!
//! 3. **Fallback por pasta** (`C:\Program Files (x86)\GOG Galaxy\Games\`):
//!    Caso o Registro não esteja acessível, verifica se a pasta padrão existe.

use anyhow::Result;
use savevault_core::models::{Confidence, Game, Platform, SaveLocation, SaveLocationType};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
use uuid::Uuid;
use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};

// Chaves de registro onde o GOG armazena seus jogos instalados
const GOG_REGISTRY_PATHS: &[&str] = &[
    r"SOFTWARE\GOG.com\Games",
    r"SOFTWARE\WOW6432Node\GOG.com\Games",
];

// Caminhos padrão de instalação do GOG Galaxy
const GOG_DEFAULT_DIRS: &[&str] = &[
    r"C:\Program Files (x86)\GOG Galaxy\Games",
    r"C:\Program Files\GOG Galaxy\Games",
    r"C:\GOG Games",
];

/// Resultado completo de um jogo GOG detectado.
pub struct GogGameInfo {
    pub game: Game,
    /// Caminho de instalação do jogo (usado para resolver saves).
    pub install_path: Option<PathBuf>,
    /// Caminhos de save associados (se detectáveis via registry).
    pub save_paths: Vec<SaveLocation>,
}

pub struct GogDetector {
    gog_root: Option<PathBuf>,
}

impl Default for GogDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl GogDetector {
    pub fn new() -> Self {
        Self {
            gog_root: find_gog_root(),
        }
    }

    /// Detecta jogos GOG retornando apenas a lista de `Game` (compatível com o orchestrator).
    pub async fn detect(&self) -> Result<Vec<Game>> {
        let infos = self.detect_with_paths().await?;
        Ok(infos.into_iter().map(|i| i.game).collect())
    }

    /// Detecta jogos GOG com caminhos de instalação e saves.
    pub async fn detect_with_paths(&self) -> Result<Vec<GogGameInfo>> {
        let mut results = Vec::new();

        // Tenta ler o Registro primeiro (método principal)
        let from_registry = read_gog_games_from_registry();
        if !from_registry.is_empty() {
            info!(
                "GOG: {} jogo(s) detectado(s) via Registro do Windows.",
                from_registry.len()
            );
            return Ok(from_registry);
        }

        // Fallback: varre a pasta de instalação do GOG Galaxy
        if let Some(ref root) = self.gog_root {
            debug!(
                "GOG: Registro não encontrado, varrendo pasta: {}",
                root.display()
            );
            results.extend(scan_gog_games_folder(root));
        } else {
            warn!("GOG Galaxy não encontrado no sistema (Registry vazio e pasta ausente).");
        }

        Ok(results)
    }
}

// ---------------------------------------------------------------------------
// Leitura do Registro do Windows
// ---------------------------------------------------------------------------

/// Lê todas as subchaves de `HKLM\SOFTWARE\GOG.com\Games` (e a variante WoW64)
/// e retorna informações completas de cada jogo instalado.
fn read_gog_games_from_registry() -> Vec<GogGameInfo> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let mut results = Vec::new();

    for reg_path in GOG_REGISTRY_PATHS {
        let gog_key = match hklm.open_subkey(reg_path) {
            Ok(k) => k,
            Err(_) => continue, // Esta chave não existe neste sistema
        };

        for subkey_name in gog_key.enum_keys().filter_map(|r| r.ok()) {
            let game_key = match gog_key.open_subkey(&subkey_name) {
                Ok(k) => k,
                Err(_) => continue,
            };

            // Lê os campos principais da subchave do jogo
            let name: String = game_key
                .get_value("GAMENAME")
                .or_else(|_| game_key.get_value("gameName"))
                .or_else(|_| game_key.get_value("GameName"))
                .unwrap_or_else(|_| format!("GOG Game {}", subkey_name));

            let install_path: Option<PathBuf> = game_key
                .get_value::<String, _>("INSTALLPATH")
                .or_else(|_| game_key.get_value("installPath"))
                .ok()
                .map(PathBuf::from)
                .filter(|p| p.exists());

            // product ID como identificador da plataforma (suporta String e DWORD numérico)
            let product_id: Option<String> = game_key
                .get_value::<String, _>("PRODUCTID")
                .or_else(|_| game_key.get_value::<String, _>("productId"))
                .or_else(|_| game_key.get_value::<String, _>("gameID"))
                .ok()
                .or_else(|| {
                    game_key
                        .get_value::<u32, _>("gameID")
                        .or_else(|_| game_key.get_value::<u32, _>("productId"))
                        .or_else(|_| game_key.get_value::<u32, _>("PRODUCTID"))
                        .map(|id| id.to_string())
                        .ok()
                })
                .or_else(|| {
                    if subkey_name.chars().all(|c| c.is_ascii_digit()) {
                        Some(subkey_name.clone())
                    } else {
                        None
                    }
                });

            if name.trim().is_empty() {
                debug!("GOG: subchave '{}' sem nome, ignorando.", subkey_name);
                continue;
            }

            debug!(
                "GOG: detectado '{}' (id: {:?}) em {:?}",
                name, product_id, install_path
            );

            let game = Game::new(
                name.trim().to_string(),
                Platform::Gog,
                product_id,
                Confidence::High,
            );

            // Tenta detectar saves associados via registry (campo `WORKINGDIR` ou `SAVEPATH`)
            let save_paths = detect_save_paths_from_registry(&game_key, game.id, &install_path);

            results.push(GogGameInfo {
                game,
                install_path,
                save_paths,
            });
        }

        // Se encontrou jogos nesta chave, não precisa tentar a outra (WoW64)
        if !results.is_empty() {
            break;
        }
    }

    results
}

/// Tenta extrair caminhos de save de campos opcionais do registro do GOG.
/// O GOG não tem um campo padrão de save, mas alguns jogos registram `WORKINGDIR`.
fn detect_save_paths_from_registry(
    game_key: &RegKey,
    game_id: Uuid,
    install_path: &Option<PathBuf>,
) -> Vec<SaveLocation> {
    let mut paths = Vec::new();

    // Alguns instaladores registram um campo SAVEPATH ou SAVEDIR
    for field in &["SAVEPATH", "SAVEDIR", "WORKINGDIR"] {
        if let Ok(raw_path) = game_key.get_value::<String, _>(field) {
            let p = PathBuf::from(&raw_path);
            if p.exists() && p.is_dir() {
                paths.push(SaveLocation::new(game_id, p, SaveLocationType::Directory));
            }
        }
    }

    // Se não achou campos de save mas tem install_path, verifica subpastas típicas
    if paths.is_empty() {
        if let Some(ref base) = install_path {
            for subfolder in &["saves", "save", "Saves", "Save", "savegames", "SaveGames"] {
                let candidate = base.join(subfolder);
                if candidate.exists() {
                    paths.push(SaveLocation::new(
                        game_id,
                        candidate,
                        SaveLocationType::Directory,
                    ));
                    break;
                }
            }
        }
    }

    paths
}

// ---------------------------------------------------------------------------
// Fallback: varredura da pasta de instalação
// ---------------------------------------------------------------------------

/// Varre a pasta de instalação do GOG Galaxy buscando subpastas (um jogo por pasta).
fn scan_gog_games_folder(root: &PathBuf) -> Vec<GogGameInfo> {
    let mut results = Vec::new();

    let read_dir = match std::fs::read_dir(root) {
        Ok(d) => d,
        Err(e) => {
            warn!("GOG: Falha ao ler pasta '{}': {}", root.display(), e);
            return results;
        }
    };

    for entry in read_dir.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("GOG Game")
            .to_string();

        // Ignora pastas do próprio GOG Galaxy (não são jogos)
        if name.to_lowercase().contains("gog galaxy") || name.to_lowercase().contains("redist") {
            continue;
        }

        debug!("GOG (fallback): possível jogo em '{}'", path.display());

        let game = Game::new(name, Platform::Gog, None, Confidence::Low);
        let game_id = game.id;
        let save_paths = detect_save_paths_in_folder(game_id, &path);

        results.push(GogGameInfo {
            game,
            install_path: Some(path),
            save_paths,
        });
    }

    results
}

/// Detecta save folders dentro de um diretório de instalação de jogo.
fn detect_save_paths_in_folder(game_id: Uuid, base: &Path) -> Vec<SaveLocation> {
    let mut paths = Vec::new();
    for subfolder in &["saves", "save", "Saves", "Save", "savegames", "SaveGames"] {
        let candidate = base.join(subfolder);
        if candidate.exists() {
            paths.push(SaveLocation::new(
                game_id,
                candidate,
                SaveLocationType::Directory,
            ));
            break;
        }
    }
    paths
}

// ---------------------------------------------------------------------------
// Detecção da pasta raiz do GOG Galaxy
// ---------------------------------------------------------------------------

pub fn find_gog_root() -> Option<PathBuf> {
    // Primeiro tenta extrair do registro a pasta de instalação do Galaxy
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    for reg_path in &[
        r"SOFTWARE\GOG.com\GalaxyClient",
        r"SOFTWARE\WOW6432Node\GOG.com\GalaxyClient",
    ] {
        if let Ok(key) = hklm.open_subkey(reg_path) {
            if let Ok(path) = key.get_value::<String, _>("client") {
                let p = PathBuf::from(path);
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }

    // Fallback: pastas padrão
    for dir in GOG_DEFAULT_DIRS {
        let p = PathBuf::from(dir);
        if p.exists() {
            return Some(p);
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Testes
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn gog_detector_returns_ok_even_without_gog_installed() {
        // Não importa se o GOG está instalado ou não — nunca deve panicar
        let detector = GogDetector::new();
        let result = detector.detect().await;
        assert!(
            result.is_ok(),
            "GogDetector::detect() não deve retornar Err"
        );
    }

    #[tokio::test]
    async fn gog_detect_with_paths_returns_ok() {
        let detector = GogDetector::new();
        let result = detector.detect_with_paths().await;
        assert!(result.is_ok());
    }

    #[test]
    fn scan_gog_games_folder_nonexistent_path_returns_empty() {
        let fake_path = PathBuf::from(r"C:\path\that\does\not\exist\gog");
        let result = scan_gog_games_folder(&fake_path);
        assert!(result.is_empty());
    }

    #[test]
    fn detect_save_paths_in_folder_finds_save_subfolder() {
        let dir = tempfile::tempdir().unwrap();
        let saves_dir = dir.path().join("saves");
        std::fs::create_dir_all(&saves_dir).unwrap();

        let game_id = Uuid::new_v4();
        let paths = detect_save_paths_in_folder(game_id, dir.path());

        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].path, saves_dir);
        assert_eq!(paths[0].game_id, game_id);
    }

    #[test]
    fn detect_save_paths_in_folder_returns_empty_when_no_save_subfolder() {
        let dir = tempfile::tempdir().unwrap();
        let game_id = Uuid::new_v4();
        let paths = detect_save_paths_in_folder(game_id, dir.path());
        assert!(paths.is_empty());
    }

    #[test]
    fn find_gog_root_returns_none_or_existing_path() {
        // Só valida que a função não entra em pânico e retorna um resultado coerente
        let root = find_gog_root();
        if let Some(ref p) = root {
            assert!(p.exists(), "find_gog_root() retornou caminho inexistente");
        }
        // None também é válido (GOG não instalado)
    }
}
