use rusqlite::{Connection, params};
use anyhow::{Result, Context};
use savevault_core::models::{SaveLocation, SaveLocationType};
use uuid::Uuid;
use std::path::{Path, PathBuf};

pub struct SaveLocationsRepo<'a> {
    pub conn: &'a Connection,
}

impl<'a> SaveLocationsRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn insert(&self, loc: &SaveLocation) -> Result<()> {
        let norm_path = savevault_core::path::normalize_windows_path(&loc.path)
            .map_err(|e| anyhow::anyhow!("Caminho de save inválido: {}", e))?;
        let path_str = norm_path.to_str().context("Caminho de save contém caracteres não-UTF8")?;
        self.conn.execute(
            "INSERT INTO save_locations (id, game_id, path, location_type)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                loc.id.to_string(),
                loc.game_id.to_string(),
                path_str,
                location_type_to_str(&loc.location_type),
            ],
        )?;
        Ok(())
    }

    /// Insere ou atualiza um `SaveLocation` de forma estritamente idempotente.
    ///
    /// Se um registro para `(game_id, path COLLATE NOCASE)` já existir:
    /// - Atualiza campos não-identitários (`location_type`);
    /// - PRESERVA o ID (UUID) original do registro no banco (não gera novo UUID silenciosamente);
    /// - Retorna o `SaveLocation` persistido refletindo o estado final no banco.
    pub fn upsert(&self, loc: &SaveLocation) -> Result<SaveLocation> {
        let norm_path = savevault_core::path::normalize_windows_path(&loc.path)
            .map_err(|e| anyhow::anyhow!("Caminho de save inválido: {}", e))?;
        let path_str = norm_path.to_str().context("Caminho de save contém caracteres não-UTF8")?;

        let mut stmt = self.conn.prepare(
            "INSERT INTO save_locations (id, game_id, path, location_type)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(game_id, path COLLATE NOCASE) DO UPDATE SET
                 location_type = excluded.location_type
             RETURNING id, game_id, path, location_type"
        )?;

        let saved = stmt.query_row(
            params![
                loc.id.to_string(),
                loc.game_id.to_string(),
                path_str,
                location_type_to_str(&loc.location_type),
            ],
            row_to_location,
        )?;

        Ok(saved)
    }

    pub fn find_by_id(&self, id: Uuid) -> Result<Option<SaveLocation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, game_id, path, location_type FROM save_locations WHERE id = ?1"
        )?;
        let mut rows = stmt.query(params![id.to_string()])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_location(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn find_by_game_and_path(&self, game_id: Uuid, path: &Path) -> Result<Option<SaveLocation>> {
        let norm_path = savevault_core::path::normalize_windows_path(path)
            .map_err(|e| anyhow::anyhow!("Caminho de save inválido: {}", e))?;
        let path_str = norm_path.to_str().context("Caminho de save contém caracteres não-UTF8")?;
        let mut stmt = self.conn.prepare(
            "SELECT id, game_id, path, location_type FROM save_locations WHERE game_id = ?1 AND path = ?2 COLLATE NOCASE"
        )?;
        let mut rows = stmt.query(params![game_id.to_string(), path_str])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_location(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn list_by_game(&self, game_id: Uuid) -> Result<Vec<SaveLocation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, game_id, path, location_type FROM save_locations WHERE game_id = ?1"
        )?;
        let mut rows = stmt.query(params![game_id.to_string()])?;
        let mut locs = Vec::new();
        while let Some(row) = rows.next()? {
            locs.push(row_to_location(row)?);
        }
        Ok(locs)
    }

    pub fn list_all(&self) -> Result<Vec<SaveLocation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, game_id, path, location_type FROM save_locations"
        )?;
        let mut rows = stmt.query([])?;
        let mut locs = Vec::new();
        while let Some(row) = rows.next()? {
            locs.push(row_to_location(row)?);
        }
        Ok(locs)
    }

    pub fn delete(&self, id: Uuid) -> Result<()> {
        self.conn.execute("DELETE FROM save_locations WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }
}

fn row_to_location(row: &rusqlite::Row) -> rusqlite::Result<SaveLocation> {
    let id_str: String = row.get(0)?;
    let game_id_str: String = row.get(1)?;
    let path_str: String = row.get(2)?;
    let loc_type: String = row.get(3)?;

    let id = Uuid::parse_str(&id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let game_id = Uuid::parse_str(&game_id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
    })?;

    Ok(SaveLocation {
        id,
        game_id,
        path: PathBuf::from(path_str),
        location_type: str_to_location_type(&loc_type),
    })
}

fn location_type_to_str(t: &SaveLocationType) -> &'static str {
    match t {
        SaveLocationType::File => "file",
        SaveLocationType::Directory => "directory",
        SaveLocationType::Registry => "registry",
        SaveLocationType::Glob => "glob",
    }
}

fn str_to_location_type(s: &str) -> SaveLocationType {
    match s {
        "file" => SaveLocationType::File,
        "registry" => SaveLocationType::Registry,
        "glob" => SaveLocationType::Glob,
        _ => SaveLocationType::Directory,
    }
}
