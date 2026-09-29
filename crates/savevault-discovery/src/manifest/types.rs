use serde::Deserialize;
use std::collections::HashMap;

/// Entrada de um jogo no Ludusavi Manifest.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct GameEntry {
    pub files: HashMap<String, FileEntry>,
    pub steam: Option<SteamInfo>,
    pub gog: Option<GogInfo>,
}

/// Uma regra de arquivo de save dentro de um jogo.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct FileEntry {
    pub tags: Vec<String>,
    pub when: Vec<WhenCondition>,
}

impl FileEntry {
    /// Retorna true se esta entrada tem a tag "save".
    pub fn has_save_tag(&self) -> bool {
        self.tags.iter().any(|t| t == "save")
    }

    /// Retorna true se esta entrada se aplica a Windows (independente de store ou genérico).
    pub fn is_applicable_windows(&self) -> bool {
        if self.when.is_empty() {
            return true;
        }
        self.when.iter().any(|c| {
            c.os.as_deref().map(|o| o == "windows").unwrap_or(true)
        })
    }

    /// Retorna true se esta entrada se aplica a Windows + Steam.
    pub fn is_applicable_windows_steam(&self) -> bool {
        if self.when.is_empty() {
            return true;
        }
        self.when.iter().any(|c| {
            let os_ok = c.os.as_deref().map(|o| o == "windows").unwrap_or(true);
            let store_ok = c.store.as_deref().map(|s| s == "steam").unwrap_or(true);
            os_ok && store_ok
        })
    }

    /// Retorna true se esta entrada se aplica a Windows + GOG.
    pub fn is_applicable_windows_gog(&self) -> bool {
        if self.when.is_empty() {
            return true;
        }
        self.when.iter().any(|c| {
            let os_ok = c.os.as_deref().map(|o| o == "windows").unwrap_or(true);
            let store_ok = c.store.as_deref().map(|s| s == "gog").unwrap_or(true);
            os_ok && store_ok
        })
    }
}

/// Condição de aplicabilidade de uma regra.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct WhenCondition {
    pub os: Option<String>,
    pub store: Option<String>,
}

/// Informações Steam de um jogo no manifest.
#[derive(Debug, Deserialize)]
pub struct SteamInfo {
    pub id: Option<u64>,
}

/// Informações GOG de um jogo no manifest.
#[derive(Debug, Deserialize)]
pub struct GogInfo {
    pub id: Option<u64>,
}
