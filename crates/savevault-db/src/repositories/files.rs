use anyhow::{Context, Result};
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

    pub fn upsert(&self, entry: &FileEntry) -> Result<()> {
        let norm_path = savevault_core::path::normalize_windows_path(&entry.path)
            .map_err(|e| anyhow::anyhow!("Caminho de arquivo inválido: {}", e))?;
        let path_str = norm_path
            .to_str()
            .context("Caminho contém caracteres não-UTF8")?;

        // 1. Verifica se já existe por equivalência canônica completa (incluindo Unicode não-ASCII e NFC/NFD)
        if let Some(existing) = self.find_by_path(&entry.path)? {
            self.conn.execute(
                "UPDATE files SET hash = ?1, size_bytes = ?2, modified_at = ?3 WHERE id = ?4",
                params![
                    entry.hash,
                    entry.size_bytes as i64,
                    entry.modified_at.to_rfc3339(),
                    existing.id.to_string(),
                ],
            )?;
            return Ok(());
        }

        // 2. Se não existe, insere normalmente com ON CONFLICT(path COLLATE NOCASE)
        self.conn.execute(
            "INSERT INTO files (id, save_location_id, path, hash, size_bytes, modified_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(path COLLATE NOCASE) DO UPDATE SET
                 hash = excluded.hash,
                 size_bytes = excluded.size_bytes,
                 modified_at = excluded.modified_at",
            params![
                entry.id.to_string(),
                entry.save_location_id.to_string(),
                path_str,
                entry.hash,
                entry.size_bytes as i64,
                entry.modified_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn find_by_path(&self, path: &std::path::Path) -> Result<Option<FileEntry>> {
        let norm_path = savevault_core::path::normalize_windows_path(path)
            .map_err(|e| anyhow::anyhow!("Caminho de arquivo inválido: {}", e))?;
        let path_str = norm_path
            .to_str()
            .context("Caminho contém caracteres não-UTF8")?;

        let mut stmt = self.conn.prepare(
            "SELECT id, save_location_id, path, hash, size_bytes, modified_at FROM files WHERE path = ?1 COLLATE NOCASE"
        )?;
        let mut rows = stmt.query_map(params![path_str], row_to_entry)?;
        if let Some(res) = rows.next() {
            return Ok(Some(res?));
        }

        // Fallback para equivalência semântica Unicode / Windows completa:
        // Caso o SQLite COLLATE NOCASE (que só dobra ASCII A-Z) não encontre
        // caminhos com diacríticos, acentos ou variações Unicode não-ASCII (ex: MÿGame vs mÿgame):
        let canon_target = savevault_core::path::canonicalize_windows_path(path).map_err(|e| {
            anyhow::anyhow!("Caminho de arquivo inválido para canonicalização: {}", e)
        })?;
        let mut all_stmt = self.conn.prepare(
            "SELECT id, save_location_id, path, hash, size_bytes, modified_at FROM files",
        )?;
        let all_rows = all_stmt.query_map([], row_to_entry)?;
        for res in all_rows {
            let entry = res?;
            if let Ok(c) = savevault_core::path::canonicalize_windows_path(&entry.path) {
                if c == canon_target {
                    return Ok(Some(entry));
                }
            }
        }

        Ok(None)
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
        let entries = stmt
            .query_map(params![location_id.to_string()], row_to_entry)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }

    pub fn list_all(&self) -> Result<Vec<FileEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, save_location_id, path, hash, size_bytes, modified_at FROM files",
        )?;
        let entries = stmt
            .query_map([], row_to_entry)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    }

    pub fn delete(&self, id: Uuid) -> Result<()> {
        self.conn
            .execute("DELETE FROM files WHERE id = ?1", params![id.to_string()])?;
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
        modified_at: modified_str
            .parse::<DateTime<Utc>>()
            .unwrap_or_else(|_| Utc::now()),
    })
}
