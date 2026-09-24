use rusqlite::Connection;
use anyhow::Result;

/// Executa as migrações do banco de dados em ordem.
/// Novas migrações devem ser adicionadas no final do array `MIGRATIONS`.
pub fn run(conn: &Connection) -> Result<()> {
    conn.execute_batch(CREATE_MIGRATIONS_TABLE)?;

    let applied: Vec<String> = {
        let mut stmt = conn.prepare("SELECT name FROM __migrations ORDER BY applied_at")?;
        let result: Vec<String> = stmt.query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        result
    };

    for (name, sql) in MIGRATIONS {
        if !applied.contains(&name.to_string()) {
            conn.execute_batch(sql)?;
            conn.execute(
                "INSERT INTO __migrations (name, applied_at) VALUES (?1, datetime('now'))",
                [name],
            )?;
            tracing::info!("Migração aplicada: {}", name);
        }
    }

    Ok(())
}

const CREATE_MIGRATIONS_TABLE: &str = "
    CREATE TABLE IF NOT EXISTS __migrations (
        name       TEXT PRIMARY KEY,
        applied_at TEXT NOT NULL
    );
";

/// Lista de migrações: (nome_único, sql).
/// Adicionar sempre no final — nunca modificar migrações já aplicadas.
const MIGRATIONS: &[(&str, &str)] = &[
    ("001_initial_schema", MIGRATION_001),
];

const MIGRATION_001: &str = "
CREATE TABLE IF NOT EXISTS games (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    platform     TEXT NOT NULL,
    platform_id  TEXT,
    confidence   TEXT NOT NULL DEFAULT 'medium',
    created_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS save_locations (
    id              TEXT PRIMARY KEY,
    game_id         TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    path            TEXT NOT NULL,
    location_type   TEXT NOT NULL DEFAULT 'directory'
);

CREATE TABLE IF NOT EXISTS files (
    id                  TEXT PRIMARY KEY,
    save_location_id    TEXT NOT NULL REFERENCES save_locations(id) ON DELETE CASCADE,
    path                TEXT NOT NULL,
    hash                TEXT NOT NULL,
    size_bytes          INTEGER NOT NULL,
    modified_at         TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS backups (
    id          TEXT PRIMARY KEY,
    file_id     TEXT NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    version     INTEGER NOT NULL,
    cloud_path  TEXT,
    sync_status TEXT NOT NULL DEFAULT 'pending',
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_queue (
    id          TEXT PRIMARY KEY,
    backup_id   TEXT NOT NULL REFERENCES backups(id) ON DELETE CASCADE,
    operation   TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'pending',
    attempts    INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_save_locations_game ON save_locations(game_id);
CREATE INDEX IF NOT EXISTS idx_files_location ON files(save_location_id);
CREATE INDEX IF NOT EXISTS idx_backups_file ON backups(file_id);
CREATE INDEX IF NOT EXISTS idx_sync_queue_status ON sync_queue(status);
";
