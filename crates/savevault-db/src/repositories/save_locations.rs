use rusqlite::{Connection, params};
use anyhow::Result;
use savevault_core::models::{SaveLocation, SaveLocationType};
use uuid::Uuid;
use std::path::PathBuf;

pub struct SaveLocationsRepo<'a> {
    pub conn: &'a Connection,
}

impl<'a> SaveLocationsRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn insert(&self, loc: &SaveLocation) -> Result<()> {
        self.conn.execute(
            "INSERT INTO save_locations (id, game_id, path, location_type) VALUES (?1, ?2, ?3, ?4)",
            params![
                loc.id.to_string(),
                loc.game_id.to_string(),
                loc.path.to_string_lossy().to_string(),
                location_type_to_str(&loc.location_type),
            ],
        )?;
        Ok(())
    }

    pub fn list_by_game(&self, game_id: Uuid) -> Result<Vec<SaveLocation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, game_id, path, location_type FROM save_locations WHERE game_id = ?1"
        )?;
        let locs = stmt.query_map(params![game_id.to_string()], row_to_location)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(locs)
    }

    pub fn delete(&self, id: Uuid) -> Result<()> {
        self.conn.execute("DELETE FROM save_locations WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }
}

fn row_to_location(row: &rusqlite::Row) -> rusqlite::Result<SaveLocation> {
    let id: String = row.get(0)?;
    let game_id: String = row.get(1)?;
    let path: String = row.get(2)?;
    let loc_type: String = row.get(3)?;
    Ok(SaveLocation {
        id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::new_v4()),
        game_id: Uuid::parse_str(&game_id).unwrap_or_else(|_| Uuid::new_v4()),
        path: PathBuf::from(path),
        location_type: str_to_location_type(&loc_type),
    })
}

fn location_type_to_str(t: &SaveLocationType) -> &'static str {
    match t {
        SaveLocationType::File => "file",
        SaveLocationType::Directory => "directory",
        SaveLocationType::Registry => "registry",
    }
}

fn str_to_location_type(s: &str) -> SaveLocationType {
    match s {
        "file" => SaveLocationType::File,
        "registry" => SaveLocationType::Registry,
        _ => SaveLocationType::Directory,
    }
}
