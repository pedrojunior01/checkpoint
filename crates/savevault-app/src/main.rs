//! SaveVault — entry point
//!
//! Inicializa configuração, banco de dados, executa discovery e
//! inicia o monitoramento contínuo de saves.

use anyhow::Result;
use savevault_core::config::Config;
use savevault_db::Database;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // Configurar logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("SAVEVAULT_LOG")
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    info!("SaveVault iniciando...");

    // Carregar configuração
    let config_path = {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
        std::path::PathBuf::from(appdata).join("SaveVault").join("config.toml")
    };

    let config = Config::load(&config_path).unwrap_or_else(|e| {
        tracing::warn!("Não foi possível carregar config ({}), usando padrões.", e);
        Config::default()
    });

    // Salvar config padrão se não existir
    if !config_path.exists() {
        if let Err(e) = config.save(&config_path) {
            tracing::warn!("Não foi possível salvar config: {}", e);
        }
    }

    info!("Config carregada: max_versions={}, log_level={}", config.general.max_versions, config.general.log_level);

    // Abrir banco de dados
    let db = match Database::open(&config.paths.database_path) {
        Ok(d) => {
            info!("Banco de dados aberto: {}", config.paths.database_path.display());
            d
        }
        Err(e) => {
            error!("Falha ao abrir banco de dados: {}", e);
            return Err(e);
        }
    };

    // Discovery: encontrar jogos e emuladores
    info!("Iniciando discovery...");
    let discovery = savevault_discovery::run_all().await?;
    info!("Discovery concluído: {} jogo(s) encontrado(s)", discovery.games.len());

    // Persistir jogos descobertos no banco
    let games_repo = savevault_db::repositories::GamesRepo::new(&db.conn);
    for game in &discovery.games {
        if let Err(e) = games_repo.insert(game) {
            tracing::warn!("Não foi possível salvar jogo '{}': {}", game.name, e);
        }
    }

    // TODO v0.3: iniciar monitoramento contínuo com FileWatcher
    info!("SaveVault v0.1 — monitoramento contínuo será implementado no v0.3.");
    info!("Encerrando. Para o MVP completo, aguarde os próximos milestones.");

    Ok(())
}
