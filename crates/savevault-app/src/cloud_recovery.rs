//! Motor de Recuperação e Restauração em Nuvem (CLOUD-REC-01 / AUD-02).
//!
//! Permite que o SaveVault recupere saves a partir do Google Drive, Microsoft OneDrive
//! ou Pasta Sincronizada Local em cenários de Disaster Recovery (ex.: computador recém-formatado,
//! novo dispositivo, perda do banco SQLite local ou exclusão acidental de arquivos locais de backup).

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::restore::download_snapshot_from_cloud;

/// Erros categorizados retornados pelas operações de Disaster Recovery da nuvem.
#[derive(Debug, thiserror::Error)]
pub enum RecoveryError {
    #[error("Usuário não autenticado no provedor de nuvem")]
    Unauthenticated,

    #[error("Erro de autenticação no provedor de nuvem: {0}")]
    Authentication(String),

    #[error("Manifesto de recuperação não encontrado na nuvem")]
    ManifestNotFound,

    #[error("Manifesto remoto inválido ou corrompido: {0}")]
    InvalidManifest(String),

    #[error("Erro de rede ao comunicar com a nuvem: {0}")]
    Network(String),

    #[error("Erro da API do provedor de nuvem: {0}")]
    Api(String),

    #[error("Erro no banco de dados local durante recuperação: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Erro de I/O durante recuperação: {0}")]
    Io(#[from] std::io::Error),

    #[error("Violação de segurança detectada no manifesto remoto: {0}")]
    SecurityViolation(String),

    #[error("Erro de integridade do backup: {0}")]
    Integrity(String),

    #[error("Outro erro durante recuperação: {0}")]
    Other(String),
}

impl From<savevault_cloud::CloudError> for RecoveryError {
    fn from(err: savevault_cloud::CloudError) -> Self {
        match err {
            savevault_cloud::CloudError::Unauthenticated => RecoveryError::Unauthenticated,
            savevault_cloud::CloudError::Authentication(msg) => RecoveryError::Authentication(msg),
            savevault_cloud::CloudError::ManifestNotFound => RecoveryError::ManifestNotFound,
            savevault_cloud::CloudError::InvalidManifest(msg) => {
                RecoveryError::InvalidManifest(msg)
            }
            savevault_cloud::CloudError::FileNotFound(_) => RecoveryError::ManifestNotFound,
            savevault_cloud::CloudError::Network(msg) => RecoveryError::Network(msg),
            savevault_cloud::CloudError::Api { status, message } => {
                RecoveryError::Api(format!("Status {:?}: {}", status, message))
            }
            savevault_cloud::CloudError::InvalidRemoteId(msg) => {
                RecoveryError::SecurityViolation(msg)
            }
            savevault_cloud::CloudError::Io(e) => RecoveryError::Io(e),
            savevault_cloud::CloudError::Other(e) => RecoveryError::Other(e.to_string()),
        }
    }
}

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

/// Lista todos os snapshots registrados que possuem cópia na nuvem (Google Drive, OneDrive ou Pasta Local),
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
/// Permite recuperar registros de jogos, save_locations, files e backups mesmo em máquina nova/reformatada
/// onde o banco de dados SQLite local foi completamente destruído.
pub async fn reconstruct_catalog_from_cloud(
    conn: &Connection,
    provider: &dyn savevault_cloud::CloudProvider,
) -> Result<usize, RecoveryError> {
    tracing::info!("Starting cloud disaster recovery catalog reconstruction");
    tracing::info!("Searching for manifest in cloud provider");

    let manifest_opt = provider.get_manifest().await.map_err(|e| {
        tracing::error!("Failed to retrieve manifest from cloud: {}", e);
        RecoveryError::from(e)
    })?;

    let manifest = match manifest_opt {
        Some(m) => {
            tracing::info!(
                "Manifest found, downloaded and parsed successfully (schema v{}, {} entries)",
                m.schema_version,
                m.entries.len()
            );
            m
        }
        None => {
            tracing::warn!("Manifest not found on cloud provider");
            return Err(RecoveryError::ManifestNotFound);
        }
    };

    tracing::info!(
        "Reconstructing catalog from manifest ({} entries)",
        manifest.entries.len()
    );

    conn.execute_batch("SAVEPOINT reconstruct_catalog;")
        .map_err(RecoveryError::Database)?;

    let mut imported_count = 0;

    for entry in manifest.entries {
        // Validação de segurança estrita contra path traversal
        if entry.relative_file_path.contains("..")
            || entry.relative_file_path.starts_with('/')
            || entry.relative_file_path.starts_with('\\')
            || entry.relative_file_path.contains(':')
        {
            tracing::warn!(
                "Entrada ignorada por violação de segurança (path traversal): '{}'",
                entry.relative_file_path
            );
            continue;
        }

        if entry.cloud_file_id.trim().is_empty() {
            tracing::warn!("Entrada ignorada: cloud_file_id vazio");
            continue;
        }

        tracing::info!(
            "Snapshot discovered: {} v{} (hash: {}, remote ID: {})",
            entry.game_name,
            entry.version,
            entry.file_hash,
            entry.cloud_file_id
        );

        // 1. Garante a existência do jogo (games)
        let game_id: Uuid = conn
            .query_row(
                "SELECT id FROM games WHERE LOWER(name) = LOWER(?1) AND platform = ?2",
                [&entry.game_name, &entry.platform],
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

        // 2. Garante a existência do local de save (save_locations) com suporte a múltiplos locais por jogo
        let norm_entry_save_path =
            savevault_core::path::normalize_windows_path_str(&entry.save_location_path)
                .unwrap_or_else(|_| entry.save_location_path.clone());
        let canon_entry_save_path =
            savevault_core::path::canonicalize_windows_path_str(&norm_entry_save_path)
                .unwrap_or_else(|_| norm_entry_save_path.to_ascii_lowercase());

        let (location_id, effective_save_path): (Uuid, PathBuf) = {
            // Consulta todas as localizações já cadastradas para este jogo
            let mut loc_stmt = match conn.prepare(
                "SELECT id, path, original_path, mapping_status FROM save_locations WHERE game_id = ?1",
            ) {
                Ok(s) => s,
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK TO reconstruct_catalog; RELEASE reconstruct_catalog;");
                    return Err(RecoveryError::Database(e));
                }
            };

            let existing_rows = loc_stmt.query_map([&game_id.to_string()], |r| {
                let id_str: String = r.get(0)?;
                let p_str: String = r.get(1)?;
                let orig_p_str: Option<String> = r.get(2)?;
                let status_str: String = r.get(3)?;
                let id = Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::new_v4());
                Ok((id, p_str, orig_p_str, status_str))
            });

            let mut matched_location: Option<(Uuid, PathBuf, String)> = None;
            if let Ok(rows) = existing_rows {
                for row in rows.flatten() {
                    let (loc_id, loc_path_str, orig_path_str, status_str) = row;
                    let matches_orig = if let Some(ref orig) = orig_path_str {
                        savevault_core::path::canonicalize_windows_path_str(orig)
                            .map(|c| c == canon_entry_save_path)
                            .unwrap_or(false)
                    } else {
                        false
                    };

                    let matches_curr =
                        savevault_core::path::canonicalize_windows_path_str(&loc_path_str)
                            .map(|c| c == canon_entry_save_path)
                            .unwrap_or(false);

                    if matches_orig || matches_curr {
                        matched_location = Some((loc_id, PathBuf::from(loc_path_str), status_str));
                        break;
                    }
                }
            }

            if let Some((existing_id, existing_path, existing_status)) = matched_location {
                // Se já existir a localização correspondente a esta entrada, preserva seu caminho e status
                tracing::info!(
                    "Local de save existente preservado para '{}' (status: {}): '{}'",
                    entry.game_name,
                    existing_status,
                    existing_path.display()
                );
                (existing_id, existing_path)
            } else {
                // Nova localização para este jogo: auto-resolução segura de caminhos
                let (resolved_path, status, source, reason, _) =
                    crate::remapping::resolve_path_for_game(
                        &entry.game_name,
                        Path::new(&entry.save_location_path),
                        None,
                    );

                let new_id = Uuid::new_v4();
                let path_str = resolved_path.to_string_lossy().to_string();
                let status_str = match status {
                    savevault_core::models::MappingStatus::Resolved => "resolved",
                    savevault_core::models::MappingStatus::AutoResolved => "auto_resolved",
                    savevault_core::models::MappingStatus::UserMapped => "user_mapped",
                    savevault_core::models::MappingStatus::NeedsUserMapping => "needs_user_mapping",
                    savevault_core::models::MappingStatus::Invalid => "invalid",
                };
                let source_str = match source {
                    savevault_core::models::MappingSource::Original => "original",
                    savevault_core::models::MappingSource::Auto => "auto",
                    savevault_core::models::MappingSource::Discovery => "discovery",
                    savevault_core::models::MappingSource::User => "user",
                };

                tracing::info!(
                    "Local de save registrado para '{}': '{}' (status: {}, source: {})",
                    entry.game_name,
                    path_str,
                    status_str,
                    source_str
                );

                if let Err(e) = conn.execute(
                    "INSERT INTO save_locations (id, game_id, path, location_type, original_path, mapping_status, mapping_source, resolution_reason)
                     VALUES (?1, ?2, ?3, 'directory', ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        new_id.to_string(),
                        game_id.to_string(),
                        path_str,
                        &entry.save_location_path,
                        status_str,
                        source_str,
                        reason,
                    ],
                ) {
                    let _ = conn.execute_batch("ROLLBACK TO reconstruct_catalog; RELEASE reconstruct_catalog;");
                    return Err(RecoveryError::Database(e));
                }
                (new_id, resolved_path)
            }
        };

        // 3. Garante a existência do registro do arquivo (files) com base no caminho efetivo da localização
        let full_file_path = effective_save_path.join(&entry.relative_file_path);
        let full_file_path_str = full_file_path.to_string_lossy().to_string();

        let norm_target_file_str =
            savevault_core::path::canonicalize_windows_path_str(&full_file_path_str)
                .unwrap_or_else(|_| full_file_path_str.to_ascii_lowercase());

        let file_id: Uuid = {
            let mut found_file_id: Option<Uuid> = None;
            if let Ok(mut files_stmt) =
                conn.prepare("SELECT id, path FROM files WHERE save_location_id = ?1")
            {
                if let Ok(file_rows) = files_stmt.query_map([&location_id.to_string()], |r| {
                    let fid: String = r.get(0)?;
                    let fpath: String = r.get(1)?;
                    Ok((fid, fpath))
                }) {
                    for row in file_rows.flatten() {
                        let (fid_str, fpath_str) = row;
                        // 1. Match exato no caminho completo
                        if let Ok(canon_existing) =
                            savevault_core::path::canonicalize_windows_path_str(&fpath_str)
                        {
                            if canon_existing == norm_target_file_str {
                                if let Ok(u) = Uuid::parse_str(&fid_str) {
                                    found_file_id = Some(u);
                                    break;
                                }
                            }
                        }
                        // 2. Match determinístico no caminho relativo sob a localização de save
                        let rel = savevault_core::path::compute_relative_path(
                            Path::new(&fpath_str),
                            &effective_save_path,
                        );
                        if rel == Path::new(&entry.relative_file_path) {
                            if let Ok(u) = Uuid::parse_str(&fid_str) {
                                found_file_id = Some(u);
                                break;
                            }
                        }
                    }
                }
            }

            match found_file_id {
                Some(id) => id,
                None => {
                    let new_id = Uuid::new_v4();
                    let now = Utc::now().to_rfc3339();
                    if let Err(e) = conn.execute(
                        "INSERT INTO files (id, save_location_id, path, hash, size_bytes, modified_at)
                         VALUES (?1, ?2, ?3, ?4, 0, ?5)",
                        (
                            new_id.to_string(),
                            &location_id.to_string(),
                            &full_file_path_str,
                            &entry.file_hash,
                            now,
                        ),
                    ) {
                        let _ = conn.execute_batch("ROLLBACK TO reconstruct_catalog; RELEASE reconstruct_catalog;");
                        return Err(RecoveryError::Database(e));
                    }
                    new_id
                }
            }
        };

        // 4. Garante a existência do registro do backup (backups) com status 'synced'
        let backup_exists: bool = conn
            .query_row(
                "SELECT 1 FROM backups WHERE drive_file_id = ?1 OR (file_id = ?2 AND version = ?3)",
                [
                    &entry.cloud_file_id,
                    &file_id.to_string(),
                    &entry.version.to_string(),
                ],
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
                "INSERT INTO backups (id, file_id, version, cloud_path, sync_status, drive_file_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, 'synced', ?5, ?6)",
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

    conn.execute_batch("RELEASE reconstruct_catalog;")
        .map_err(RecoveryError::Database)?;

    tracing::info!(
        "Catalog reconstruction completed: {} snapshots imported/updated",
        imported_count
    );
    Ok(imported_count)
}

/// Baixa todos os snapshots ausentes localmente da nuvem usando o provider fornecido.
pub async fn pull_snapshots_with_provider(
    conn: &Connection,
    backup_root: &Path,
    provider: &dyn savevault_cloud::CloudProvider,
    filter_game: Option<&str>,
) -> Result<PullReport, RecoveryError> {
    // Sincroniza primeiro o catálogo da nuvem (F-05)
    reconstruct_catalog_from_cloud(conn, provider).await?;

    let all =
        list_cloud_snapshots(conn, backup_root).map_err(|e| RecoveryError::Other(e.to_string()))?;

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

        match provider
            .download(remote_id, &item.local_snapshot_path)
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

/// Baixa todos os snapshots ausentes localmente da nuvem para o disco local de backups.
///
/// Permite recuperar todo o histórico de saves ou filtrar por um jogo específico.
pub async fn pull_snapshots_from_cloud(
    conn: &Connection,
    backup_root: &Path,
    preferred_provider: Option<&str>,
    filter_game: Option<&str>,
) -> Result<PullReport, RecoveryError> {
    let prov_name = preferred_provider.unwrap_or("auto");
    let provider = savevault_sync::resolve_active_cloud_provider(prov_name, backup_root);

    // Sincroniza primeiro o catálogo da nuvem se fornecido provedor ou se banco local estiver vazio (F-05)
    reconstruct_catalog_from_cloud(conn, provider.as_ref()).await?;

    let all =
        list_cloud_snapshots(conn, backup_root).map_err(|e| RecoveryError::Other(e.to_string()))?;

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
    use savevault_cloud::CloudError;
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

        let provider = savevault_cloud::LocalFolderProvider::new(tmp.path().to_path_buf());

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

        let count_before: usize = conn
            .query_row("SELECT count(*) FROM games", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count_before, 0);

        let imported = reconstruct_catalog_from_cloud(&conn, &provider)
            .await
            .unwrap();
        assert_eq!(imported, 2);

        let snapshots = list_cloud_snapshots(&conn, tmp.path()).unwrap();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].game_name, "Hollow Knight");
        assert_eq!(snapshots[0].version, 2);
        assert_eq!(snapshots[1].version, 1);
    }

    struct MockErrorCloudProvider {
        manifest_res: Result<Option<savevault_cloud::CloudCatalogManifest>, CloudError>,
    }

    #[async_trait::async_trait]
    impl savevault_cloud::CloudProvider for MockErrorCloudProvider {
        async fn upload(&self, _: &Path, _: &str) -> savevault_cloud::CloudResult<String> {
            Ok("id".into())
        }
        async fn download(&self, _: &str, _: &Path) -> savevault_cloud::CloudResult<()> {
            Ok(())
        }
        async fn delete(&self, _: &str) -> savevault_cloud::CloudResult<()> {
            Ok(())
        }
        async fn get_manifest(
            &self,
        ) -> savevault_cloud::CloudResult<Option<savevault_cloud::CloudCatalogManifest>> {
            match &self.manifest_res {
                Ok(m) => Ok(m.clone()),
                Err(CloudError::Unauthenticated) => Err(CloudError::Unauthenticated),
                Err(CloudError::Authentication(s)) => Err(CloudError::Authentication(s.clone())),
                Err(CloudError::ManifestNotFound) => Err(CloudError::ManifestNotFound),
                Err(CloudError::InvalidManifest(s)) => Err(CloudError::InvalidManifest(s.clone())),
                Err(CloudError::Network(s)) => Err(CloudError::Network(s.clone())),
                Err(CloudError::Api { status, message }) => Err(CloudError::Api {
                    status: *status,
                    message: message.clone(),
                }),
                Err(e) => Err(CloudError::Other(anyhow::anyhow!("{}", e))),
            }
        }
        async fn save_manifest(
            &self,
            _: &savevault_cloud::CloudCatalogManifest,
        ) -> savevault_cloud::CloudResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_negative_manifest_not_found_returns_explicit_error() {
        let conn = setup_test_db();
        let provider = MockErrorCloudProvider {
            manifest_res: Ok(None),
        };

        let res = reconstruct_catalog_from_cloud(&conn, &provider).await;
        assert!(matches!(res, Err(RecoveryError::ManifestNotFound)));
    }

    #[tokio::test]
    async fn test_negative_cloud_unauthenticated_returns_auth_error() {
        let conn = setup_test_db();
        let provider = MockErrorCloudProvider {
            manifest_res: Err(CloudError::Unauthenticated),
        };

        let res = reconstruct_catalog_from_cloud(&conn, &provider).await;
        assert!(matches!(res, Err(RecoveryError::Unauthenticated)));
    }

    #[tokio::test]
    async fn test_negative_cloud_offline_returns_network_error() {
        let conn = setup_test_db();
        let provider = MockErrorCloudProvider {
            manifest_res: Err(CloudError::Network("Conexão recusada".into())),
        };

        let res = reconstruct_catalog_from_cloud(&conn, &provider).await;
        assert!(matches!(res, Err(RecoveryError::Network(_))));
    }

    #[tokio::test]
    async fn test_negative_corrupted_manifest_returns_invalid_manifest() {
        let conn = setup_test_db();
        let provider = MockErrorCloudProvider {
            manifest_res: Err(CloudError::InvalidManifest("JSON sintaxe inválida".into())),
        };

        let res = reconstruct_catalog_from_cloud(&conn, &provider).await;
        assert!(matches!(res, Err(RecoveryError::InvalidManifest(_))));
    }

    #[tokio::test]
    async fn test_valid_manifest_with_zero_entries_returns_ok_zero() {
        let conn = setup_test_db();
        let provider = MockErrorCloudProvider {
            manifest_res: Ok(Some(savevault_cloud::CloudCatalogManifest {
                schema_version: 1,
                updated_at: Utc::now().to_rfc3339(),
                entries: vec![],
            })),
        };

        let res = reconstruct_catalog_from_cloud(&conn, &provider).await;
        assert_eq!(res.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_path_traversal_in_manifest_is_rejected_and_skipped() {
        let conn = setup_test_db();
        let provider = MockErrorCloudProvider {
            manifest_res: Ok(Some(savevault_cloud::CloudCatalogManifest {
                schema_version: 1,
                updated_at: Utc::now().to_rfc3339(),
                entries: vec![
                    savevault_cloud::CloudManifestEntry {
                        game_name: "Evil Game".to_string(),
                        platform: "pc".to_string(),
                        save_location_path: r"C:\Saves".to_string(),
                        relative_file_path: r"..\..\Windows\System32\evil.dll".to_string(),
                        file_hash: "evilhash".to_string(),
                        version: 1,
                        cloud_file_id: "evil-id".to_string(),
                        cloud_path: "evil".to_string(),
                        created_at: Utc::now().to_rfc3339(),
                    },
                    savevault_cloud::CloudManifestEntry {
                        game_name: "Good Game".to_string(),
                        platform: "pc".to_string(),
                        save_location_path: r"C:\Saves".to_string(),
                        relative_file_path: "good_save.dat".to_string(),
                        file_hash: "goodhash".to_string(),
                        version: 1,
                        cloud_file_id: "good-id".to_string(),
                        cloud_path: "good".to_string(),
                        created_at: Utc::now().to_rfc3339(),
                    },
                ],
            })),
        };

        let imported = reconstruct_catalog_from_cloud(&conn, &provider)
            .await
            .unwrap();
        // Apenas a entrada segura foi importada
        assert_eq!(imported, 1);

        let count: usize = conn
            .query_row(
                "SELECT count(*) FROM files WHERE path LIKE '%evil%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
}
