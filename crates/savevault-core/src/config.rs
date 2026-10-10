use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuração principal do SaveVault (lida de `config.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub general: GeneralConfig,
    pub cloud: CloudConfig,
    pub paths: PathsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    /// Número máximo de versões mantidas por save.
    pub max_versions: u32,
    /// Nível de log: "trace" | "debug" | "info" | "warn" | "error"
    pub log_level: String,
    /// Tempo de espera (ms) para estabilização do arquivo após detecção de mudança.
    pub debounce_ms: u64,
    /// Intervalo (segundos) entre ciclos de reconciliação periódica.
    #[serde(default = "default_reconciliation_interval_secs")]
    pub reconciliation_interval_secs: u64,
}

fn default_reconciliation_interval_secs() -> u64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudConfig {
    /// Provedor ativo: "google_drive", "onedrive", "local_folder", "none"
    pub provider: String,
    /// Caminho da pasta de sincronização local da nuvem (ex.: C:\Users\...\OneDrive\SaveVault)
    #[serde(default)]
    pub local_sync_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathsConfig {
    /// Diretório de backups locais (snapshots antes do upload).
    pub backup_dir: PathBuf,
    /// Caminho do banco SQLite.
    pub database_path: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        let base = dirs_default();
        Self {
            general: GeneralConfig {
                max_versions: 10,
                log_level: "info".into(),
                debounce_ms: 2000,
                reconciliation_interval_secs: 30,
            },
            cloud: CloudConfig {
                provider: "local_folder".into(),
                local_sync_path: None,
            },
            paths: PathsConfig {
                backup_dir: base.join("backups"),
                database_path: base.join("savevault.db"),
            },
        }
    }
}

/// Retorna o diretório de dados padrão do SaveVault no Windows:
/// `%APPDATA%\SaveVault`
fn dirs_default() -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(appdata).join("SaveVault")
}

impl Config {
    /// Carrega a configuração de um arquivo TOML. Se não existir, retorna o default.
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            let config: Config = toml::from_str(&content)?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    /// Salva a configuração em um arquivo TOML.
    pub fn save(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}
