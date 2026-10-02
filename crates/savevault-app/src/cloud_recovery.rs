//! Motor de Recuperação e Restauração em Nuvem (CLOUD-REC-01).
//!
//! Permite que o SaveVault recupere saves a partir do Google Drive ou Microsoft OneDrive
//! em cenários de Disaster Recovery (ex.: computador recém-formatado, novo dispositivo ou
//! exclusão acidental de arquivos locais de backup).

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::restore::download_snapshot_from_cloud;

/// Informações de um snapshot presente na nuvem e sua disponibilidade local.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudSnapshotEntry {
    pub snapshot_id: Uuid,
    pub file_id: Uuid,
    pub game_name: String,
    pub version: u32,
    pub original_save_path: PathBuf,
    pub cloud_path: Option<String>,
    pub drive_file_id: Option<String>,
    pub local_exists: bool,
    pub local_snapshot_path: PathBuf,
    pub created_at: DateTime<Utc>,
}

/// Relatório de download em lote da nuvem.
#[derive(Debug, Clone, Default)]
pub struct PullReport {
    pub total_scanned: usize,
    pub already_local: usize,
    pub downloaded: usize,
    pub failed: usize,
    pub details: Vec<String>,
}

/// Lista todos os snapshots registrados que possuem cópia na nuvem (Google Drive ou OneDrive),
/// indicando se o arquivo físico já existe no disco local ou se precisa ser baixado.
pub fn list_cloud_snapshots(
    conn: &Connection,
    backup_root: &Path,
) -> Result<Vec<CloudSnapshotEntry>> {
    let mut stmt = conn.prepare(
        "SELECT b.id, b.file_id, b.version, b.cloud_path, b.drive_file_id, b.created_at,
                f.path, g.name
         FROM backups b
         JOIN files f ON b.file_id = f.id
         JOIN save_locations sl ON f.save_location_id = sl.id
         JOIN games g ON sl.game_id = g.id
         WHERE b.drive_file_id IS NOT NULL OR b.cloud_path IS NOT NULL
         ORDER BY g.name ASC, b.version DESC",
    )?;

    let rows = stmt.query_map([], |row| {
        let id_str: String = row.get(0)?;
        let file_id_str: String = row.get(1)?;
        let version: u32 = row.get(2)?;
        let cloud_path: Option<String> = row.get(3)?;
        let drive_file_id: Option<String> = row.get(4)?;
        let created_at_str: String = row.get(5)?;
        let file_path_str: String = row.get(6)?;
        let game_name: String = row.get(7)?;

        let snapshot_id = Uuid::parse_str(&id_str).unwrap_or_default();
        let file_id = Uuid::parse_str(&file_id_str).unwrap_or_default();
        let created_at = DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        Ok((
            snapshot_id,
            file_id,
            version,
            cloud_path,
            drive_file_id,
            created_at,
            PathBuf::from(file_path_str),
            game_name,
        ))
    })?;

    let mut result = Vec::new();
    for r in rows {
        let (
            snapshot_id,
            file_id,
            version,
            cloud_path,
            drive_file_id,
            created_at,
            original_save_path,
            game_name,
        ) = r?;

        let filename = original_save_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("backup_{}.dat", snapshot_id.as_simple()));

        let safe_game_name = savevault_backup::snapshot::sanitize_name(&game_name);

        let candidate_versioned = backup_root
            .join(&safe_game_name)
            .join(format!("v{}", version))
            .join(&filename);

        let candidate_latest = backup_root
            .join(&safe_game_name)
            .join("latest")
            .join(&filename);

        let (local_exists, local_snapshot_path) = if candidate_versioned.exists() {
            (true, candidate_versioned)
        } else if candidate_latest.exists() {
            (true, candidate_latest)
        } else {
            (false, candidate_versioned)
        };

        result.push(CloudSnapshotEntry {
            snapshot_id,
            file_id,
            game_name,
            version,
            original_save_path,
            cloud_path,
            drive_file_id,
            local_exists,
            local_snapshot_path,
            created_at,
        });
    }

    Ok(result)
}

/// Reconstrói ou sincroniza o catálogo local no banco SQLite a partir do manifesto remoto (F-05).
/// Permite recuperar registros de jogos, save_locations, files e backups mesmo em máquina nova/reformatada.
pub async fn reconstruct_catalog_from_cloud(
    conn: &Connection,
    provider: &dyn savevault_cloud::CloudProvider,
) -> Result<usize> {
    let manifest_opt = provider.get_manifest().await?;
    let manifest = match manifest_opt {
        Some(m) => m,
        None => return Ok(0),
    };

    let mut imported_count = 0;

    for entry in manifest.entries {
        // 1. Garante a existência do jogo (games)
        let game_id: Uuid = conn
            .query_row(
                "SELECT id FROM games WHERE LOWER(name) = LOWER(?1)",
                [&entry.game_name],
                |r| {
                    let s: String = r.get(0)?;
                    Ok(Uuid::parse_str(&s).unwrap_or_else(|_| Uuid::new_v4()))
                },
            )
            .unwrap_or_else(|_| {
                let new_id = Uuid::new_v4();
                let now = Utc::now().to_rfc3339();
                let _ = conn.execute(
                    "INSERT INTO games (id, name, platform, confidence, created_at)
                     VALUES (?1, ?2, ?3, 'high', ?4)",
                    (new_id.to_string(), &entry.game_name, &entry.platform, now),
                );
                new_id
            });

        // 2. Garante a existência do local de save (save_locations)
        let location_id: Uuid = conn
            .query_row(
                "SELECT id FROM save_locations WHERE game_id = ?1 AND path = ?2",
                [&game_id.to_string(), &entry.save_location_path],
                |r| {
                    let s: String = r.get(0)?;
                    Ok(Uuid::parse_str(&s).unwrap_or_else(|_| Uuid::new_v4()))
                },
            )
            .unwrap_or_else(|_| {
                let new_id = Uuid::new_v4();
                let _ = conn.execute(
                    "INSERT INTO save_locations (id, game_id, path, location_type)
                     VALUES (?1, ?2, ?3, 'directory')",
                    (
                        new_id.to_string(),
                        &game_id.to_string(),
                        &entry.save_location_path,
                    ),
                );
                new_id
            });

        // 3. Garante a existência do registro do arquivo (files)
        let full_file_path =
            PathBuf::from(&entry.save_location_path).join(&entry.relative_file_path);
        let full_file_path_str = full_file_path.to_string_lossy().to_string();

        let file_id: Uuid = conn
            .query_row(
                "SELECT id FROM files WHERE save_location_id = ?1 AND path = ?2",
                [&location_id.to_string(), &full_file_path_str],
                |r| {
                    let s: String = r.get(0)?;
                    Ok(Uuid::parse_str(&s).unwrap_or_else(|_| Uuid::new_v4()))
                },
            )
            .unwrap_or_else(|_| {
                let new_id = Uuid::new_v4();
                let now = Utc::now().to_rfc3339();
                let _ = conn.execute(
                    "INSERT INTO files (id, save_location_id, path, hash, size_bytes, modified_at)
                     VALUES (?1, ?2, ?3, ?4, 0, ?5)",
                    (
                        new_id.to_string(),
                        &location_id.to_string(),
                        &full_file_path_str,
                        &entry.file_hash,
                        now,
                    ),
                );
                new_id
            });

        // 4. Garante a existência do registro do backup (backups)
        let backup_exists: bool = conn
            .query_row(
                "SELECT 1 FROM backups WHERE file_id = ?1 AND version = ?2",
                [&file_id.to_string(), &entry.version.to_string()],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if !backup_exists {
            let backup_id = Uuid::new_v4();
            let created_at = if entry.created_at.is_empty() {
                Utc::now().to_rfc3339()
            } else {
                entry.created_at
            };

            let inserted = conn.execute(
                "INSERT INTO backups (id, file_id, version, cloud_path, drive_file_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (
                    backup_id.to_string(),
                    file_id.to_string(),
                    entry.version,
                    &entry.cloud_path,
                    &entry.cloud_file_id,
                    created_at,
                ),
            );

            if inserted.is_ok() {
                imported_count += 1;
            }
        }
    }

    Ok(imported_count)
}

/// Baixa todos os snapshots ausentes localmente da nuvem para o disco local de backups.
///
/// Permite recuperar todo o histórico de saves ou filtrar por um jogo específico.
pub async fn pull_snapshots_from_cloud(
    conn: &Connection,
    backup_root: &Path,
    preferred_provider: Option<&str>,
    filter_game: Option<&str>,
) -> Result<PullReport> {
    // Sincroniza primeiro o catálogo da nuvem se fornecido provedor ou se banco local estiver vazio (F-05)
    if let Some(prov_name) = preferred_provider {
        let provider = savevault_sync::resolve_active_cloud_provider(prov_name, backup_root);
        let _ = reconstruct_catalog_from_cloud(conn, provider.as_ref()).await;
    }

    let all = list_cloud_snapshots(conn, backup_root)?;
    let mut report = PullReport {
        total_scanned: all.len(),
        ..Default::default()
    };

    let target_game_lower = filter_game.map(|g| g.trim().to_lowercase());

    for item in all {
        if let Some(ref target) = target_game_lower {
            if !item.game_name.to_lowercase().contains(target) {
                continue;
            }
        }

        if item.local_exists {
            report.already_local += 1;
            continue;
        }

        let remote_id = match item.drive_file_id.as_deref() {
            Some(id) if !id.trim().is_empty() => id,
            _ => {
                report.failed += 1;
                report.details.push(format!(
                    "{} (v{}): Metadados remotos ausentes no banco",
                    item.game_name, item.version
                ));
                continue;
            }
        };

        tracing::info!(
            "Baixando snapshot da nuvem: {} v{} (remote ID: {}) → '{}'",
            item.game_name,
            item.version,
            remote_id,
            item.local_snapshot_path.display()
        );

        match download_snapshot_from_cloud(remote_id, &item.local_snapshot_path, preferred_provider)
            .await
        {
            Ok(()) => {
                report.downloaded += 1;
                report.details.push(format!(
                    "{} (v{}): Baixado com sucesso",
                    item.game_name, item.version
                ));
            }
            Err(e) => {
                report.failed += 1;
                report.details.push(format!(
                    "{} (v{}): Falha ao baixar: {}",
                    item.game_name, item.version, e
                ));
            }
        }
    }

    if report.downloaded > 0 {
        crate::notifications::notify(
            "SaveVault — Recuperação da Nuvem",
            format!(
                "Recuperação concluída: {} save(s) restaurado(s) da nuvem com sucesso!",
                report.downloaded
            ),
        );
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::tempdir;

    fn setup_test_db() -> Connection {
        savevault_db::Database::open_in_memory().unwrap().conn
    }

    #[test]
    fn test_list_cloud_snapshots_empty() {
        let conn = setup_test_db();
        let tmp = tempdir().unwrap();
        let list = list_cloud_snapshots(&conn, tmp.path()).unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn test_list_cloud_snapshots_detects_cloud_records() {
        let conn = setup_test_db();
        let tmp = tempdir().unwrap();

        // Insere jogo, location, file e backup com drive_file_id
        let game_id = Uuid::new_v4();
        let loc_id = Uuid::new_v4();
        let file_id = Uuid::new_v4();
        let backup_id = Uuid::new_v4();

        conn.execute(
            "INSERT INTO games (id, name, platform, confidence, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            (game_id.to_string(), "Elden Ring", "steam", "high", Utc::now().to_rfc3339()),
        ).unwrap();

        conn.execute(
            "INSERT INTO save_locations (id, game_id, path, location_type) VALUES (?1, ?2, ?3, ?4)",
            (
                loc_id.to_string(),
                game_id.to_string(),
                r"C:\Saves\EldenRing",
                "directory",
            ),
        )
        .unwrap();

        conn.execute(
            "INSERT INTO files (id, save_location_id, path, hash, size_bytes, modified_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (file_id.to_string(), loc_id.to_string(), r"C:\Saves\EldenRing\save01.dat", "dummyhash", 1024, Utc::now().to_rfc3339()),
        ).unwrap();

        conn.execute(
            "INSERT INTO backups (id, file_id, version, cloud_path, drive_file_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (backup_id.to_string(), file_id.to_string(), 1, "EldenRing/save01.dat", "remote-drive-id-123", Utc::now().to_rfc3339()),
        ).unwrap();

        let list = list_cloud_snapshots(&conn, tmp.path()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].game_name, "Elden Ring");
        assert_eq!(list[0].version, 1);
        assert_eq!(
            list[0].drive_file_id.as_deref(),
            Some("remote-drive-id-123")
        );
        assert!(!list[0].local_exists);
    }

    #[tokio::test]
    async fn test_reconstruct_catalog_from_cloud_on_empty_database() {
        use savevault_cloud::CloudProvider;
        let conn = setup_test_db();
        let tmp = tempdir().unwrap();

        // Inicializa LocalFolderProvider apontando para a pasta temporária
        let provider = savevault_cloud::LocalFolderProvider::new(tmp.path().to_path_buf());

        // Cria e salva um manifesto simulando backups existentes na nuvem
        let manifest = savevault_cloud::CloudCatalogManifest {
            schema_version: 1,
            updated_at: Utc::now().to_rfc3339(),
            entries: vec![
                savevault_cloud::CloudManifestEntry {
                    game_name: "Hollow Knight".to_string(),
                    platform: "steam".to_string(),
                    save_location_path: r"C:\Games\HollowKnight\Saves".to_string(),
                    relative_file_path: "user1.dat".to_string(),
                    file_hash: "hash_hk_1".to_string(),
                    version: 1,
                    cloud_file_id: "hk-v1-cloud-id".to_string(),
                    cloud_path: "loc/file/v1_user1.dat".to_string(),
                    created_at: Utc::now().to_rfc3339(),
                },
                savevault_cloud::CloudManifestEntry {
                    game_name: "Hollow Knight".to_string(),
                    platform: "steam".to_string(),
                    save_location_path: r"C:\Games\HollowKnight\Saves".to_string(),
                    relative_file_path: "user1.dat".to_string(),
                    file_hash: "hash_hk_2".to_string(),
                    version: 2,
                    cloud_file_id: "hk-v2-cloud-id".to_string(),
                    cloud_path: "loc/file/v2_user1.dat".to_string(),
                    created_at: Utc::now().to_rfc3339(),
                },
            ],
        };

        provider.save_manifest(&manifest).await.unwrap();

        // Verifica que o banco local está completamente vazio
        let count_before: usize = conn
            .query_row("SELECT count(*) FROM games", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count_before, 0);

        // Executa a reconstrução de desastre (F-05)
        let imported = reconstruct_catalog_from_cloud(&conn, &provider)
            .await
            .unwrap();
        assert_eq!(imported, 2);

        // Valida que o banco SQLite foi populado
        let snapshots = list_cloud_snapshots(&conn, tmp.path()).unwrap();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].game_name, "Hollow Knight");
        assert_eq!(snapshots[0].version, 2);
        assert_eq!(snapshots[1].version, 1);
    }
}
