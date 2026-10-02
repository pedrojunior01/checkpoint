use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use savevault_core::models::{Backup, SyncStatus};
use uuid::Uuid;

pub struct BackupsRepo<'a> {
    pub conn: &'a Connection,
}

impl<'a> BackupsRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn insert(&self, backup: &Backup) -> Result<()> {
        self.conn.execute(
            "INSERT INTO backups (id, file_id, version, cloud_path, drive_file_id, sync_status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                backup.id.to_string(),
                backup.file_id.to_string(),
                backup.version as i64,
                backup.cloud_path,
                backup.drive_file_id,
                sync_status_to_str(&backup.sync_status),
                backup.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn list_by_file(&self, file_id: Uuid) -> Result<Vec<Backup>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_id, version, cloud_path, drive_file_id, sync_status, created_at
             FROM backups WHERE file_id = ?1 ORDER BY version DESC",
        )?;
        let rows = stmt.query_map(params![file_id.to_string()], row_to_backup)?;
        let mut backups = Vec::new();
        for r in rows {
            backups.push(r?);
        }
        Ok(backups)
    }

    pub fn find_by_id(&self, id: Uuid) -> Result<Option<Backup>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_id, version, cloud_path, drive_file_id, sync_status, created_at
             FROM backups WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id.to_string()], row_to_backup)?;
        Ok(rows.next().transpose()?)
    }

    pub fn prune_old_records(&self, days_to_keep: u32) -> Result<usize> {
        let cutoff = Utc::now() - chrono::Duration::days(days_to_keep as i64);
        let deleted = self.conn.execute(
            "DELETE FROM backups WHERE created_at < ?1",
            params![cutoff.to_rfc3339()],
        )?;
        Ok(deleted)
    }


    pub fn update_sync_status(&self, id: Uuid, status: &SyncStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE backups SET sync_status = ?1 WHERE id = ?2",
            params![sync_status_to_str(status), id.to_string()],
        )?;
        Ok(())
    }

    /// Atualiza os metadados de nuvem (`cloud_path` lógico e `drive_file_id` remoto).
    pub fn update_cloud_metadata(
        &self,
        id: Uuid,
        cloud_path: &str,
        drive_file_id: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE backups SET cloud_path = ?1, drive_file_id = ?2 WHERE id = ?3",
            params![cloud_path, drive_file_id, id.to_string()],
        )?;
        Ok(())
    }

    /// Remove um backup específico pelo ID.
    pub fn delete(&self, id: Uuid) -> Result<()> {
        self.conn
            .execute("DELETE FROM backups WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }

    /// Limpa os metadados de nuvem (`cloud_path` e `drive_file_id`) e atualiza o `sync_status`.
    pub fn clear_cloud_metadata(&self, id: Uuid, status: &SyncStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE backups SET cloud_path = NULL, drive_file_id = NULL, sync_status = ?1 WHERE id = ?2",
            params![sync_status_to_str(status), id.to_string()],
        )?;
        Ok(())
    }

    /// Remove versões antigas mantendo apenas as N mais recentes por file_id.
    ///
    /// Preservação da referência remota:
    /// - Se o backup antigo NÃO possui `drive_file_id`, é deletado imediatamente do banco de dados.
    /// - Se o backup antigo POSSUI `drive_file_id`, a referência remota é preservada enfileirando
    ///   uma operação `SyncOperation::Delete` na `sync_queue`. O registro do backup é mantido no
    ///   SQLite para preservar a integridade referencial da chave estrangeira e a identidade do
    ///   objeto remoto até que a deleção na nuvem seja confirmada pelo `SyncEngine`.
    pub fn prune_old_versions(&self, file_id: Uuid, keep: u32) -> Result<Vec<Backup>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_id, version, cloud_path, drive_file_id, sync_status, created_at
             FROM backups WHERE file_id = ?1 AND id NOT IN (
                 SELECT id FROM backups WHERE file_id = ?1 ORDER BY version DESC LIMIT ?2
             )",
        )?;
        let rows = stmt.query_map(params![file_id.to_string(), keep as i64], row_to_backup)?;
        let mut old_backups = Vec::new();
        for r in rows {
            old_backups.push(r?);
        }

        let tx = self.conn.unchecked_transaction()?;

        for b in &old_backups {
            if let Some(ref _drive_id) = b.drive_file_id {
                // Preserva a referência remota: enfileira SyncOperation::Delete
                // se ainda não estiver na fila.
                let mut check_stmt = tx.prepare(
                    "SELECT COUNT(*) FROM sync_queue WHERE backup_id = ?1 AND operation = 'delete'",
                )?;
                let count: i64 =
                    check_stmt.query_row(params![b.id.to_string()], |row| row.get(0))?;
                if count == 0 {
                    // Cancela qualquer upload pendente deste backup antigo para não enviar versão já podada
                    tx.execute(
                        "DELETE FROM sync_queue WHERE backup_id = ?1 AND operation = 'upload'",
                        params![b.id.to_string()],
                    )?;
                    tx.execute(
                        "INSERT INTO sync_queue (id, backup_id, operation, status, attempts, created_at)
                         VALUES (?1, ?2, 'delete', 'pending', 0, ?3)",
                        params![
                            Uuid::new_v4().to_string(),
                            b.id.to_string(),
                            Utc::now().to_rfc3339(),
                        ],
                    )?;
                }
            } else {
                // Sem drive_file_id: nunca esteve na nuvem, pode ser removido imediatamente
                tx.execute(
                    "DELETE FROM backups WHERE id = ?1",
                    params![b.id.to_string()],
                )?;
            }
        }

        tx.commit()?;
        Ok(old_backups)
    }
}

fn row_to_backup(row: &rusqlite::Row) -> rusqlite::Result<Backup> {
    let id_str: String = row.get(0)?;
    let file_id_str: String = row.get(1)?;
    let version: i64 = row.get(2)?;
    let cloud_path: Option<String> = row.get(3)?;
    let drive_file_id: Option<String> = row.get(4)?;
    let status_str: String = row.get(5)?;
    let created_str: String = row.get(6)?;

    let id = Uuid::parse_str(&id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let file_id = Uuid::parse_str(&file_id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let created_at = created_str.parse::<DateTime<Utc>>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e))
    })?;

    Ok(Backup {
        id,
        file_id,
        version: version as u32,
        cloud_path,
        drive_file_id,
        sync_status: str_to_sync_status(&status_str),
        created_at,
    })
}

pub fn sync_status_to_str(s: &SyncStatus) -> &'static str {
    match s {
        SyncStatus::Pending => "pending",
        SyncStatus::Synced => "synced",
        SyncStatus::Failed => "failed",
        SyncStatus::PendingOffline => "pending_offline",
    }
}

pub fn str_to_sync_status(s: &str) -> SyncStatus {
    match s {
        "synced" => SyncStatus::Synced,
        "failed" => SyncStatus::Failed,
        "pending_offline" => SyncStatus::PendingOffline,
        _ => SyncStatus::Pending,
    }
}
