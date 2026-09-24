use rusqlite::{Connection, params};
use anyhow::Result;
use savevault_core::models::FileEntry;
use uuid::Uuid;
use std::path::PathBuf;
use chrono::{DateTime, Utc};

pub struct FilesRepo<'a> {
    pub conn: &'a Connection,
}

impl<'a> FilesRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn upsert(&self, entry: &FileEntry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO files (id, save_location_id, path, hash, size_bytes, modified_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET hash=excluded.hash, size_bytes=excluded.size_bytes, modified_at=excluded.modified_at",
            params![
                entry.id.to_string(),
                entry.save_location_id.to_string(),
                entry.path.to_string_lossy().to_string(),
                entry.hash,
                entry.size_bytes as i64,
                entry.modified_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn find_by_path(&self, path: &std::path::Path) -> Result<Option<FileEntry>> {
        let path_str = path.to_string_lossy().to_string();
        let mut stmt = self.conn.prepare(
            "SELECT id, save_location_id, path, hash, size_bytes, modified_at FROM files WHERE path = ?1"
        )?;
        let mut rows = stmt.query_map(params![path_str], row_to_entry)?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_by_location(&self, location_id: Uuid) -> Result<Vec<FileEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, save_location_id, path, hash, size_bytes, modified_at FROM files WHERE save_location_id = ?1"
        )?;
        let entries = stmt.query_map(params![location_id.to_string()], row_to_entry)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }

    pub fn delete(&self, id: Uuid) -> Result<()> {
        self.conn.execute("DELETE FROM files WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }
}

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<FileEntry> {
    let id: String = row.get(0)?;
    let loc_id: String = row.get(1)?;
    let path: String = row.get(2)?;
    let size: i64 = row.get(4)?;
    let modified_str: String = row.get(5)?;
    Ok(FileEntry {
        id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::new_v4()),
        save_location_id: Uuid::parse_str(&loc_id).unwrap_or_else(|_| Uuid::new_v4()),
        path: PathBuf::from(path),
        hash: row.get(3)?,
        size_bytes: size as u64,
        modified_at: modified_str.parse::<DateTime<Utc>>().unwrap_or_else(|_| Utc::now()),
    })
}
