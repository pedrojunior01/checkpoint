//! scan_report.rs — Relatório visual dos jogos e saves detectados pelo SaveVault.
//! Uso: cargo run --bin scan_report

use savevault_core::config::Config;
use savevault_core::models::Confidence;
use savevault_db::{
    repositories::{GamesRepo, SaveLocationsRepo},
    Database,
};

fn main() -> anyhow::Result<()> {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    let config_dir = std::path::PathBuf::from(&appdata).join("SaveVault");
    let config = Config::load(&config_dir.join("config.toml")).unwrap_or_default();

    let db = Database::open(&config.paths.database_path)?;
    let games_repo = GamesRepo::new(&db.conn);
    let loc_repo = SaveLocationsRepo::new(&db.conn);

    let games = games_repo.list_all()?;

    println!();
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║           SAVEVAULT v0.4 — RELATORIO DE SCAN STEAM           ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    if games.is_empty() {
        println!("  Nenhum jogo encontrado. Execute 'cargo run --bin savevault' primeiro.");
        return Ok(());
    }

    println!("  Banco de dados: {}", config.paths.database_path.display());
    println!("  Jogos detectados: {}", games.len());
    println!();

    let mut total_saves = 0usize;
    let mut games_with_saves = 0usize;

    for game in &games {
        let locations = loc_repo.list_by_game(game.id)?;
        let confidence_str = match game.confidence {
            Confidence::High => "Alta",
            Confidence::Medium => "Media",
            Confidence::Low => "Baixa",
        };
        let app_id = game.platform_id.as_deref().unwrap_or("—");

        let name = if game.name.len() > 50 {
            format!("{}...", &game.name[..47])
        } else {
            game.name.clone()
        };

        println!(
            "  [{:<6}] {:<52} Confianca: {}",
            app_id, name, confidence_str
        );

        if locations.is_empty() {
            println!("           Save: Nao detectado via manifest");
        } else {
            games_with_saves += 1;
            for loc in &locations {
                let exists = if loc.path.exists() {
                    "OK     "
                } else {
                    "Ausente"
                };
                println!("           Save [{}]: {}", exists, loc.path.display());
                total_saves += 1;
            }
        }
        println!();
    }

    println!("  {}", "─".repeat(72));
    println!("  Resumo:");
    println!(
        "    Jogos com saves detectados : {}/{}",
        games_with_saves,
        games.len()
    );
    println!("    Total de caminhos de save  : {}", total_saves);
    println!();
    println!("  LEGENDA DE STATUS DOS SAVES:");
    println!("    OK      : Diretorio existe no sistema (jogo ja foi jogado)");
    println!("    Ausente : Diretorio ainda nao existe (jogo nunca aberto / save em nuvem)");
    println!();

    Ok(())
}
