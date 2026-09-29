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
    /// Provedor ativo: "google_drive" (MVP). Outros pós-MVP.
    pub provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathsConfig {
    /// Diretório de backups locais (snapshots antes do upload).
    pub backup_dir: PathBuf,
    /// Caminho do banco SQLite.
    pub database_path: PathBuf,
    /// Diretório raiz sincronizado pelo Google Drive para computador (opcional).
    #[serde(default)]
    pub drive_dir: Option<PathBuf>,
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
                provider: "google_drive".into(),
            },
            paths: PathsConfig {
                backup_dir: base.join("backups"),
                database_path: base.join("savevault.db"),
                drive_dir: None,
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
    /// Carrega a configuração de um arquivo TOML. Se não existir ou estiver vazio (0 bytes), retorna o default.
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            if content.trim().is_empty() {
                return Ok(Config::default());
            }
            let config: Config = toml::from_str(&content)?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    /// Salva a configuração atomicamente em um arquivo TOML usando arquivo temporário + sync_all + rename.
    pub fn save(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
        std::fs::create_dir_all(parent)?;

        let content = toml::to_string_pretty(self)?;
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("config.toml");
        let tmp_file_name = format!(".{}.tmp.{}", file_name, uuid::Uuid::new_v4());
        let tmp_path = parent.join(tmp_file_name);

        // Escreve no arquivo temporário e força sincronização em disco
        {
            use std::io::Write;
            let mut file = std::fs::File::create(&tmp_path)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
        }

        // Renomeação atômica (no Windows usa MoveFileExW com REPLACE_EXISTING)
        if let Err(e) = std::fs::rename(&tmp_path, path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e.into());
        }

        Ok(())
    }
}
