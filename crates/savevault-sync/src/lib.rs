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
use savevault_db::repositories::{BackupsRepo, FilesRepo, SyncQueueRepo};
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
                                let filename = {
                                    let files_repo = FilesRepo::new(conn);
                                    files_repo
                                        .find_by_id(backup.file_id)?
                                        .and_then(|f| {
                                            f.path
                                                .file_name()
                                                .map(|n| n.to_string_lossy().to_string())
                                        })
                                        .unwrap_or_else(|| "save.dat".to_string())
                                };
                                let local_snapshot = self
                                    .backup_root
                                    .join(backup.file_id.to_string())
                                    .join(format!("v{}", backup.version))
                                    .join(&filename);

                                let tx = conn.unchecked_transaction()?;
                                if local_snapshot.exists() {
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

    /// Processa a fila de sincronização utilizando uma referência a `tokio::sync::Mutex<Connection>`.
    ///
    /// Garante que o mutex do banco de dados NUNCA seja segurado durante:
    /// - Operações de rede HTTP (upload / download / delete);
    /// - Backoff e retries (`tokio::time::sleep`);
    /// - Token refresh / autenticação OAuth.
    ///
    /// O ciclo segue estritamente:
    /// 1. Lock DB curto -> lê próximo item pendente e caminhos -> libera DB;
    /// 2. Sleep de backoff (se necessário) -> sem lock;
    /// 3. Operação de nuvem (upload / delete) -> sem lock;
    /// 4. Lock DB curto -> transação atômica atualiza status / remove da fila -> libera DB.
    pub async fn process_queue_with_mutex(
        &self,
        db: &tokio::sync::Mutex<Connection>,
    ) -> Result<usize> {
        let mut synced_count = 0usize;

        loop {
            // Estágio 1: Leitura do próximo item com lock curto
            enum QueueAction {
                Upload {
                    item: savevault_core::models::SyncQueueItem,
                    backup: savevault_core::models::Backup,
                    local_path: std::path::PathBuf,
                    cloud_path: String,
                },
                Delete {
                    item: savevault_core::models::SyncQueueItem,
                    backup: savevault_core::models::Backup,
                    drive_file_id: String,
                    local_snapshot_exists: bool,
                },
                NoActionDeleteMissingId {
                    item_id: Uuid,
                },
                Empty,
            }

            let action = {
                let conn = db.lock().await;
                let queue_repo = SyncQueueRepo::new(&conn);
                let backups_repo = BackupsRepo::new(&conn);

                let pending = queue_repo.list_pending()?;
                if let Some(item) = pending.into_iter().next() {
                    let backup = match backups_repo.find_by_id(item.backup_id)? {
                        Some(b) => b,
                        None => {
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
                            match self.prepare_upload_paths(backup.file_id, backup.version, &conn) {
                                Ok((local_path, cloud_path)) => QueueAction::Upload {
                                    item,
                                    backup,
                                    local_path,
                                    cloud_path,
                                },
                                Err(e) => {
                                    error!("Erro preparando upload do backup {}: {}", backup.id, e);
                                    let new_attempts = item.attempts + 1;
                                    queue_repo.update_status(
                                        item.id,
                                        &SyncStatus::Failed,
                                        new_attempts,
                                    )?;
                                    continue;
                                }
                            }
                        }
                        SyncOperation::Delete => {
                            if let Some(drive_file_id) = backup.drive_file_id.clone() {
                                let filename = {
                                    let files_repo = FilesRepo::new(&conn);
                                    files_repo
                                        .find_by_id(backup.file_id)?
                                        .and_then(|f| {
                                            f.path
                                                .file_name()
                                                .map(|n| n.to_string_lossy().to_string())
                                        })
                                        .unwrap_or_else(|| "save.dat".to_string())
                                };
                                let local_snapshot = self
                                    .backup_root
                                    .join(backup.file_id.to_string())
                                    .join(format!("v{}", backup.version))
                                    .join(&filename);

                                QueueAction::Delete {
                                    item,
                                    backup,
                                    drive_file_id,
                                    local_snapshot_exists: local_snapshot.exists(),
                                }
                            } else {
                                QueueAction::NoActionDeleteMissingId { item_id: item.id }
                            }
                        }
                    }
                } else {
                    QueueAction::Empty
                }
            }; // Lock DB liberado aqui!

            match action {
                QueueAction::Empty => break,
                QueueAction::NoActionDeleteMissingId { item_id } => {
                    let conn = db.lock().await;
                    let queue_repo = SyncQueueRepo::new(&conn);
                    queue_repo.delete(item_id)?;
                }
                QueueAction::Upload {
                    item,
                    backup,
                    local_path,
                    cloud_path,
                } => {
                    // Backoff sleep sem segurar o lock do DB
                    if item.attempts > 0 && self.config.base_backoff_ms > 0 {
                        let backoff_ms =
                            self.config.base_backoff_ms * (1u64 << (item.attempts - 1).min(6));
                        debug_or_warn(item.attempts, backoff_ms);
                        tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)).await;
                    }

                    // Operação de rede HTTP no Google Drive sem segurar o lock do DB!
                    let upload_result = self.provider.upload(&local_path, &cloud_path).await;

                    // Estágio 3: Atualização atômica curta no DB
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
                QueueAction::Delete {
                    item,
                    backup,
                    drive_file_id,
                    local_snapshot_exists,
                } => {
                    // Operação de rede sem segurar o lock do DB!
                    let delete_result = self.provider.delete(&drive_file_id).await;

                    // Estágio 3: Atualização atômica curta no DB
                    let conn = db.lock().await;
                    let queue_repo = SyncQueueRepo::new(&conn);

                    match delete_result {
                        Ok(_) => {
                            info!("Arquivo removido do Drive (File ID: {})", drive_file_id);
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
                            warn!("Falha ao remover do Drive: {}", e);
                            queue_repo.update_status(
                                item.id,
                                &SyncStatus::Failed,
                                item.attempts + 1,
                            )?;
                        }
                    }
                }
            }
        }

        Ok(synced_count)
    }

    /// Prepara os caminhos locais e remotos para upload de um snapshot.
    pub fn prepare_upload_paths(
        &self,
        file_id: Uuid,
        version: u32,
        conn: &Connection,
    ) -> Result<(std::path::PathBuf, String)> {
        use savevault_db::repositories::FilesRepo;

        let files_repo = FilesRepo::new(conn);

        // Busca o FileEntry real para obter o nome e o save_location_id
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

        let filename = filename_owned.as_str();

        // Caminho local do snapshot: <backup_root>/<file_id>/v<version>/<filename>
        let local_path = self
            .backup_root
            .join(file_id.to_string())
            .join(format!("v{}", version))
            .join(filename);

        if !local_path.exists() {
            return Err(anyhow!(
                "Snapshot local não encontrado no disco: '{}'",
                local_path.display()
            ));
        }

        // Nome no Drive inclui versão para facilitar identificação visual
        let drive_filename = format!("v{}_{}", version, filename);

        // cloud_path hierárquico: <save_location_id>/<file_id>/<drive_filename>
        let cloud_path = format!("{}/{}/{}", save_location_id, file_id, drive_filename);

        Ok((local_path, cloud_path))
    }

    /// Executa o upload físico para a cloud.
    async fn perform_upload(
        &self,
        file_id: Uuid,
        version: u32,
        conn: &Connection,
    ) -> Result<(String, String)> {
        let (local_path, cloud_path) = self.prepare_upload_paths(file_id, version, conn)?;
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

/// Processa a fila de sync com o provedor Google Drive padrão recebendo um `Mutex<Connection>`.
///
/// O mutex NUNCA é segurado durante requisições de rede ou backoff sleeps.
pub async fn process_queue_google_drive(
    db: &tokio::sync::Mutex<Connection>,
    backup_root: std::path::PathBuf,
) -> Result<usize> {
    let provider = match savevault_cloud::GoogleDriveProvider::from_env() {
        Some(p) => p,
        None => {
            warn!(
                "Credenciais do Google Drive não encontradas (env vars ausentes). \
                Sincronização desativada."
            );
            return Ok(0);
        }
    };

    if !provider.is_authenticated() {
        warn!(
            "Usuário não autenticado no Google Drive. \
            Execute: cargo run --bin savevault -- auth login"
        );
        return Ok(0);
    }

    let engine = SyncEngine::new(provider, backup_root);
    engine.process_queue_with_mutex(db).await
}

/// Processa a fila de sync de forma pontual a partir de uma conexão já aberta (ex.: comandos CLI).
pub async fn process_queue_google_drive_conn(
    conn: &Connection,
    backup_root: std::path::PathBuf,
) -> Result<usize> {
    let provider = match savevault_cloud::GoogleDriveProvider::from_env() {
        Some(p) => p,
        None => {
            warn!(
                "Credenciais do Google Drive não encontradas (env vars ausentes). \
                Sincronização desativada."
            );
            return Ok(0);
        }
    };

    if !provider.is_authenticated() {
        warn!(
            "Usuário não autenticado no Google Drive. \
            Execute: cargo run --bin savevault -- auth login"
        );
        return Ok(0);
    }

    let engine = SyncEngine::new(provider, backup_root);
    engine.process_queue(conn).await
}

#[cfg(test)]
mod tests;
