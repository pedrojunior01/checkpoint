//! Motor de Detecção e Resolução de Conflitos Multi-PC (CROSS-DEVICE-01).
//!
//! Gerencia divergências entre múltiplos dispositivos quando um usuário joga o mesmo jogo
//! em diferentes máquinas (ex: Desktop e Notebook) e alterações ocorrem offline ou concorrentemente.
//!
//! Princípios de Segurança:
//! 1. Nunca apagar ou sobrescrever saves sem criar Safety Backup prévio;
//! 2. Política padrão `KeepBoth`: preserva ambos os estados sem perda de progresso;
//! 3. Classificação causal: compara hash local, hash remoto e hash-base (último sync conhecido);
//! 4. Notificações imediatas via Windows Toast para manter o usuário informado.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use tracing::{info, warn};
use uuid::Uuid;

use savevault_core::config::ConflictPolicy;

/// Classificação do estado de sincronização entre o arquivo local e o snapshot na nuvem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictClassification {
    /// O arquivo local e o snapshot remoto possuem o mesmo hash BLAKE3 (sincronizados).
    InSync,
    /// O arquivo local possui alterações mais recentes, e o remoto permanece no estado base.
    LocalAhead,
    /// O snapshot remoto na nuvem foi atualizado por outro PC, e o local permanece inalterado.
    RemoteAhead,
    /// CONFLITO: Ambos os computadores alteraram o save de forma independente e os hashes divergem.
    Diverged,
}

/// Representa um save em análise de sincronização/conflito.
#[derive(Debug, Clone)]
pub struct ConflictItem {
    pub game_id: Uuid,
    pub game_name: String,
    pub file_id: Uuid,
    pub local_file_path: PathBuf,
    pub local_hash: String,
    pub local_mtime: DateTime<Utc>,
    pub remote_hash: Option<String>,
    pub remote_version: u32,
    pub remote_created_at: DateTime<Utc>,
    pub remote_drive_file_id: Option<String>,
    pub classification: ConflictClassification,
}

/// Resultado da execução da resolução de um conflito.
#[derive(Debug, Clone)]
pub enum ConflictResolutionResult {
    /// Ambos os arquivos foram preservados: local mantido e cópia remota arquivada na pasta `conflicts/`.
    KeptBoth {
        conflict_copy_path: PathBuf,
        message: String,
    },
    /// A versão local foi mantida como prioritária e preparada para envio na próxima sincronização.
    LocalKept {
        message: String,
    },
    /// A versão da nuvem foi aplicada no destino após criação de Safety Backup do arquivo local.
    RemoteApplied {
        safety_backup_path: PathBuf,
        message: String,
    },
    /// A versão mais recente no tempo venceu.
    NewerApplied {
        winner: String,
        message: String,
    },
    /// Requer intervenção manual do usuário via linha de comando ou interface.
    ManualRequired {
        message: String,
    },
}

/// Classifica o status de sincronização comparando hash local, remoto e base.
pub fn classify_conflict(
    local_hash: &str,
    remote_hash: &str,
    base_hash: Option<&str>,
) -> ConflictClassification {
    if local_hash.eq_ignore_ascii_case(remote_hash) {
        return ConflictClassification::InSync;
    }

    match base_hash {
        Some(base) if !base.trim().is_empty() => {
            let local_changed = !local_hash.eq_ignore_ascii_case(base);
            let remote_changed = !remote_hash.eq_ignore_ascii_case(base);

            match (local_changed, remote_changed) {
                (false, false) => ConflictClassification::InSync,
                (true, false) => ConflictClassification::LocalAhead,
                (false, true) => ConflictClassification::RemoteAhead,
                (true, true) => ConflictClassification::Diverged,
            }
        }
        _ => {
            // Sem base conhecida: se os hashes forem diferentes, considera divergência
            ConflictClassification::Diverged
        }
    }
}

/// Varre o banco de dados e o disco local em busca de divergências e conflitos multi-PC.
pub async fn scan_conflicts(conn: &Connection, backup_root: &Path) -> Result<Vec<ConflictItem>> {
    let mut stmt = conn.prepare(
        "SELECT f.id, f.path, f.hash, f.modified_at,
                sl.game_id, g.name,
                b.id, b.version, b.created_at, b.drive_file_id, b.cloud_path
         FROM files f
         JOIN save_locations sl ON f.save_location_id = sl.id
         JOIN games g ON sl.game_id = g.id
         LEFT JOIN backups b ON b.file_id = f.id AND (b.drive_file_id IS NOT NULL OR b.cloud_path IS NOT NULL)
         ORDER BY g.name ASC, b.version DESC",
    )?;

    let rows = stmt.query_map([], |row| {
        let file_id_str: String = row.get(0)?;
        let file_path_str: String = row.get(1)?;
        let base_hash: String = row.get(2)?;
        let modified_at_str: String = row.get(3)?;
        let game_id_str: String = row.get(4)?;
        let game_name: String = row.get(5)?;
        let _backup_id_str: Option<String> = row.get(6)?;
        let remote_version: Option<u32> = row.get(7)?;
        let remote_created_str: Option<String> = row.get(8)?;
        let remote_drive_file_id: Option<String> = row.get(9)?;
        let cloud_path: Option<String> = row.get(10)?;

        Ok((
            file_id_str,
            file_path_str,
            base_hash,
            modified_at_str,
            game_id_str,
            game_name,
            remote_version,
            remote_created_str,
            remote_drive_file_id,
            cloud_path,
        ))
    })?;

    let mut items = Vec::new();
    let mut seen_files = std::collections::HashSet::new();

    for r in rows {
        let (
            file_id_str,
            file_path_str,
            base_hash,
            modified_at_str,
            game_id_str,
            game_name,
            remote_version_opt,
            remote_created_str,
            remote_drive_file_id,
            _cloud_path,
        ) = r?;

        let file_id = Uuid::parse_str(&file_id_str).unwrap_or_default();
        let game_id = Uuid::parse_str(&game_id_str).unwrap_or_default();

        if !seen_files.insert(file_id) {
            continue; // Já processamos a versão mais recente deste arquivo
        }

        let local_file_path = PathBuf::from(&file_path_str);
        if !local_file_path.exists() {
            continue; // Arquivo físico de save não está presente no disco desta máquina
        }

        // Calcula o hash atual do arquivo local em disco
        let current_local_hash = match savevault_backup::hasher::hash_file(&local_file_path).await {
            Ok(h) => h,
            Err(_) => continue,
        };

        let local_mtime = DateTime::parse_from_rfc3339(&modified_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        let remote_version = remote_version_opt.unwrap_or(0);
        let remote_created_at = remote_created_str
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);

        // Se houver um snapshot correspondente no disco ou na nuvem
        let safe_game_name = savevault_backup::snapshot::sanitize_name(&game_name);
        let filename = local_file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "save.dat".into());

        let remote_snapshot_path = backup_root
            .join(&safe_game_name)
            .join(format!("v{}", remote_version))
            .join(&filename);

        let remote_hash = if remote_snapshot_path.exists() {
            savevault_backup::hasher::hash_file(&remote_snapshot_path).await.ok()
        } else {
            None
        };

        let classification = if let Some(ref r_hash) = remote_hash {
            classify_conflict(&current_local_hash, r_hash.as_str(), Some(&base_hash))
        } else if remote_drive_file_id.is_some() {
            // Existe na nuvem mas ainda não foi baixado localmente
            if current_local_hash.eq_ignore_ascii_case(&base_hash) {
                ConflictClassification::RemoteAhead
            } else {
                ConflictClassification::Diverged
            }
        } else {
            ConflictClassification::LocalAhead
        };

        items.push(ConflictItem {
            game_id,
            game_name,
            file_id,
            local_file_path,
            local_hash: current_local_hash,
            local_mtime,
            remote_hash,
            remote_version,
            remote_created_at,
            remote_drive_file_id,
            classification,
        });
    }

    Ok(items)
}

async fn resolve_prefer_local(item: &ConflictItem) -> Result<ConflictResolutionResult> {
    crate::notifications::notify_conflict_detected(
        &item.game_name,
        "Versão local mantida como definitiva.",
    );

    Ok(ConflictResolutionResult::LocalKept {
        message: format!(
            "A versão local de '{}' foi mantida como prioritária e será sincronizada na nuvem.",
            item.game_name
        ),
    })
}

async fn resolve_prefer_remote(
    item: &ConflictItem,
    backup_root: &Path,
    preferred_provider: Option<&str>,
) -> Result<ConflictResolutionResult> {
    // Cria Safety Backup do arquivo local atual antes de substituir
    let safety_bak_dir = backup_root
        .join(savevault_backup::snapshot::sanitize_name(&item.game_name))
        .join("safety_backups");
    tokio::fs::create_dir_all(&safety_bak_dir).await?;

    let filename = item
        .local_file_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "save.dat".into());

    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let safety_bak_path = safety_bak_dir.join(format!("{}.pre_conflict_remote_{}", filename, timestamp));
    tokio::fs::copy(&item.local_file_path, &safety_bak_path).await?;

    // Obtém snapshot remoto
    let safe_game_name = savevault_backup::snapshot::sanitize_name(&item.game_name);
    let candidate_versioned = backup_root
        .join(&safe_game_name)
        .join(format!("v{}", item.remote_version))
        .join(&filename);

    if candidate_versioned.exists() {
        tokio::fs::copy(&candidate_versioned, &item.local_file_path).await?;
    } else if let Some(ref remote_id) = item.remote_drive_file_id {
        crate::restore::download_snapshot_from_cloud(remote_id, &item.local_file_path, preferred_provider).await?;
    } else {
        return Err(anyhow!("Snapshot remoto não acessível para restauração."));
    }

    crate::notifications::notify_conflict_detected(
        &item.game_name,
        "Versão da nuvem aplicada. Safety backup do save local gerado com sucesso.",
    );

    Ok(ConflictResolutionResult::RemoteApplied {
        safety_backup_path: safety_bak_path.clone(),
        message: format!(
            "Versão remota da nuvem aplicada em '{}'. Safety backup local: '{}'",
            item.local_file_path.display(),
            safety_bak_path.display()
        ),
    })
}

/// Aplica a política de resolução selecionada para um item em conflito.
pub async fn resolve_conflict(
    policy: ConflictPolicy,
    item: &ConflictItem,
    backup_root: &Path,
    _conn: &Connection,
    preferred_provider: Option<&str>,
) -> Result<ConflictResolutionResult> {
    info!(
        "Resolvendo conflito para '{}' usando política {:?}",
        item.game_name, policy
    );

    match policy {
        ConflictPolicy::KeepBoth => {
            let safe_game_name = savevault_backup::snapshot::sanitize_name(&item.game_name);
            let conflicts_dir = backup_root.join(&safe_game_name).join("conflicts");
            tokio::fs::create_dir_all(&conflicts_dir).await?;

            let filename = item
                .local_file_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "save.dat".into());

            let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
            let conflict_filename = format!("{}.conflict-cloud-v{}_{}.bak", filename, item.remote_version, timestamp);
            let conflict_dest = conflicts_dir.join(&conflict_filename);

            // Se o snapshot remoto já existe localmente, copia para a pasta de conflitos
            let candidate_versioned = backup_root
                .join(&safe_game_name)
                .join(format!("v{}", item.remote_version))
                .join(&filename);

            if candidate_versioned.exists() {
                tokio::fs::copy(&candidate_versioned, &conflict_dest).await?;
            } else if let Some(ref remote_id) = item.remote_drive_file_id {
                // Baixa diretamente da nuvem para o arquivo de conflito
                crate::restore::download_snapshot_from_cloud(remote_id, &conflict_dest, preferred_provider).await?;
            } else {
                return Err(anyhow!(
                    "Não foi possível obter a versão remota para gerar cópia de conflito de '{}'",
                    item.game_name
                ));
            }

            crate::notifications::notify_conflict_detected(
                &item.game_name,
                "Cópia segura gerada na pasta de conflitos. Nenhum dado foi perdido.",
            );

            Ok(ConflictResolutionResult::KeptBoth {
                conflict_copy_path: conflict_dest.clone(),
                message: format!(
                    "Save local preservado. Cópia divergente da nuvem salva em: '{}'",
                    conflict_dest.display()
                ),
            })
        }

        ConflictPolicy::PreferLocal => resolve_prefer_local(item).await,

        ConflictPolicy::PreferRemote => {
            resolve_prefer_remote(item, backup_root, preferred_provider).await
        }

        ConflictPolicy::PreferNewest => {
            if item.local_mtime >= item.remote_created_at {
                info!("Conflito PreferNewest: save local é mais recente");
                let sub = resolve_prefer_local(item).await?;
                match sub {
                    ConflictResolutionResult::LocalKept { message } => {
                        Ok(ConflictResolutionResult::NewerApplied {
                            winner: "local".into(),
                            message: format!("Versão local é mais recente. {}", message),
                        })
                    }
                    other => Ok(other),
                }
            } else {
                info!("Conflito PreferNewest: snapshot remoto é mais recente");
                let sub = resolve_prefer_remote(item, backup_root, preferred_provider).await?;
                match sub {
                    ConflictResolutionResult::RemoteApplied { safety_backup_path: _, message } => {
                        Ok(ConflictResolutionResult::NewerApplied {
                            winner: "remote".into(),
                            message: format!("Versão remota é mais recente. {}", message),
                        })
                    }
                    other => Ok(other),
                }
            }
        }

        ConflictPolicy::Manual => {
            warn!("Conflito detectado para '{}', aguardando intervenção manual.", item.game_name);
            crate::notifications::notify_conflict_detected(
                &item.game_name,
                "Aguardando resolução manual do usuário (savevault conflict resolve).",
            );

            Ok(ConflictResolutionResult::ManualRequired {
                message: format!(
                    "Conflito em '{}' aguardando comando manual: savevault conflict resolve \"{}\"",
                    item.game_name, item.game_name
                ),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_identical_is_insync() {
        let hash = "a1b2c3d4e5f6";
        assert_eq!(
            classify_conflict(hash, hash, Some(hash)),
            ConflictClassification::InSync
        );
        assert_eq!(
            classify_conflict(hash, hash, None),
            ConflictClassification::InSync
        );
    }

    #[test]
    fn test_classify_local_ahead() {
        let base = "base_hash_000";
        let local = "local_modified_111";
        let remote = "base_hash_000";
        assert_eq!(
            classify_conflict(local, remote, Some(base)),
            ConflictClassification::LocalAhead
        );
    }

    #[test]
    fn test_classify_remote_ahead() {
        let base = "base_hash_000";
        let local = "base_hash_000";
        let remote = "remote_cloud_222";
        assert_eq!(
            classify_conflict(local, remote, Some(base)),
            ConflictClassification::RemoteAhead
        );
    }

    #[test]
    fn test_classify_diverged_conflict() {
        let base = "base_hash_000";
        let local = "pc_a_modified_111";
        let remote = "pc_b_modified_222";
        assert_eq!(
            classify_conflict(local, remote, Some(base)),
            ConflictClassification::Diverged
        );
        // Sem base conhecida e com hashes diferentes:
        assert_eq!(
            classify_conflict(local, remote, None),
            ConflictClassification::Diverged
        );
    }

    #[tokio::test]
    async fn test_resolve_keep_both_creates_conflict_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let backup_root = temp_dir.path().join("backups");
        tokio::fs::create_dir_all(&backup_root).await.unwrap();

        let game_dir = backup_root.join("The Witcher 3").join("v1");
        tokio::fs::create_dir_all(&game_dir).await.unwrap();
        let remote_file = game_dir.join("save.sav");
        tokio::fs::write(&remote_file, b"remote cloud save content").await.unwrap();

        let local_save = temp_dir.path().join("save.sav");
        tokio::fs::write(&local_save, b"local pc save content").await.unwrap();

        let item = ConflictItem {
            game_id: Uuid::new_v4(),
            game_name: "The Witcher 3".into(),
            file_id: Uuid::new_v4(),
            local_file_path: local_save.clone(),
            local_hash: "hash_local".into(),
            local_mtime: Utc::now(),
            remote_hash: Some("hash_remote".into()),
            remote_version: 1,
            remote_created_at: Utc::now(),
            remote_drive_file_id: None,
            classification: ConflictClassification::Diverged,
        };

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let result = resolve_conflict(ConflictPolicy::KeepBoth, &item, &backup_root, &conn, None).await.unwrap();

        match result {
            ConflictResolutionResult::KeptBoth { conflict_copy_path, .. } => {
                assert!(conflict_copy_path.exists());
                let content = tokio::fs::read_to_string(&conflict_copy_path).await.unwrap();
                assert_eq!(content, "remote cloud save content");
                // Local save deve permanecer intacto
                let local_content = tokio::fs::read_to_string(&local_save).await.unwrap();
                assert_eq!(local_content, "local pc save content");
            }
            other => panic!("Esperado KeptBoth, recebido: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_resolve_prefer_local() {
        let temp_dir = tempfile::tempdir().unwrap();
        let local_save = temp_dir.path().join("save.sav");
        tokio::fs::write(&local_save, b"local content").await.unwrap();

        let item = ConflictItem {
            game_id: Uuid::new_v4(),
            game_name: "Cyberpunk 2077".into(),
            file_id: Uuid::new_v4(),
            local_file_path: local_save.clone(),
            local_hash: "local_hash".into(),
            local_mtime: Utc::now(),
            remote_hash: Some("remote_hash".into()),
            remote_version: 2,
            remote_created_at: Utc::now(),
            remote_drive_file_id: None,
            classification: ConflictClassification::Diverged,
        };

        let backup_root = temp_dir.path().join("backups");
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let result = resolve_conflict(ConflictPolicy::PreferLocal, &item, &backup_root, &conn, None).await.unwrap();

        match result {
            ConflictResolutionResult::LocalKept { .. } => {
                let content = tokio::fs::read_to_string(&local_save).await.unwrap();
                assert_eq!(content, "local content");
            }
            other => panic!("Esperado LocalKept, recebido: {:?}", other),
        }
    }
}
