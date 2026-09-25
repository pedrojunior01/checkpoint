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
use savevault_cloud::CloudProvider;
use savevault_core::models::{SyncOperation, SyncStatus};
use savevault_db::repositories::{BackupsRepo, SyncQueueRepo};
use rusqlite::Connection;
use tracing::{error, info, warn};
use uuid::Uuid;

/// Máximo de tentativas antes de marcar permanentemente como falha.
const MAX_ATTEMPTS: u32 = 5;
/// Tempo base para o backoff exponencial (ms).
const BASE_BACKOFF_MS: u64 = 2_000;

// ---------------------------------------------------------------------------
// SyncEngine
// ---------------------------------------------------------------------------

/// Motor de sincronização de backups com a cloud.
pub struct SyncEngine<P: CloudProvider> {
    provider: P,
    /// Diretório raiz onde os snapshots locais estão armazenados.
    /// Deve coincidir com o `backup_root` do `SnapshotManager`.
    backup_root: std::path::PathBuf,
}

impl<P: CloudProvider> SyncEngine<P> {
    /// Cria um novo `SyncEngine`.
    pub fn new(provider: P, backup_root: std::path::PathBuf) -> Self {
        Self {
            provider,
            backup_root,
        }
    }

    /// Processa todos os itens pendentes na fila de sync da SQLite.
    ///
    /// Tenta realizar o upload de cada item. Em caso de falha,
    /// incrementa o contador de tentativas e aplica backoff exponencial.
    /// Após `MAX_ATTEMPTS`, o item é marcado como `Failed`.
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
            if item.attempts > 0 {
                let backoff_ms = BASE_BACKOFF_MS * (1u64 << (item.attempts - 1).min(6));
                debug_or_warn(item.attempts, backoff_ms);
                tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)).await;
            }

            // Busca o backup associado ao item da fila
            let backups = backups_repo.list_by_file(item.backup_id)?;
            let backup = match backups.into_iter().find(|b| b.id == item.backup_id) {
                Some(b) => b,
                None => {
                    // Backup referenciado não existe mais: remove da fila silenciosamente
                    warn!(
                        "Backup {} não encontrado no banco. Removendo da fila.",
                        item.backup_id
                    );
                    queue_repo.delete(item.id)?;
                    continue;
                }
            };

            match item.operation {
                SyncOperation::Upload => {
                    let result = self
                        .perform_upload(item.backup_id, backup.version, conn)
                        .await;

                    match result {
                        Ok(cloud_path) => {
                            // Atualiza backup com o cloud_path e marca como Synced
                            update_backup_cloud_path(conn, backup.id, &cloud_path)?;
                            backups_repo.update_sync_status(backup.id, &SyncStatus::Synced)?;
                            queue_repo.delete(item.id)?;
                            info!(
                                "Backup {} sincronizado com sucesso → {}",
                                backup.id, cloud_path
                            );
                            synced_count += 1;
                        }
                        Err(e) => {
                            let new_attempts = item.attempts + 1;
                            if new_attempts >= MAX_ATTEMPTS {
                                error!(
                                    "Backup {} falhou {} vezes. Marcando como falha permanente: {}",
                                    backup.id, new_attempts, e
                                );
                                queue_repo.update_status(
                                    item.id,
                                    &SyncStatus::Failed,
                                    new_attempts,
                                )?;
                                backups_repo
                                    .update_sync_status(backup.id, &SyncStatus::Failed)?;
                            } else {
                                warn!(
                                    "Falha ao sincronizar backup {} (tentativa {}/{}): {}",
                                    backup.id, new_attempts, MAX_ATTEMPTS, e
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
                    if let Some(ref cloud_path) = backup.cloud_path {
                        match self.provider.delete(cloud_path).await {
                            Ok(_) => {
                                queue_repo.delete(item.id)?;
                                info!("Arquivo removido do Drive: {}", cloud_path);
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
                        // Sem cloud_path: nada para deletar no Drive
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

    /// Localiza o arquivo de snapshot local e executa o upload para o Drive.
    ///
    /// O `cloud_path` retornado é formatado como `<save_location_id>/<file_uuid>/<filename>`.
    async fn perform_upload(
        &self,
        file_uuid: Uuid,
        version: u32,
        conn: &Connection,
    ) -> Result<String> {
        use savevault_db::repositories::FilesRepo;

        let files_repo = FilesRepo::new(conn);

        // Busca o FileEntry real para obter o nome e o save_location_id
        let (filename_owned, save_location_id) = match files_repo.find_by_id(file_uuid)? {
            Some(entry) => {
                let name = entry
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("backup_{}.dat", file_uuid.as_simple()));
                (name, entry.save_location_id)
            }
            None => {
                // Arquivo removido do banco: usa fallback genérico
                let placeholder = format!("backup_{}.dat", file_uuid.as_simple());
                (placeholder, file_uuid)
            }
        };

        let filename = filename_owned.as_str();

        // Caminho local do snapshot: <backup_root>/<file_uuid>/v<version>/<filename>
        let local_path = self
            .backup_root
            .join(file_uuid.to_string())
            .join(format!("v{}", version))
            .join(filename);

        if !local_path.exists() {
            return Err(anyhow!(
                "Snapshot local não encontrado: '{}'",
                local_path.display()
            ));
        }

        // Nome no Drive inclui versão para facilitar identificação visual
        let drive_filename = format!("v{}_{}", version, filename);

        // cloud_path hierárquico: <save_location_id>/<file_uuid>/<drive_filename>
        let cloud_path = format!("{}/{}/{}", save_location_id, file_uuid, drive_filename);

        self.provider.upload(&local_path, &cloud_path).await?;

        Ok(cloud_path)
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



/// Atualiza o `cloud_path` de um backup no banco.
fn update_backup_cloud_path(conn: &Connection, backup_id: Uuid, cloud_path: &str) -> Result<()> {
    conn.execute(
        "UPDATE backups SET cloud_path = ?1 WHERE id = ?2",
        rusqlite::params![cloud_path, backup_id.to_string()],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// API de alto nível (função de conveniência)
// ---------------------------------------------------------------------------

/// Processa a fila de sync com o provedor Google Drive padrão.
///
/// Lê as credenciais das variáveis de ambiente:
/// - `SAVEVAULT_GOOGLE_CLIENT_ID`
/// - `SAVEVAULT_GOOGLE_CLIENT_SECRET`
///
/// Se as variáveis não estiverem definidas ou o usuário não estiver autenticado,
/// a função registra um aviso e retorna `Ok(0)` sem falhar — permitindo que o
/// SaveVault funcione normalmente em modo offline.
pub async fn process_queue_google_drive(
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
