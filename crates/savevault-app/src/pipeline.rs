//! Pipeline de integração do SaveVault:
//! Conecta eventos estáveis de filesystem (`StableFileEvent`) à criação
//! de snapshots locais, versionamento, persistência SQLite e enfileiramento
//! na `SyncQueue` para sincronização em nuvem.

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

use savevault_backup::hasher::hash_file;
use savevault_backup::snapshot::SnapshotManager;
use savevault_core::models::{Backup, FileEntry, SyncOperation, SyncQueueItem, SyncStatus};
use savevault_db::repositories::{BackupsRepo, FilesRepo, SyncQueueRepo};
use savevault_monitor::events::{StableFileEvent, StableFileEventKind};

/// Consulta SQLite para deduplicação pelo hash e determinação do próximo número de versão.
pub fn check_existing_and_next_version(
    conn: &Connection,
    path: &Path,
    hash: &str,
) -> Result<Option<(Uuid, u32)>> {
    let files_repo = FilesRepo::new(conn);
    let existing_entry = files_repo.find_by_path(path)?;

    if let Some(ref entry) = existing_entry {
        if entry.hash == hash {
            tracing::debug!(
                "Arquivo {} inalterado (mesmo hash {}). Ignorando snapshot.",
                path.display(),
                hash
            );
            return Ok(None);
        }
    }

    let file_id = existing_entry.map(|e| e.id).unwrap_or_else(Uuid::new_v4);
    let backups_repo = BackupsRepo::new(conn);
    let existing_backups = backups_repo.list_by_file(file_id)?;
    let next_version = existing_backups.first().map(|b| b.version + 1).unwrap_or(1);

    Ok(Some((file_id, next_version)))
}

/// Persiste metadados do snapshot no SQLite e enfileira na `sync_queue`.
pub fn persist_snapshot_and_sync(
    conn: &Connection,
    file_id: Uuid,
    event: &StableFileEvent,
    hash: String,
    backup: &Backup,
    max_versions: u32,
) -> Result<(SyncQueueItem, Vec<Backup>)> {
    let files_repo = FilesRepo::new(conn);
    let file_entry = FileEntry {
        id: file_id,
        save_location_id: event.save_location_id,
        path: event.path.clone(),
        hash,
        size_bytes: event.size_bytes,
        modified_at: Utc::now(),
    };
    files_repo.upsert(&file_entry)?;

    let backups_repo = BackupsRepo::new(conn);
    backups_repo.insert(backup)?;

    let mut pruned_backups = Vec::new();
    if max_versions > 0 {
        match backups_repo.prune_old_versions(file_id, max_versions) {
            Ok(pruned) => pruned_backups = pruned,
            Err(e) => {
                tracing::warn!(
                    "Falha ao podar versões antigas do arquivo {}: {}",
                    file_id,
                    e
                );
            }
        }
    }

    let sync_queue_repo = SyncQueueRepo::new(conn);
    let item = SyncQueueItem {
        id: Uuid::new_v4(),
        backup_id: backup.id, // OBRIGATÓRIO: backup_id, NUNCA file_id
        operation: SyncOperation::Upload,
        status: SyncStatus::Pending,
        attempts: 0,
        created_at: Utc::now(),
    };
    sync_queue_repo.enqueue(&item)?;

    tracing::info!(
        "Backup #{} criado para {} (backup_id: {}, queue_id: {})",
        backup.version,
        event.path.display(),
        backup.id,
        item.id
    );

    Ok((item, pruned_backups))
}

/// Processa um único `StableFileEvent`, garantindo o ciclo completo:
/// hash → verificação/upsert FileEntry → snapshot em disco → Backup persistido → SyncQueueItem enfileirado.
///
/// Versão direta recebendo `&Connection` para compatibilidade com chamadas síncronas/testes existentes.
pub async fn handle_stable_event(
    conn: &Connection,
    event: &StableFileEvent,
    backup_root: &Path,
    max_versions: u32,
) -> Result<Option<(Backup, SyncQueueItem)>> {
    match event.kind {
        StableFileEventKind::Created | StableFileEventKind::Modified => {}
        StableFileEventKind::Removed => {
            tracing::info!(
                "Arquivo removido detectado: {} (save_location: {})",
                event.path.display(),
                event.save_location_id
            );
            return Ok(None);
        }
    }

    if !event.path.exists() {
        tracing::warn!(
            "Evento estável recebido mas arquivo não existe no disco: {}",
            event.path.display()
        );
        return Ok(None);
    }

    let hash = match hash_file(&event.path).await {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Falha ao calcular hash de {}: {}", event.path.display(), e);
            return Err(e);
        }
    };

    let (file_id, next_version) = match check_existing_and_next_version(conn, &event.path, &hash)? {
        Some(pair) => pair,
        None => return Ok(None),
    };

    let snapshot_manager = SnapshotManager::new(backup_root.to_path_buf(), max_versions);
    let backup = snapshot_manager
        .create_snapshot(file_id, &event.path, next_version)
        .await?;

    let (item, pruned_backups) =
        persist_snapshot_and_sync(conn, file_id, event, hash, &backup, max_versions)?;

    for pruned in pruned_backups {
        if let Err(e) = snapshot_manager.remove_snapshot_version(file_id, pruned.version).await {
            tracing::warn!(
                "Falha ao remover arquivo de snapshot físico versão {} de {}: {}",
                pruned.version,
                file_id,
                e
            );
        }
    }

    Ok(Some((backup, item)))
}

/// Processa um arquivo com concorrência segura (`Send`), adquirindo o lock do banco
/// apenas durante as consultas/escritas SQLite, mantendo-o livre durante o hashing e snapshot em disco.
pub async fn process_file(
    db: &tokio::sync::Mutex<Connection>,
    event: &StableFileEvent,
    backup_root: &Path,
    max_versions: u32,
) -> Result<Option<(Backup, SyncQueueItem)>> {
    match event.kind {
        StableFileEventKind::Created | StableFileEventKind::Modified => {}
        StableFileEventKind::Removed => {
            tracing::info!(
                "Arquivo removido detectado: {} (save_location: {})",
                event.path.display(),
                event.save_location_id
            );
            return Ok(None);
        }
    }

    if !event.path.exists() {
        tracing::warn!("Arquivo não existe no disco: {}", event.path.display());
        return Ok(None);
    }

    let hash = match hash_file(&event.path).await {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Falha ao calcular hash de {}: {}", event.path.display(), e);
            return Err(e);
        }
    };

    let (file_id, next_version) = {
        let conn = db.lock().await;
        match check_existing_and_next_version(&conn, &event.path, &hash)? {
            Some(pair) => pair,
            None => return Ok(None),
        }
    };

    let snapshot_manager = SnapshotManager::new(backup_root.to_path_buf(), max_versions);
    let backup = snapshot_manager
        .create_snapshot(file_id, &event.path, next_version)
        .await?;

    let (item, pruned_backups) = {
        let conn = db.lock().await;
        persist_snapshot_and_sync(&conn, file_id, event, hash, &backup, max_versions)?
    };

    for pruned in pruned_backups {
        if let Err(e) = snapshot_manager.remove_snapshot_version(file_id, pruned.version).await {
            tracing::warn!(
                "Falha ao remover arquivo de snapshot físico versão {} de {}: {}",
                pruned.version,
                file_id,
                e
            );
        }
    }

    Ok(Some((backup, item)))
}

/// Loop residente de processamento de eventos estáveis da aplicação.
/// Recebe conexão compartilhada via `Arc<Mutex<Connection>>` para concorrência segura com Tokio.
pub async fn run_pipeline(
    db: Arc<tokio::sync::Mutex<Connection>>,
    mut stable_rx: mpsc::Receiver<StableFileEvent>,
    backup_root: PathBuf,
    max_versions: u32,
    mut shutdown_rx: watch::Receiver<bool>,
    trigger_sync: bool,
) -> Result<()> {
    loop {
        tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    tracing::info!("Shutdown recebido no processador de eventos do pipeline.");
                    break;
                }
            }
            event_opt = stable_rx.recv() => {
                match event_opt {
                    Some(event) => {
                        let res = process_file(
                            &db,
                            &event,
                            &backup_root,
                            max_versions,
                        ).await;

                        match res {
                            Ok(Some((_backup, _queue_item))) => {
                                if trigger_sync {
                                    let conn = db.lock().await;
                                    if let Err(e) = savevault_sync::process_queue_google_drive(
                                        &conn,
                                        backup_root.clone(),
                                    ).await {
                                        tracing::debug!("Sync em nuvem durante evento: {}", e);
                                    }
                                }
                            }
                            Ok(None) => {}
                            Err(e) => {
                                tracing::error!(
                                    "Erro processando evento de {}: {}",
                                    event.path.display(),
                                    e
                                );
                            }
                        }
                    }
                    None => {
                        tracing::info!("Canal de eventos estáveis foi encerrado.");
                        break;
                    }
                }
            }
        }
    }

    // Drenagem final no encerramento limpo
    while let Ok(event) = stable_rx.try_recv() {
        let _ = process_file(&db, &event, &backup_root, max_versions).await;
    }

    Ok(())
}
