use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use savevault_core::models::FileEntry;
use std::path::PathBuf;
use uuid::Uuid;

pub struct FilesRepo<'a> {
    pub conn: &'a Connection,
}

impl<'a> FilesRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn upsert(&self, entry: &FileEntry) -> Result<FileEntry> {
        let normalized_path = entry.path.to_string_lossy().replace('\\', "/");
        let mut stmt = self.conn.prepare(
            "INSERT INTO files (id, save_location_id, path, hash, size_bytes, modified_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(save_location_id, path COLLATE NOCASE) DO UPDATE SET
                 hash = excluded.hash,
                 size_bytes = excluded.size_bytes,
                 modified_at = excluded.modified_at
             RETURNING id, save_location_id, path, hash, size_bytes, modified_at",
        )?;

        let row = stmt.query_row(
            params![
                entry.id.to_string(),
                entry.save_location_id.to_string(),
                normalized_path,
                entry.hash,
                entry.size_bytes as i64,
                entry.modified_at.to_rfc3339(),
            ],
            row_to_entry,
        )?;

        Ok(row)
    }

    pub fn find_by_path(&self, path: &std::path::Path) -> Result<Option<FileEntry>> {
        let normalized_path = path.to_string_lossy().replace('\\', "/");
        let mut stmt = self.conn.prepare(
            "SELECT id, save_location_id, path, hash, size_bytes, modified_at FROM files WHERE path = ?1 COLLATE NOCASE"
        )?;
        let mut rows = stmt.query_map(params![normalized_path], row_to_entry)?;
        Ok(rows.next().transpose()?)
    }

    pub fn find_by_id(&self, id: Uuid) -> Result<Option<FileEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, save_location_id, path, hash, size_bytes, modified_at FROM files WHERE id = ?1"
        )?;
        let mut rows = stmt.query_map(params![id.to_string()], row_to_entry)?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_by_location(&self, location_id: Uuid) -> Result<Vec<FileEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, save_location_id, path, hash, size_bytes, modified_at FROM files WHERE save_location_id = ?1"
        )?;
        let rows = stmt.query_map(params![location_id.to_string()], row_to_entry)?;
        let mut entries = Vec::new();
        for r in rows {
            entries.push(r?);
        }
        Ok(entries)
    }

    pub fn count_by_all_locations(&self) -> Result<std::collections::HashMap<Uuid, i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT save_location_id, COUNT(*) FROM files GROUP BY save_location_id")?;
        let rows = stmt.query_map([], |row| {
            let id_str: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            let id = Uuid::parse_str(&id_str).unwrap_or_default();
            Ok((id, count))
        })?;
        let mut map = std::collections::HashMap::new();
        for r in rows {
            let (id, count) = r?;
            map.insert(id, count);
        }
        Ok(map)
    }

    pub fn delete(&self, id: Uuid) -> Result<()> {
        self.conn
            .execute("DELETE FROM files WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }
}

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<FileEntry> {
    let id_str: String = row.get(0)?;
    let loc_id_str: String = row.get(1)?;
    let path: String = row.get(2)?;
    let size: i64 = row.get(4)?;
    let modified_str: String = row.get(5)?;

    let id = Uuid::parse_str(&id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let save_location_id = Uuid::parse_str(&loc_id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let modified_at = modified_str.parse::<DateTime<Utc>>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
    })?;

    Ok(FileEntry {
        id,
        save_location_id,
        path: PathBuf::from(path),
        hash: row.get(3)?,
        size_bytes: size as u64,
        modified_at,
    })
}
