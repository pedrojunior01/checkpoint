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
) -> Result<SyncQueueItem> {
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

    if max_versions > 0 {
        if let Err(e) = backups_repo.prune_old_versions(file_id, max_versions) {
            tracing::warn!(
                "Falha ao podar versões antigas do arquivo {}: {}",
                file_id,
                e
            );
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

    Ok(item)
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

    // Se o caminho estiver atualmente sob processo de restauração ativa,
    // suprime o evento para prevenir concorrência com o restore e evitar
    // criação desnecessária de um novo snapshot do arquivo que acabou de ser restaurado (SV-ADV-010).
    if crate::restore::is_path_under_active_restore(&event.path) {
        tracing::debug!(
            "Arquivo {} está sob restauração ativa. Suprimindo evento do watcher.",
            event.path.display()
        );
        return Ok(None);
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

    let item = persist_snapshot_and_sync(conn, file_id, event, hash, &backup, max_versions)?;

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

    // Se o caminho estiver atualmente sob processo de restauração ativa,
    // suprime o evento para prevenir concorrência com o restore e evitar
    // criação desnecessária de um novo snapshot do arquivo que acabou de ser restaurado (SV-ADV-010).
    if crate::restore::is_path_under_active_restore(&event.path) {
        tracing::debug!(
            "Arquivo {} está sob restauração ativa. Suprimindo evento do watcher.",
            event.path.display()
        );
        return Ok(None);
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

    let item = {
        let conn = db.lock().await;
        persist_snapshot_and_sync(&conn, file_id, event, hash, &backup, max_versions)?
    };

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
                                    if let Err(e) = savevault_sync::process_queue_google_drive(
                                        &db,
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

/// Sincroniza um snapshot específico para a pasta local sincronizada do Google Drive para computador.
///
/// Garante:
/// 1. Verificação prévia da acessibilidade da pasta raiz;
/// 2. Estrutura por plataforma e jogo (<DriveRoot>/SaveVault/<Platform>/<Game>/);
/// 3. Escrita atômica via arquivo temporário no mesmo volume do destino;
/// 4. Verificação de integridade via BLAKE3 e conferência de tamanho;
/// 5. Atualização do status para `SyncStatus::CopiedToDriveFolder`;
/// 6. Caso a pasta do Drive esteja indisponível, mantém o snapshot local 100% intacto e marca status pendente offline.
pub fn sync_backup_to_drive_folder(
    conn: &Connection,
    backup_id: Uuid,
    backup_root: &Path,
    drive_dir: &Path,
) -> Result<bool> {
    use savevault_backup::storage::{BackupSnapshotParams, BackupStorage, FilesystemBackupStorage};
    use savevault_db::repositories::{
        BackupsRepo, FilesRepo, GamesRepo, SaveLocationsRepo, SyncQueueRepo,
    };

    let backups_repo = BackupsRepo::new(conn);
    let backup = match backups_repo.find_by_id(backup_id)? {
        Some(b) => b,
        None => return Ok(false),
    };

    let files_repo = FilesRepo::new(conn);
    let file_entry = match files_repo.find_by_id(backup.file_id)? {
        Some(f) => f,
        None => return Ok(false),
    };

    let locs_repo = SaveLocationsRepo::new(conn);
    let loc = match locs_repo.find_by_id(file_entry.save_location_id)? {
        Some(l) => l,
        None => return Ok(false),
    };

    let games_repo = GamesRepo::new(conn);
    let game = match games_repo.find_by_id(loc.game_id)? {
        Some(g) => g,
        None => return Ok(false),
    };

    let file_name = file_entry
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("save.dat");

    let local_snapshot_path = backup_root
        .join(backup.file_id.to_string())
        .join(format!("v{}", backup.version))
        .join(file_name);

    let local_meta = match std::fs::metadata(&local_snapshot_path) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(
                "Snapshot local não encontrado ou inacessível em {}: {}",
                local_snapshot_path.display(),
                e
            );
            return Ok(false);
        }
    };
    let local_size = local_meta.len();

    let storage = FilesystemBackupStorage::new(drive_dir.to_path_buf());
    let folder_status = storage.validate_folder();
    if !folder_status.is_available() {
        tracing::info!(
            "Pasta do Drive indisponível no momento: {:?}. Mantendo snapshot local.",
            folder_status
        );
        let _ = backups_repo.update_sync_status(backup.id, &SyncStatus::PendingOffline);
        return Ok(false);
    }

    // Se este backup for a versão mais recente cadastrada para o arquivo,
    // o hash em `file_entry.hash` corresponde ao seu conteúdo.
    // Caso contrário (versão anterior), não podemos usar `file_entry.hash` (que reflete o estado mais recente).
    let is_latest = backups_repo
        .list_by_file(backup.file_id)
        .ok()
        .and_then(|list| list.first().map(|latest| latest.version == backup.version))
        .unwrap_or(true);

    let expected_hash = if is_latest {
        Some(file_entry.hash)
    } else {
        None
    };

    let params = BackupSnapshotParams {
        local_snapshot_path,
        platform: game.platform,
        game_name: game.name,
        game_id: Some(game.id),
        file_id: Some(file_entry.id),
        backup_id: Some(backup.id),
        version: backup.version,
        file_name: file_name.to_string(),
        expected_hash,
        expected_size: Some(local_size),
    };

    match storage.backup_snapshot(params) {
        Ok(_) => {
            backups_repo.update_sync_status(backup.id, &SyncStatus::CopiedToDriveFolder)?;
            let queue_repo = SyncQueueRepo::new(conn);
            if let Ok(pending) = queue_repo.list_pending() {
                for item in pending {
                    if item.backup_id == backup.id {
                        let _ = queue_repo.delete(item.id);
                    }
                }
            }
            tracing::info!(
                "Backup #{} de '{}' copiado com sucesso para a pasta do Drive",
                backup.version,
                file_name
            );
            Ok(true)
        }
        Err(e) => {
            tracing::error!("Falha ao copiar snapshot para pasta do Drive: {}", e);
            let _ = backups_repo.update_sync_status(backup.id, &SyncStatus::PendingOffline);
            Ok(false)
        }
    }
}

/// Sincroniza todos os backups pendentes para a pasta do Drive configurada.
/// Utiliza reconciliação leve (existência e tamanho antes de cópia).
pub fn sync_all_pending_to_drive_folder(
    conn: &Connection,
    backup_root: &Path,
    drive_dir: &Path,
) -> Result<usize> {
    use savevault_backup::storage::FilesystemBackupStorage;
    use savevault_db::repositories::{BackupsRepo, FilesRepo, GamesRepo, SaveLocationsRepo};

    let storage = FilesystemBackupStorage::new(drive_dir.to_path_buf());
    if !storage.validate_folder().is_available() {
        return Ok(0);
    }

    let files_repo = FilesRepo::new(conn);
    let games_repo = GamesRepo::new(conn);
    let locs_repo = SaveLocationsRepo::new(conn);
    let backups_repo = BackupsRepo::new(conn);

    let all_files = files_repo.list_all()?;
    let mut copied_count = 0usize;

    for f in all_files {
        let loc = match locs_repo.find_by_id(f.save_location_id)? {
            Some(l) => l,
            None => continue,
        };
        let game = match games_repo.find_by_id(loc.game_id)? {
            Some(g) => g,
            None => continue,
        };

        let backups = backups_repo.list_by_file(f.id)?;
        for b in backups {
            if b.sync_status == SyncStatus::CopiedToDriveFolder {
                continue;
            }

            let file_name = f
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("save.dat");
            let paths = storage.get_game_paths(&game.platform, &game.name, Some(game.id));
            let drive_file = paths
                .snapshots_dir
                .join(format!("v{}", b.version))
                .join(file_name);
            let meta_file = paths
                .metadata_dir
                .join(format!("v{}_{}.json", b.version, file_name));

            let local_snapshot_path = backup_root
                .join(b.file_id.to_string())
                .join(format!("v{}", b.version))
                .join(file_name);

            // Se o snapshot e seus metadados já existem no Drive e o tamanho bate com o snapshot local,
            // considera já sincronizado e atualiza o status diretamente.
            if drive_file.exists() && meta_file.exists() && local_snapshot_path.exists() {
                if let (Ok(d_meta), Ok(l_meta)) = (
                    std::fs::metadata(&drive_file),
                    std::fs::metadata(&local_snapshot_path),
                ) {
                    if d_meta.len() == l_meta.len() {
                        let _ =
                            backups_repo.update_sync_status(b.id, &SyncStatus::CopiedToDriveFolder);
                        copied_count += 1;
                        continue;
                    }
                }
            }

            // Caso contrário, copia atomicamente com validação de hash e tamanho
            if let Ok(true) = sync_backup_to_drive_folder(conn, b.id, backup_root, drive_dir) {
                copied_count += 1;
            }
        }
    }

    Ok(copied_count)
}
