use savevault_core::config::Config;
use savevault_db::{
    Database,
    repositories::{GamesRepo, SaveLocationsRepo},
};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Inicializar tracing
    let log_level = std::env::var("SAVEVAULT_LOG").unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&log_level)),
        )
        .init();

    tracing::info!("SaveVault iniciando...");

    // Carregar configuração
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    let config_dir = PathBuf::from(&appdata).join("SaveVault");
    let config_path = config_dir.join("config.toml");
    let config = Config::load(&config_path).unwrap_or_default();
    config.save(&config_path)?;

    tracing::info!(
        "Config carregada: max_versions={}, log_level={}",
        config.general.max_versions,
        config.general.log_level
    );

    // Abrir banco de dados
    let db = Database::open(&config.paths.database_path)?;
    let games_repo = GamesRepo::new(&db.conn);
    let locations_repo = SaveLocationsRepo::new(&db.conn);

    tracing::info!("Banco de dados aberto: {}", config.paths.database_path.display());

    // Executar discovery (inclui manifest do Ludusavi)
    tracing::info!("Iniciando discovery...");
    let result = savevault_discovery::run_all(&config_dir).await?;

    // Persistir jogos e obter o ID real do banco
    let mut real_ids = std::collections::HashMap::new();
    for game in &result.games {
        let mut real_id = game.id;
        
        // Tentar encontrar o jogo pelo platform_id para não mudar o UUID de um jogo existente
        if let Some(ref platform_id) = game.platform_id {
            if let Ok(Some(existing)) = games_repo.find_by_platform_id(&game.platform, platform_id) {
                real_id = existing.id;
            } else {
                games_repo.insert(game)?;
            }
        } else {
            // Jogos sem platform_id
            games_repo.insert(game)?;
        }
        
        real_ids.insert(game.id, real_id);
    }

    // Persistir save locations detectadas via manifest
    let mut new_locations = 0usize;
    for loc in &result.save_locations {
        if let Some(&real_id) = real_ids.get(&loc.game_id) {
            let mut loc_to_insert = loc.clone();
            loc_to_insert.game_id = real_id;
            if locations_repo.insert(&loc_to_insert).is_ok() {
                new_locations += 1;
            }
        }
    }

    if new_locations > 0 {
        tracing::info!(
            "{} caminho(s) de save novo(s) registrado(s) no banco.",
            new_locations
        );
    }

    tracing::info!(
        "SaveVault v0.4 — {} jogo(s) registrado(s), {} caminho(s) de save detectado(s).",
        result.games.len(),
        result.save_locations.len()
    );

    Ok(())
}
