use std::str::FromStr;

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use savevault_core::models::{GameMetadata, MetadataSource};
use uuid::Uuid;

pub struct GameMetadataRepo<'a> {
    pub conn: &'a Connection,
}

impl<'a> GameMetadataRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn upsert(&self, meta: &GameMetadata) -> Result<()> {
        self.conn.execute(
            "INSERT INTO game_metadata (game_id, steam_app_id, cover_url, source, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(game_id) DO UPDATE SET
                steam_app_id = excluded.steam_app_id,
                cover_url = excluded.cover_url,
                source = excluded.source,
                updated_at = excluded.updated_at",
            params![
                meta.game_id.to_string(),
                meta.steam_app_id,
                meta.cover_url,
                meta.source.to_string(),
                meta.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn find_by_game_id(&self, game_id: Uuid) -> Result<Option<GameMetadata>> {
        let mut stmt = self.conn.prepare(
            "SELECT game_id, steam_app_id, cover_url, source, updated_at
             FROM game_metadata WHERE game_id = ?1",
        )?;
        let mut rows = stmt.query_map(params![game_id.to_string()], row_to_game_metadata)?;
        Ok(rows.next().transpose()?)
    }

    pub fn find_by_steam_app_id(&self, app_id: &str) -> Result<Option<GameMetadata>> {
        let mut stmt = self.conn.prepare(
            "SELECT game_id, steam_app_id, cover_url, source, updated_at
             FROM game_metadata WHERE steam_app_id = ?1",
        )?;
        let mut rows = stmt.query_map(params![app_id], row_to_game_metadata)?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_all(&self) -> Result<Vec<GameMetadata>> {
        let mut stmt = self.conn.prepare(
            "SELECT game_id, steam_app_id, cover_url, source, updated_at
             FROM game_metadata ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_game_metadata)?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn delete(&self, game_id: Uuid) -> Result<()> {
        self.conn.execute(
            "DELETE FROM game_metadata WHERE game_id = ?1",
            params![game_id.to_string()],
        )?;
        Ok(())
    }
}

fn row_to_game_metadata(row: &rusqlite::Row) -> rusqlite::Result<GameMetadata> {
    let game_id_str: String = row.get(0)?;
    let steam_app_id: Option<String> = row.get(1)?;
    let cover_url: Option<String> = row.get(2)?;
    let source_str: String = row.get(3)?;
    let updated_at_str: String = row.get(4)?;

    let game_id = Uuid::parse_str(&game_id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let updated_at = updated_at_str.parse::<DateTime<Utc>>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let source = MetadataSource::from_str(&source_str).unwrap_or(MetadataSource::None);

    Ok(GameMetadata {
        game_id,
        steam_app_id,
        cover_url,
        source,
        updated_at,
    })
}
