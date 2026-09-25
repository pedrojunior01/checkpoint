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
            SaveDiscoveryResult::Found(locs) | SaveDiscoveryResult::KnownNotFoundOnDisk { expected_locations: locs } => {
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
