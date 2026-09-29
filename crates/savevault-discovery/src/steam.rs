//! Detector do Steam: lê libraryfolders.vdf e lista jogos instalados.

use anyhow::Result;
use savevault_core::models::{Confidence, Game, Platform};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// Informações completas de um jogo Steam detectado.
pub struct SteamGameInfo {
    pub game: Game,
    /// Caminho completo do diretório de instalação do jogo.
    pub install_path: Option<PathBuf>,
}

pub struct SteamDetector {
    steam_root: Option<PathBuf>,
}

impl Default for SteamDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl SteamDetector {
    pub fn new() -> Self {
        Self {
            steam_root: find_steam_root(),
        }
    }

    /// Cria detector com root explícito (útil para testes unitários ou fixtures).
    pub fn with_root(steam_root: Option<PathBuf>) -> Self {
        Self { steam_root }
    }

    /// Detecta jogos Steam com seus caminhos de instalação.
    pub async fn detect_with_paths(&self) -> Result<Vec<SteamGameInfo>> {
        let root = match &self.steam_root {
            Some(r) => r,
            None => {
                warn!("Steam não encontrado no sistema.");
                return Ok(vec![]);
            }
        };

        debug!("Steam root: {}", root.display());
        let library_folders = root.join("steamapps").join("libraryfolders.vdf");

        if !library_folders.exists() {
            warn!(
                "libraryfolders.vdf não encontrado em {}",
                library_folders.display()
            );
            return Ok(vec![]);
        }

        let content = tokio::fs::read_to_string(&library_folders).await?;
        let library_paths = parse_library_folders(&content, root);

        let mut results = Vec::new();
        for lib_path in library_paths {
            let apps_dir = lib_path.join("steamapps");
            if !apps_dir.exists() {
                continue;
            }

            let mut entries = tokio::fs::read_dir(&apps_dir).await?;
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                if name.starts_with("appmanifest_") && name.ends_with(".acf") {
                    if let Ok(acf_content) = tokio::fs::read_to_string(&path).await {
                        if let Some(info) = parse_acf(&acf_content, &lib_path) {
                            debug!(
                                "Steam jogo encontrado: {} (AppID: {:?}, instalado em: {:?})",
                                info.game.name, info.game.platform_id, info.install_path
                            );
                            results.push(info);
                        }
                    }
                }
            }
        }

        Ok(results)
    }

    /// Detecta apenas os Games (sem install path) — compatibilidade com testes existentes.
    pub async fn detect(&self) -> Result<Vec<Game>> {
        Ok(self
            .detect_with_paths()
            .await?
            .into_iter()
            .map(|i| i.game)
            .collect())
    }
}

/// Localiza a pasta raiz do Steam no Windows consultando:
/// 1. Variável de ambiente `STEAM_ROOT`
/// 2. Registro do Windows (HKCU e HKLM)
/// 3. Caminhos de instalação padrão
fn find_steam_root() -> Option<PathBuf> {
    // 1. Variável de ambiente (override explícito ou testes)
    if let Ok(path) = std::env::var("STEAM_ROOT") {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }

    // 2. Registro do Windows
    #[cfg(windows)]
    {
        // HKCU\Software\Valve\Steam -> SteamPath
        if let Some(path_str) = query_registry_value("HKCU\\Software\\Valve\\Steam", "SteamPath") {
            let p = PathBuf::from(path_str.replace('/', "\\"));
            if p.exists() {
                return Some(p);
            }
        }

        // HKLM\SOFTWARE\WOW6432Node\Valve\Steam -> InstallPath
        if let Some(path_str) =
            query_registry_value("HKLM\\SOFTWARE\\WOW6432Node\\Valve\\Steam", "InstallPath")
        {
            let p = PathBuf::from(path_str);
            if p.exists() {
                return Some(p);
            }
        }

        // HKLM\SOFTWARE\Valve\Steam -> InstallPath
        if let Some(path_str) = query_registry_value("HKLM\\SOFTWARE\\Valve\\Steam", "InstallPath")
        {
            let p = PathBuf::from(path_str);
            if p.exists() {
                return Some(p);
            }
        }
    }

    // 3. Caminhos padrão do sistema de arquivos
    let common_paths = [
        "C:\\Program Files (x86)\\Steam",
        "C:\\Program Files\\Steam",
        "D:\\Steam",
        "E:\\Steam",
    ];

    for &path in &common_paths {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }

    None
}

/// Consulta o registro do Windows via utilitário nativo `reg query`.
#[cfg(windows)]
pub fn query_registry_value(key: &str, value: &str) -> Option<String> {
    use std::process::Command;
    let output = Command::new("reg")
        .args(["query", key, "/v", value])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.to_lowercase().starts_with(&value.to_lowercase()) {
            if let Some(pos) = line.find("REG_SZ") {
                let data = line[pos + 6..].trim();
                if !data.is_empty() {
                    return Some(data.to_string());
                }
            } else if let Some(pos) = line.find("REG_EXPAND_SZ") {
                let data = line[pos + 13..].trim();
                if !data.is_empty() {
                    return Some(data.to_string());
                }
            }
        }
    }
    None
}

#[cfg(not(windows))]
pub fn query_registry_value(_key: &str, _value: &str) -> Option<String> {
    None
}

/// Parse simples do libraryfolders.vdf para extrair caminhos de bibliotecas.
/// Formato VDF (KeyValues): `"path"   "C:\\Games\\Steam"`
fn parse_library_folders(content: &str, steam_root: &Path) -> Vec<PathBuf> {
    let mut paths = vec![steam_root.to_path_buf()];

    for line in content.lines() {
        let line = line.trim();
        if line.to_lowercase().contains("\"path\"") {
            let parts: Vec<&str> = line.splitn(4, '"').collect();
            if parts.len() >= 4 {
                let raw = parts[3].trim_end_matches('"');
                let path = PathBuf::from(raw.replace("\\\\", "\\"));
                if path.exists() && !paths.contains(&path) {
                    paths.push(path);
                }
            }
        }
    }

    paths
}

/// Parse de um arquivo .acf para extrair nome, AppID e diretório de instalação.
fn parse_acf(content: &str, library_path: &Path) -> Option<SteamGameInfo> {
    let mut name: Option<String> = None;
    let mut app_id: Option<String> = None;
    let mut install_dir: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();
        let parts: Vec<&str> = line.splitn(4, '"').collect();
        if parts.len() < 4 {
            continue;
        }
        let key = parts[1].to_lowercase();
        let value = parts[3].trim_end_matches('"');
        match key.as_str() {
            "name" => name = Some(value.to_string()),
            "appid" => app_id = Some(value.to_string()),
            "installdir" => install_dir = Some(value.to_string()),
            _ => {}
        }
    }

    let name = name?;

    // Caminho completo: {library}/steamapps/common/{installdir}
    let install_path =
        install_dir.map(|dir| library_path.join("steamapps").join("common").join(dir));

    let game = Game::new(name, Platform::Steam, app_id, Confidence::High);
    Some(SteamGameInfo { game, install_path })
}
