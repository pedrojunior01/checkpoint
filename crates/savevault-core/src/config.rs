use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuração principal do SaveVault (lida de `config.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub general: GeneralConfig,
    pub cloud: CloudConfig,
    pub paths: PathsConfig,
    #[serde(default)]
    pub notifications: NotificationsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationsConfig {
    /// Se notificações Toast estão ativadas globalmente.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Se deve notificar a cada save salvo localmente (padrão: false para não incomodar durante o jogo).
    #[serde(default)]
    pub notify_on_save: bool,
    /// Se deve notificar quando a sincronização na nuvem for concluída.
    #[serde(default = "default_true")]
    pub notify_on_sync: bool,
    /// Se deve notificar quando uma restauração for realizada.
    #[serde(default = "default_true")]
    pub notify_on_restore: bool,
    /// Intervalo mínimo (em segundos) entre notificações repetidas do mesmo tipo (cooldown anti-spam).
    #[serde(default = "default_cooldown_secs")]
    pub cooldown_secs: u64,
}

fn default_true() -> bool {
    true
}

fn default_cooldown_secs() -> u64 {
    300
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            notify_on_save: false,
            notify_on_sync: true,
            notify_on_restore: true,
            cooldown_secs: 300,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Preserva ambos os arquivos: mantém o save local e salva o snapshot divergente
    /// da nuvem em uma pasta de conflitos (`conflicts/`), evitando perda de dados (padrão seguro).
    #[default]
    KeepBoth,
    /// O arquivo com timestamp de modificação mais recente vence.
    PreferNewest,
    /// O arquivo do computador local tem prioridade.
    PreferLocal,
    /// O arquivo da nuvem tem prioridade.
    PreferRemote,
    /// Marca o conflito como pendente e aguarda resolução manual.
    Manual,
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
    /// Se a inicialização automática com o Windows está habilitada.
    #[serde(default)]
    pub autostart: bool,
    /// Nome deste dispositivo no ecossistema multi-PC
    #[serde(default = "default_device_name")]
    pub device_name: String,
    /// Política de resolução de conflitos entre múltiplos computadores
    #[serde(default)]
    pub conflict_policy: ConflictPolicy,
}

fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Windows-PC".into())
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
                autostart: false,
                device_name: default_device_name(),
                conflict_policy: ConflictPolicy::KeepBoth,
            },
            cloud: CloudConfig {
                provider: "local_folder".into(),
                local_sync_path: None,
            },
            paths: PathsConfig {
                backup_dir: base.join("backups"),
                database_path: base.join("savevault.db"),
            },
            notifications: NotificationsConfig::default(),
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
