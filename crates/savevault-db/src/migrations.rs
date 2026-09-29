use anyhow::Result;
use rusqlite::Connection;

/// Executa as migrações do banco de dados em ordem.
/// Novas migrações devem ser adicionadas no final do array `MIGRATIONS`.
pub fn run(conn: &Connection) -> Result<()> {
    conn.execute_batch(CREATE_MIGRATIONS_TABLE)?;

    let applied: Vec<String> = {
        let mut stmt = conn.prepare("SELECT name FROM __migrations ORDER BY applied_at")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        let mut result = Vec::new();
        for r in rows {
            result.push(r?);
        }
        result
    };

    for (name, sql) in MIGRATIONS {
        if !applied.contains(&name.to_string()) {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.execute(
                "INSERT INTO __migrations (name, applied_at) VALUES (?1, datetime('now'))",
                [name],
            )?;
            tx.commit()?;
            tracing::info!("Migração aplicada com atomicidade: {}", name);
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
    ("002_save_locations_unique_path", MIGRATION_002),
    ("003_save_locations_nocase_index", MIGRATION_003),
    ("004_backups_drive_file_id", MIGRATION_004),
    ("005_files_nocase_index", MIGRATION_005),
];

const MIGRATION_001: &str = "
CREATE TABLE IF NOT EXISTS games (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    platform     TEXT NOT NULL,
    platform_id  TEXT UNIQUE,
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

const MIGRATION_002: &str = "
-- Remove duplicatas de save_locations mantendo a entrada mais antiga por (game_id, path)
DELETE FROM save_locations
WHERE id NOT IN (
    SELECT MIN(id) FROM save_locations GROUP BY game_id, path
);

-- Cria índice único para garantir idempotência em inserções futuras
CREATE UNIQUE INDEX IF NOT EXISTS idx_save_locations_game_path
    ON save_locations(game_id, path);
";

const MIGRATION_003: &str = "
DROP INDEX IF EXISTS idx_save_locations_game_path;
DROP INDEX IF EXISTS idx_save_locations_unique_path;
CREATE UNIQUE INDEX IF NOT EXISTS idx_save_locations_game_path
    ON save_locations(game_id, path COLLATE NOCASE);
";

const MIGRATION_004: &str = "
ALTER TABLE backups ADD COLUMN drive_file_id TEXT;
CREATE INDEX IF NOT EXISTS idx_backups_drive_file_id ON backups(drive_file_id);
";

const MIGRATION_005: &str = "
-- 1. Consolida backups que apontam para arquivos duplicados na tabela files antes de criar o índice único
UPDATE backups
SET file_id = (
    SELECT f_canonical.id
    FROM files f_canonical
    JOIN files f_dup ON f_dup.id = backups.file_id
    WHERE f_canonical.path = f_dup.path COLLATE NOCASE
    ORDER BY f_canonical.rowid ASC
    LIMIT 1
)
WHERE file_id IN (
    SELECT id FROM files f_inner
    WHERE f_inner.rowid > (
        SELECT MIN(f_min.rowid) FROM files f_min
        WHERE f_min.path = f_inner.path COLLATE NOCASE
    )
);

-- 2. Remove registros duplicados na tabela files preservando a primeira ocorrência
DELETE FROM files
WHERE rowid NOT IN (
    SELECT MIN(rowid)
    FROM files
    GROUP BY path COLLATE NOCASE
);

-- 3. Cria índice único case-insensitive na tabela files para garantir unicidade física no Windows
CREATE UNIQUE INDEX IF NOT EXISTS idx_files_path_nocase ON files(path COLLATE NOCASE);
";
