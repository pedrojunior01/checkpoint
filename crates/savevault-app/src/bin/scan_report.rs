/// scan_report.rs — Ferramenta de diagnóstico: lê o banco SQLite e imprime
/// um relatório visual dos jogos detectados pela Steam.
/// Uso: cargo run --bin scan_report

use savevault_core::config::Config;
use savevault_db::{Database, repositories::{GamesRepo, SaveLocationsRepo}};
use savevault_core::models::Confidence;

fn main() -> anyhow::Result<()> {
    let config_path = {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
        std::path::PathBuf::from(appdata).join("SaveVault").join("config.toml")
    };
    let config = Config::load(&config_path).unwrap_or_default();

    let db = Database::open(&config.paths.database_path)?;
    let games_repo = GamesRepo::new(&db.conn);
    let loc_repo = SaveLocationsRepo::new(&db.conn);

    let games = games_repo.list_all()?;

    println!();
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║              SAVEVAULT — RELATORIO DE SCAN STEAM             ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    if games.is_empty() {
        println!("  Nenhum jogo encontrado. Execute 'cargo run -p savevault-app' primeiro.");
        return Ok(());
    }

    println!("  Jogos detectados: {}", games.len());
    println!("  Banco de dados  : {}", config.paths.database_path.display());
    println!();
    println!("  {:<45} {:<12} {:<10} {:<8}", "JOGO", "APP ID", "CONFIANCA", "SAVES");
    println!("  {}", "-".repeat(80));

    for game in &games {
        let locations = loc_repo.list_by_game(game.id)?;
        let confidence_str = match game.confidence {
            Confidence::High   => "Alta",
            Confidence::Medium => "Media",
            Confidence::Low    => "Baixa",
        };
        let app_id = game.platform_id.as_deref().unwrap_or("—");
        let saves_count = if locations.is_empty() {
            "Pendente".to_string()
        } else {
            locations.len().to_string()
        };

        // Trunca nomes longos
        let name = if game.name.len() > 43 {
            format!("{}...", &game.name[..40])
        } else {
            game.name.clone()
        };

        println!("  {:<45} {:<12} {:<10} {:<8}", name, app_id, confidence_str, saves_count);
    }

    println!("  {}", "-".repeat(80));
    println!();
    println!("  LEGENDA");
    println!("  Alta confianca  : Jogo identificado via Steam AppID + manifest");
    println!("  Pendente (saves): Deteccao de saves implementada no v0.4");
    println!();
    println!("  Para ativar deteccao de saves, aguarde o milestone v0.4");
    println!("  (integracao com o Ludusavi Manifest — +19.000 jogos mapeados)");
    println!();

    Ok(())
}
