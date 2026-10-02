//! savevault-sync
//!
//! Motor de sincronização: consome a fila de sync da SQLite e envia os
//! snapshots locais para o provedor de cloud configurado.
//!
//! ## Pipeline de execução:
//!
//! ```text
//! sync_queue (status=pending) → upload para Drive → status=synced
//!                             ↘ falha (timeout/rede) → status=failed + tentativas++
//! ```
//!
//! ## Retry com backoff exponencial:
//! - Tentativa 1: imediata
//! - Tentativa 2: após ~2s
//! - Tentativa 3: após ~4s
//! - Tentativas > MAX_ATTEMPTS: marcado como `failed` permanentemente

use anyhow::{anyhow, Result};
use rusqlite::Connection;
use savevault_cloud::CloudProvider;
use savevault_core::models::{SyncOperation, SyncStatus};
use savevault_db::repositories::{BackupsRepo, SyncQueueRepo};
use std::path::PathBuf;
use tracing::{error, info, warn};
use uuid::Uuid;

/// Configurações do motor de sincronização.
#[derive(Debug, Clone)]
pub struct SyncEngineConfig {
    /// Máximo de tentativas antes de marcar permanentemente como falha.
    pub max_attempts: u32,
    /// Tempo base para o backoff exponencial (ms).
    pub base_backoff_ms: u64,
}

impl Default for SyncEngineConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_backoff_ms: 2_000,
        }
    }
}

impl SyncEngineConfig {
    /// Configuração acelerada para testes determinísticos sem longos sleeps.
    pub fn fast_for_tests() -> Self {
        Self {
            max_attempts: 3,
            base_backoff_ms: 1,
        }
    }
}

// ---------------------------------------------------------------------------
// SyncEngine
// ---------------------------------------------------------------------------

/// Motor de sincronização de backups com a cloud.
pub struct SyncEngine<P: CloudProvider> {
    provider: P,
    /// Diretório raiz onde os snapshots locais estão armazenados.
    /// Deve coincidir com o `backup_root` do `SnapshotManager`.
    backup_root: std::path::PathBuf,
    config: SyncEngineConfig,
}

impl<P: CloudProvider> SyncEngine<P> {
    /// Cria um novo `SyncEngine` com configurações padrão.
    pub fn new(provider: P, backup_root: std::path::PathBuf) -> Self {
        Self {
            provider,
            backup_root,
            config: SyncEngineConfig::default(),
        }
    }

    /// Cria um novo `SyncEngine` com configurações personalizadas.
    pub fn with_config(
        provider: P,
        backup_root: std::path::PathBuf,
        config: SyncEngineConfig,
    ) -> Self {
        Self {
            provider,
            backup_root,
            config,
        }
    }

    /// Processa todos os itens pendentes na fila de sync da SQLite.
    ///
    /// Tenta realizar o upload de cada item. Em caso de falha,
    /// incrementa o contador de tentativas e aplica backoff exponencial.
    /// Após `max_attempts`, o item é marcado como `Failed`.
    ///
    /// Retorna o número de itens sincronizados com sucesso.
    pub async fn process_queue(&self, conn: &Connection) -> Result<usize> {
        let queue_repo = SyncQueueRepo::new(conn);
        let backups_repo = BackupsRepo::new(conn);

        let pending = queue_repo.list_pending()?;

        if pending.is_empty() {
            info!("Fila de sync vazia — nada a fazer.");
            return Ok(0);
        }

        info!("Fila de sync: {} item(ns) pendente(s).", pending.len());
        let mut synced_count = 0usize;

        for item in &pending {
            // Aplica backoff exponencial baseado no número de tentativas
            if item.attempts > 0 && self.config.base_backoff_ms > 0 {
                let backoff_ms = self.config.base_backoff_ms * (1u64 << (item.attempts - 1).min(6));
                debug_or_warn(item.attempts, backoff_ms);
                tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)).await;
            }

            // Busca o backup associado ao item da fila diretamente pelo backup_id
            let backup = match backups_repo.find_by_id(item.backup_id)? {
                Some(b) => b,
                None => {
                    // Integridade: o backup referenciado não existe mais no banco.
                    // NUNCA remover silenciosamente: registra erro observável e marca como Failed.
                    error!(
                        "Integridade referencial violada: Backup {} referenciado na fila de sync não existe no banco.",
                        item.backup_id
                    );
                    let new_attempts = item.attempts + 1;
                    queue_repo.update_status(item.id, &SyncStatus::Failed, new_attempts)?;
                    continue;
                }
            };

            match item.operation {
                SyncOperation::Upload => {
                    let result = self
                        .perform_upload(backup.file_id, backup.version, conn)
                        .await;

                    match result {
                        Ok((cloud_path, drive_file_id)) => {
                            // Transação atômica SQLite: atualiza metadados + status para Synced + remove da fila
                            // Garante consistência local e idempotência mesmo em caso de crash do processo.
                            // Nota distribuída: a transação ACID local não desfaz operações já concluídas no Google Drive.
                            // A recuperação distribuída é garantida pela idempotência pré-upload do provider.
                            let tx = conn.unchecked_transaction()?;
                            tx.execute(
                                "UPDATE backups SET cloud_path = ?1, drive_file_id = ?2, sync_status = 'synced' WHERE id = ?3",
                                rusqlite::params![cloud_path, drive_file_id, backup.id.to_string()],
                            )?;
                            tx.execute(
                                "DELETE FROM sync_queue WHERE id = ?1",
                                rusqlite::params![item.id.to_string()],
                            )?;
                            tx.commit()?;

                            info!(
                                "Backup {} sincronizado com sucesso → {} (Drive ID: {})",
                                backup.id, cloud_path, drive_file_id
                            );
                            synced_count += 1;
                        }
                        Err(e) => {
                            let new_attempts = item.attempts + 1;
                            if new_attempts >= self.config.max_attempts {
                                error!(
                                    "Backup {} falhou {} vezes. Marcando como falha permanente: {}",
                                    backup.id, new_attempts, e
                                );
                                queue_repo.update_status(
                                    item.id,
                                    &SyncStatus::Failed,
                                    new_attempts,
                                )?;
                                backups_repo.update_sync_status(backup.id, &SyncStatus::Failed)?;
                            } else {
                                warn!(
                                    "Falha ao sincronizar backup {} (tentativa {}/{}): {}",
                                    backup.id, new_attempts, self.config.max_attempts, e
                                );
                                queue_repo.update_status(
                                    item.id,
                                    &SyncStatus::Pending,
                                    new_attempts,
                                )?;
                            }
                        }
                    }
                }

                SyncOperation::Delete => {
                    // Google Drive API requer o drive_file_id (ID opaco), e NUNCA o cloud_path lógico
                    if let Some(ref drive_file_id) = backup.drive_file_id {
                        match self.provider.delete(drive_file_id).await {
                            Ok(_) => {
                                info!("Arquivo removido do Drive (File ID: {})", drive_file_id);
                                // Verifica se o snapshot local ainda existe em disco
                                let local_snapshot_exists = match self.resolve_local_snapshot_path(
                                    backup.file_id,
                                    backup.version,
                                    conn,
                                ) {
                                    Ok((path, _, _)) => path.exists(),
                                    Err(_) => false,
                                };

                                let tx = conn.unchecked_transaction()?;
                                if local_snapshot_exists {
                                    // Backup local ainda existe: limpa os metadados remotos e marca status local
                                    tx.execute(
                                        "UPDATE backups SET cloud_path = NULL, drive_file_id = NULL, sync_status = 'pending_offline' WHERE id = ?1",
                                        rusqlite::params![backup.id.to_string()],
                                    )?;
                                    tx.execute(
                                        "DELETE FROM sync_queue WHERE id = ?1",
                                        rusqlite::params![item.id.to_string()],
                                    )?;
                                } else {
                                    // Backup local já foi podado do disco (pruned): remove definitivamente do banco
                                    tx.execute(
                                        "DELETE FROM sync_queue WHERE id = ?1",
                                        rusqlite::params![item.id.to_string()],
                                    )?;
                                    tx.execute(
                                        "DELETE FROM backups WHERE id = ?1",
                                        rusqlite::params![backup.id.to_string()],
                                    )?;
                                }
                                tx.commit()?;
                            }
                            Err(e) => {
                                warn!("Falha ao remover do Drive: {}", e);
                                queue_repo.update_status(
                                    item.id,
                                    &SyncStatus::Failed,
                                    item.attempts + 1,
                                )?;
                            }
                        }
                    } else {
                        // Sem drive_file_id: nunca foi sincronizado com sucesso ou é backup puramente local
                        queue_repo.delete(item.id)?;
                    }
                }
            }
        }

        info!(
            "Processamento da fila concluído: {}/{} sincronizado(s).",
            synced_count,
            pending.len()
        );
        Ok(synced_count)
    }

    /// Processa todos os itens pendentes na fila de sync liberando a trava do banco durante
    /// uploads de rede e retries com backoff sleep, evitando qualquer congelamento da aplicação ou da DEV-UI.
    pub async fn process_queue_unlocked(
        &self,
        db: &std::sync::Arc<tokio::sync::Mutex<Connection>>,
    ) -> Result<usize> {
        let pending = {
            let conn = db.lock().await;
            let queue_repo = SyncQueueRepo::new(&conn);
            queue_repo.list_pending()?
        };

        if pending.is_empty() {
            info!("Fila de sync vazia — nada a fazer.");
            return Ok(0);
        }

        info!("Fila de sync: {} item(ns) pendente(s).", pending.len());
        let mut synced_count = 0usize;

        for item in &pending {
            // Aplica backoff exponencial baseado no número de tentativas (SEM LOCK DO BANCO)
            if item.attempts > 0 && self.config.base_backoff_ms > 0 {
                let backoff_ms = self.config.base_backoff_ms * (1u64 << (item.attempts - 1).min(6));
                debug_or_warn(item.attempts, backoff_ms);
                tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)).await;
            }

            // Consulta metadados com lock temporário
            let lookup_result = {
                let conn = db.lock().await;
                let backups_repo = BackupsRepo::new(&conn);
                let backup_opt = backups_repo.find_by_id(item.backup_id)?;

                match backup_opt {
                    Some(b) => match item.operation {
                        SyncOperation::Upload => {
                            match self.resolve_local_snapshot_path(b.file_id, b.version, &conn) {
                                Ok((local_path, filename_owned, save_location_id)) => {
                                    let filename = filename_owned.as_str();
                                    let drive_filename = format!("v{}_{}", b.version, filename);
                                    let cloud_path = format!(
                                        "{}/{}/{}",
                                        save_location_id, b.file_id, drive_filename
                                    );
                                    Ok((b, Some((local_path, cloud_path)), None))
                                }
                                Err(e) => Err(e),
                            }
                        }
                        SyncOperation::Delete => {
                            let local_exists = match self
                                .resolve_local_snapshot_path(b.file_id, b.version, &conn)
                            {
                                Ok((path, _, _)) => path.exists(),
                                Err(_) => false,
                            };
                            Ok((b, None, Some(local_exists)))
                        }
                    },
                    None => {
                        error!(
                            "Integridade referencial violada: Backup {} referenciado na fila de sync não existe no banco.",
                            item.backup_id
                        );
                        let queue_repo = SyncQueueRepo::new(&conn);
                        let new_attempts = item.attempts + 1;
                        queue_repo.update_status(item.id, &SyncStatus::Failed, new_attempts)?;
                        continue;
                    }
                }
            };

            let (backup, upload_info, delete_info) = match lookup_result {
                Ok(info) => info,
                Err(e) => {
                    let conn = db.lock().await;
                    let queue_repo = SyncQueueRepo::new(&conn);
                    let backups_repo = BackupsRepo::new(&conn);
                    let new_attempts = item.attempts + 1;
                    if new_attempts >= self.config.max_attempts {
                        queue_repo.update_status(item.id, &SyncStatus::Failed, new_attempts)?;
                        backups_repo.update_sync_status(item.backup_id, &SyncStatus::Failed)?;
                    } else {
                        queue_repo.update_status(item.id, &SyncStatus::Pending, new_attempts)?;
                    }
                    warn!("Falha ao resolver snapshot para item {}: {}", item.id, e);
                    continue;
                }
            };

            match item.operation {
                SyncOperation::Upload => {
                    let (local_path, cloud_path) = upload_info.unwrap();
                    if !local_path.exists() {
                        let conn = db.lock().await;
                        let queue_repo = SyncQueueRepo::new(&conn);
                        let backups_repo = BackupsRepo::new(&conn);
                        let new_attempts = item.attempts + 1;
                        queue_repo.update_status(item.id, &SyncStatus::Failed, new_attempts)?;
                        backups_repo.update_sync_status(backup.id, &SyncStatus::Failed)?;
                        warn!(
                            "Snapshot local não existe no disco: '{}'",
                            local_path.display()
                        );
                        continue;
                    }

                    // Upload de rede assíncrono SEM LOCK DO BANCO
                    let upload_result = self.provider.upload(&local_path, &cloud_path).await;

                    let conn = db.lock().await;
                    let queue_repo = SyncQueueRepo::new(&conn);
                    let backups_repo = BackupsRepo::new(&conn);

                    match upload_result {
                        Ok(drive_file_id) => {
                            let tx = conn.unchecked_transaction()?;
                            tx.execute(
                                "UPDATE backups SET cloud_path = ?1, drive_file_id = ?2, sync_status = 'synced' WHERE id = ?3",
                                rusqlite::params![cloud_path, drive_file_id, backup.id.to_string()],
                            )?;
                            tx.execute(
                                "DELETE FROM sync_queue WHERE id = ?1",
                                rusqlite::params![item.id.to_string()],
                            )?;
                            tx.commit()?;

                            info!(
                                "Backup {} sincronizado com sucesso → {} (Drive ID: {})",
                                backup.id, cloud_path, drive_file_id
                            );
                            synced_count += 1;
                        }
                        Err(e) => {
                            let new_attempts = item.attempts + 1;
                            if new_attempts >= self.config.max_attempts {
                                error!(
                                    "Backup {} falhou {} vezes. Marcando como falha permanente: {}",
                                    backup.id, new_attempts, e
                                );
                                queue_repo.update_status(
                                    item.id,
                                    &SyncStatus::Failed,
                                    new_attempts,
                                )?;
                                backups_repo.update_sync_status(backup.id, &SyncStatus::Failed)?;
                            } else {
                                warn!(
                                    "Falha ao sincronizar backup {} (tentativa {}/{}): {}",
                                    backup.id, new_attempts, self.config.max_attempts, e
                                );
                                queue_repo.update_status(
                                    item.id,
                                    &SyncStatus::Pending,
                                    new_attempts,
                                )?;
                            }
                        }
                    }
                }
                SyncOperation::Delete => {
                    let local_snapshot_exists = delete_info.unwrap_or(false);
                    if let Some(ref drive_file_id) = backup.drive_file_id {
                        // Deleção de rede assíncrona SEM LOCK DO BANCO
                        let delete_result = self.provider.delete(drive_file_id).await;

                        let conn = db.lock().await;
                        let queue_repo = SyncQueueRepo::new(&conn);

                        match delete_result {
                            Ok(_) => {
                                info!("Arquivo removido da nuvem (File ID: {})", drive_file_id);
                                let tx = conn.unchecked_transaction()?;
                                if local_snapshot_exists {
                                    tx.execute(
                                        "UPDATE backups SET cloud_path = NULL, drive_file_id = NULL, sync_status = 'pending_offline' WHERE id = ?1",
                                        rusqlite::params![backup.id.to_string()],
                                    )?;
                                    tx.execute(
                                        "DELETE FROM sync_queue WHERE id = ?1",
                                        rusqlite::params![item.id.to_string()],
                                    )?;
                                } else {
                                    tx.execute(
                                        "DELETE FROM sync_queue WHERE id = ?1",
                                        rusqlite::params![item.id.to_string()],
                                    )?;
                                    tx.execute(
                                        "DELETE FROM backups WHERE id = ?1",
                                        rusqlite::params![backup.id.to_string()],
                                    )?;
                                }
                                tx.commit()?;
                            }
                            Err(e) => {
                                warn!("Falha ao remover da nuvem: {}", e);
                                queue_repo.update_status(
                                    item.id,
                                    &SyncStatus::Failed,
                                    item.attempts + 1,
                                )?;
                            }
                        }
                    } else {
                        let conn = db.lock().await;
                        let queue_repo = SyncQueueRepo::new(&conn);
                        queue_repo.delete(item.id)?;
                    }
                }
            }
        }

        info!(
            "Processamento da fila sem bloqueio concluído: {}/{} sincronizado(s).",
            synced_count,
            pending.len()
        );
        Ok(synced_count)
    }

    /// Localiza o arquivo físico do snapshot local no disco, suportando:
    /// 1. Novo padrão: `<backup_root>/<game_name>/latest/<filename>`
    /// 2. Versão pontual: `<backup_root>/<game_name>/v<version>/<filename>`
    /// 3. Versão legada por ID: `<backup_root>/<file_id>/v<version>/<filename>`
    /// 4. Legado por ID latest: `<backup_root>/<file_id>/latest/<filename>`
    fn resolve_local_snapshot_path(
        &self,
        file_id: Uuid,
        version: u32,
        conn: &Connection,
    ) -> Result<(PathBuf, String, Uuid)> {
        use savevault_db::repositories::{FilesRepo, GamesRepo, SaveLocationsRepo};

        let files_repo = FilesRepo::new(conn);
        let (filename_owned, save_location_id) = match files_repo.find_by_id(file_id)? {
            Some(entry) => {
                let name = entry
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("backup_{}.dat", file_id.as_simple()));
                (name, entry.save_location_id)
            }
            None => {
                return Err(anyhow!(
                    "FileEntry associado ({}) não encontrado no banco de dados.",
                    file_id
                ));
            }
        };

        let game_name = {
            let loc_repo = SaveLocationsRepo::new(conn);
            let games_repo = GamesRepo::new(conn);
            if let Ok(Some(loc)) = loc_repo.find_by_id(save_location_id) {
                if let Ok(Some(game)) = games_repo.find_by_id(loc.game_id) {
                    game.name
                } else {
                    "Desconhecido".to_string()
                }
            } else {
                "Desconhecido".to_string()
            }
        };

        let safe_game_name = savevault_backup::snapshot::sanitize_name(&game_name);

        let candidate_latest = self
            .backup_root
            .join(&safe_game_name)
            .join("latest")
            .join(&filename_owned);

        let candidate_versioned = self
            .backup_root
            .join(&safe_game_name)
            .join(format!("v{}", version))
            .join(&filename_owned);

        let legacy_versioned = self
            .backup_root
            .join(file_id.to_string())
            .join(format!("v{}", version))
            .join(&filename_owned);

        let legacy_latest = self
            .backup_root
            .join(file_id.to_string())
            .join("latest")
            .join(&filename_owned);

        let local_path = if candidate_versioned.exists() {
            candidate_versioned
        } else if candidate_latest.exists() {
            candidate_latest
        } else if legacy_versioned.exists() {
            legacy_versioned
        } else if legacy_latest.exists() {
            legacy_latest
        } else {
            candidate_latest
        };

        Ok((local_path, filename_owned, save_location_id))
    }

    /// Localiza o arquivo de snapshot local e executa o upload para o Drive.
    ///
    /// Retorna tupla `(cloud_path, drive_file_id)`:
    /// - `cloud_path`: identidade lógica formatada como `<save_location_id>/<file_id>/<filename>`.
    /// - `drive_file_id`: identidade remota opaca retornada pela API do Google Drive.
    async fn perform_upload(
        &self,
        file_id: Uuid,
        version: u32,
        conn: &Connection,
    ) -> Result<(String, String)> {
        let (local_path, filename_owned, save_location_id) =
            self.resolve_local_snapshot_path(file_id, version, conn)?;

        if !local_path.exists() {
            return Err(anyhow!(
                "Snapshot local não encontrado no disco: '{}'",
                local_path.display()
            ));
        }

        let filename = filename_owned.as_str();

        // Nome no Drive inclui versão para facilitar identificação visual
        let drive_filename = format!("v{}_{}", version, filename);

        // cloud_path hierárquico: <save_location_id>/<file_id>/<drive_filename>
        let cloud_path = format!("{}/{}/{}", save_location_id, file_id, drive_filename);

        let drive_file_id = self.provider.upload(&local_path, &cloud_path).await?;

        Ok((cloud_path, drive_file_id))
    }
}

// ---------------------------------------------------------------------------
// Helpers internos
// ---------------------------------------------------------------------------

fn debug_or_warn(attempts: u32, backoff_ms: u64) {
    if attempts == 1 {
        tracing::debug!("Aplicando backoff de {}ms antes de retry...", backoff_ms);
    } else {
        tracing::warn!(
            "Tentativa {}: aguardando {}ms antes de retry...",
            attempts + 1,
            backoff_ms
        );
    }
}

// ---------------------------------------------------------------------------
// API de alto nível (função de conveniência)
// ---------------------------------------------------------------------------

/// Processa a fila de sync usando a pasta local da nuvem ("local_folder").
pub async fn process_queue_active_provider(
    conn: &Connection,
    backup_root: std::path::PathBuf,
    provider_name: &str,
) -> Result<usize> {
    match provider_name.to_lowercase().as_str() {
        "none" | "disabled" | "offline" => Ok(0),
        _ => {
            let sync_path = std::env::var("SAVEVAULT_LOCAL_SYNC_PATH")
                .map(std::path::PathBuf::from)
                .ok()
                .or_else(|| {
                    savevault_cloud::detect_cloud_folders()
                        .into_iter()
                        .next()
                        .map(|d| d.savevault_folder)
                })
                .unwrap_or_else(|| backup_root.clone());

            let provider = savevault_cloud::LocalFolderProvider::new(sync_path);
            let engine = SyncEngine::new(provider, backup_root);
            engine.process_queue(conn).await
        }
    }
}

/// Processa a fila de sync sem reter o Mutex do banco durante a cópia dos snapshots para a pasta de nuvem.
pub async fn process_queue_active_provider_unlocked(
    db: &std::sync::Arc<tokio::sync::Mutex<Connection>>,
    backup_root: std::path::PathBuf,
    provider_name: &str,
) -> Result<usize> {
    match provider_name.to_lowercase().as_str() {
        "none" | "disabled" | "offline" => Ok(0),
        _ => {
            let sync_path = std::env::var("SAVEVAULT_LOCAL_SYNC_PATH")
                .map(std::path::PathBuf::from)
                .ok()
                .or_else(|| {
                    savevault_cloud::detect_cloud_folders()
                        .into_iter()
                        .next()
                        .map(|d| d.savevault_folder)
                })
                .unwrap_or_else(|| backup_root.clone());

            let provider = savevault_cloud::LocalFolderProvider::new(sync_path);
            let engine = SyncEngine::new(provider, backup_root);
            engine.process_queue_unlocked(db).await
        }
    }
}

#[cfg(test)]
mod tests;
