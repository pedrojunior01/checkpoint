//! Camada de orquestração da aplicação SaveVault:
//! Conecta Game Discovery → Save Discovery → Canonicalização → Persistência SQLite.

use anyhow::Result;
use rusqlite::Connection;
use savevault_core::models::{Game, SaveLocation};
use savevault_db::repositories::SaveLocationsRepo;
use savevault_discovery::{SaveDetector, SaveDiscoveryResult};

/// Sumário com métricas do resultado da orquestração.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DiscoverySyncReport {
    /// Quantidade de jogos processados
    pub games_processed: usize,
    /// Locais de save persistidos (idempotente)
    pub persisted_locations: Vec<SaveLocation>,
    /// Mensagens de conflito seguro de identidade
    pub conflicts: Vec<String>,
    /// Nomes de jogos não reconhecidos pela base
    pub unrecognized: Vec<String>,
}

/// Orquestra a descoberta e persistência idempotente de saves para uma lista de jogos:
///
/// 1. Consulta o detector para cada jogo;
/// 2. Propaga erros de descoberta quando críticos;
/// 3. Se `Found` ou `KnownNotFoundOnDisk`, persiste cada local individualmente usando `SaveLocationsRepo::upsert`;
/// 4. O `upsert` aplica normalização de caminhos e preserva o UUID existente no banco;
/// 5. Se `ConflictingGameIdentity`, bloqueia a persistência para evitar associação errônea e registra o conflito;
/// 6. Se ocorrer qualquer erro no SQLite, o erro é imediatamente propagado (sem engolir ou esconder com ok()).
pub fn discover_and_persist_saves<D: SaveDetector>(
    conn: &Connection,
    games: &[Game],
    detector: &D,
) -> Result<DiscoverySyncReport> {
    let loc_repo = SaveLocationsRepo::new(conn);
    let mut report = DiscoverySyncReport::default();

    for game in games {
        report.games_processed += 1;
        let discovery = detector.detect_save_locations(game)?;

        match discovery {
            SaveDiscoveryResult::Found(locs)
            | SaveDiscoveryResult::KnownNotFoundOnDisk {
                expected_locations: locs,
            } => {
                for loc in locs {
                    let persisted = loc_repo.upsert(&loc)?;
                    report.persisted_locations.push(persisted);
                }
            }
            SaveDiscoveryResult::ConflictingGameIdentity {
                game_name,
                platform,
                expected_platform_id,
                manifest_platform_id,
            } => {
                let msg = format!(
                    "Conflito de identidade para '{}' ({:?}): ID esperado '{}' difere do manifest '{}'",
                    game_name, platform, expected_platform_id, manifest_platform_id
                );
                tracing::warn!("{}", msg);
                report.conflicts.push(msg);
            }
            SaveDiscoveryResult::UnrecognizedGame => {
                report.unrecognized.push(game.name.clone());
            }
        }
    }

    Ok(report)
}

/// Executa a varredura completa de todos os launchers e heurísticas,
/// persistindo os jogos e locais de save no banco de dados de forma idempotente.
///
/// Esta função é resiliente: mesmo que o download do manifest do Ludusavi falhe
/// (sem internet na primeira execução), os jogos instalados ainda serão detectados
/// via Steam, GOG e detector genérico.
pub async fn run_full_discovery(
    db_conn: &std::sync::Arc<tokio::sync::Mutex<rusqlite::Connection>>,
    config_dir: &std::path::Path,
) -> Result<(usize, usize)> {
    tracing::info!("Iniciando varredura e discovery de jogos...");

    // Garante que o diretório de config (usado para cache do manifest) existe
    let _ = std::fs::create_dir_all(config_dir);

    // run_all já é tolerante a falhas individuais de cada detector (Steam, GOG, Generic,
    // Manifest). O único cenário em que retorna Err é uma falha crítica de I/O.
    // Usamos map_err para logar e retornar 0 em vez de propagar o erro,
    // pois o usuário não deve ver uma tela em branco por causa de um manifest.
    let result = match savevault_discovery::run_all(config_dir).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "run_all retornou erro inesperado: {}. Discovery parcial indisponível.",
                e
            );
            savevault_discovery::DiscoveryResult {
                games: vec![],
                save_locations: vec![],
            }
        }
    };

    let conn = db_conn.lock().await;
    let games_repo = savevault_db::repositories::GamesRepo::new(&conn);
    let locations_repo = savevault_db::repositories::SaveLocationsRepo::new(&conn);

    let mut real_ids = std::collections::HashMap::new();
    for game in &result.games {
        let mut existing_opt = None;

        if let Some(ref platform_id) = game.platform_id {
            if let Ok(Some(existing)) = games_repo.find_by_platform_id(&game.platform, platform_id)
            {
                existing_opt = Some(existing);
            }
        }

        if existing_opt.is_none() {
            if let Ok(Some(existing)) =
                games_repo.find_by_name_and_platform(&game.name, &game.platform)
            {
                existing_opt = Some(existing);
            }
        }

        let real_id = match existing_opt {
            Some(existing) => existing.id,
            None => {
                games_repo.insert(game)?;
                game.id
            }
        };

        real_ids.insert(game.id, real_id);
    }

    let ignored_paths: std::collections::HashSet<String> = {
        let mut set = std::collections::HashSet::new();
        if let Ok(mut stmt) = conn.prepare("SELECT path FROM ignored_locations") {
            if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                for p in rows.flatten() {
                    set.insert(p.trim().to_ascii_lowercase());
                }
            }
        }
        set
    };

    let mut new_locations = 0usize;
    for loc in &result.save_locations {
        let norm_loc_path = loc.path.to_string_lossy().trim().to_ascii_lowercase();
        if ignored_paths.contains(&norm_loc_path) {
            tracing::debug!(
                "Ignorando save location explicitamente excluído pelo usuário: {:?}",
                loc.path
            );
            continue;
        }

        if let Some(&real_id) = real_ids.get(&loc.game_id) {
            let mut loc_to_insert = loc.clone();
            loc_to_insert.game_id = real_id;
            if locations_repo.upsert(&loc_to_insert).is_ok() {
                new_locations += 1;
            }
        }
    }

    // Limpa jogos que ficaram sem nenhuma save_location cadastrada
    let _ = conn.execute(
        "DELETE FROM games WHERE id NOT IN (SELECT DISTINCT game_id FROM save_locations)",
        [],
    );

    tracing::info!(
        "checkpoint — Discovery concluído: {} jogo(s) catalogado(s), {} caminho(s) de save registrado(s).",
        result.games.len(),
        new_locations
    );

    Ok((result.games.len(), new_locations))
}
