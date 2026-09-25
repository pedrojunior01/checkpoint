use rusqlite::{Connection, params};
use anyhow::Result;
use savevault_core::models::{Game, Platform, Confidence};
use uuid::Uuid;
use chrono::{DateTime, Utc};

pub struct GamesRepo<'a> {
    pub conn: &'a Connection,
}

impl<'a> GamesRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn insert(&self, game: &Game) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO games (id, name, platform, platform_id, confidence, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                game.id.to_string(),
                game.name,
                platform_to_str(&game.platform),
                game.platform_id,
                confidence_to_str(&game.confidence),
                game.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn find_by_id(&self, id: Uuid) -> Result<Option<Game>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, platform, platform_id, confidence, created_at FROM games WHERE id = ?1"
        )?;
        let mut rows = stmt.query_map(params![id.to_string()], row_to_game)?;
        Ok(rows.next().transpose()?)
    }

    pub fn find_by_platform_id(&self, platform: &Platform, platform_id: &str) -> Result<Option<Game>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, platform, platform_id, confidence, created_at FROM games WHERE platform = ?1 AND platform_id = ?2"
        )?;
        let mut rows = stmt.query_map(params![platform_to_str(platform), platform_id], row_to_game)?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_all(&self) -> Result<Vec<Game>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, platform, platform_id, confidence, created_at FROM games ORDER BY name"
        )?;
        let games = stmt.query_map([], row_to_game)?.filter_map(|r| r.ok()).collect();
        Ok(games)
    }

    pub fn delete(&self, id: Uuid) -> Result<()> {
        self.conn.execute("DELETE FROM games WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }
}

fn row_to_game(row: &rusqlite::Row) -> rusqlite::Result<Game> {
    let id: String = row.get(0)?;
    let platform_str: String = row.get(2)?;
    let confidence_str: String = row.get(4)?;
    let created_at_str: String = row.get(5)?;

    Ok(Game {
        id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::new_v4()),
        name: row.get(1)?,
        platform: str_to_platform(&platform_str),
        platform_id: row.get(3)?,
        confidence: str_to_confidence(&confidence_str),
        created_at: created_at_str.parse::<DateTime<Utc>>().unwrap_or_else(|_| Utc::now()),
    })
}

fn platform_to_str(p: &Platform) -> &'static str {
    match p {
        Platform::Steam => "steam",
        Platform::Gog => "gog",
        Platform::Dolphin => "dolphin",
        Platform::Pcsx2 => "pcsx2",
        Platform::Ppsspp => "ppsspp",
        Platform::RetroArch => "retroarch",
        Platform::Generic => "generic",
    }
}

fn str_to_platform(s: &str) -> Platform {
    match s {
        "steam" => Platform::Steam,
        "gog" => Platform::Gog,
        "dolphin" => Platform::Dolphin,
        "pcsx2" => Platform::Pcsx2,
        "ppsspp" => Platform::Ppsspp,
        "retroarch" => Platform::RetroArch,
        _ => Platform::Generic,
    }
}

fn confidence_to_str(c: &Confidence) -> &'static str {
    match c {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

fn str_to_confidence(s: &str) -> Confidence {
    match s {
        "high" => Confidence::High,
        "low" => Confidence::Low,
        _ => Confidence::Medium,
    }
}
