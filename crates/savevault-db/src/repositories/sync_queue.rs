use rusqlite::{Connection, params};
use anyhow::Result;
use savevault_core::models::{SyncQueueItem, SyncOperation, SyncStatus};
use uuid::Uuid;
use chrono::{DateTime, Utc};

pub struct SyncQueueRepo<'a> {
    pub conn: &'a Connection,
}

impl<'a> SyncQueueRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn enqueue(&self, item: &SyncQueueItem) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sync_queue (id, backup_id, operation, status, attempts, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                item.id.to_string(),
                item.backup_id.to_string(),
                operation_to_str(&item.operation),
                super::backups::sync_status_to_str(&item.status),
                item.attempts as i64,
                item.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn list_pending(&self) -> Result<Vec<SyncQueueItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, backup_id, operation, status, attempts, created_at
             FROM sync_queue WHERE status = 'pending'
             ORDER BY created_at ASC LIMIT 50"
        )?;
        let items = stmt.query_map([], row_to_item)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(items)
    }

    pub fn find_by_id(&self, id: Uuid) -> Result<Option<SyncQueueItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, backup_id, operation, status, attempts, created_at
             FROM sync_queue WHERE id = ?1"
        )?;
        let mut rows = stmt.query_map(params![id.to_string()], row_to_item)?;
        Ok(rows.next().transpose()?)
    }

    pub fn update_status(&self, id: Uuid, status: &SyncStatus, attempts: u32) -> Result<()> {
        self.conn.execute(
            "UPDATE sync_queue SET status = ?1, attempts = ?2 WHERE id = ?3",
            params![
                super::backups::sync_status_to_str(status),
                attempts as i64,
                id.to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn prune_completed_or_old(&self, days_to_keep: u32) -> Result<usize> {
        let cutoff = Utc::now() - chrono::Duration::days(days_to_keep as i64);
        let deleted = self.conn.execute(
            "DELETE FROM sync_queue WHERE status = 'completed' OR created_at < ?1",
            params![cutoff.to_rfc3339()],
        )?;
        Ok(deleted)
    }


    pub fn delete(&self, id: Uuid) -> Result<()> {
        self.conn.execute("DELETE FROM sync_queue WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }
}

fn row_to_item(row: &rusqlite::Row) -> rusqlite::Result<SyncQueueItem> {
    let id: String = row.get(0)?;
    let backup_id: String = row.get(1)?;
    let op_str: String = row.get(2)?;
    let status_str: String = row.get(3)?;
    let attempts: i64 = row.get(4)?;
    let created_str: String = row.get(5)?;
    Ok(SyncQueueItem {
        id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::new_v4()),
        backup_id: Uuid::parse_str(&backup_id).unwrap_or_else(|_| Uuid::new_v4()),
        operation: str_to_operation(&op_str),
        status: super::backups::str_to_sync_status(&status_str),
        attempts: attempts as u32,
        created_at: created_str.parse::<DateTime<Utc>>().unwrap_or_else(|_| Utc::now()),
    })
}

fn operation_to_str(op: &SyncOperation) -> &'static str {
    match op {
        SyncOperation::Upload => "upload",
        SyncOperation::Delete => "delete",
    }
}

fn str_to_operation(s: &str) -> SyncOperation {
    match s {
        "delete" => SyncOperation::Delete,
        _ => SyncOperation::Upload,
    }
}
