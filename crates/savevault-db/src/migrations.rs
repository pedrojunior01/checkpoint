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
    ("002_save_locations_unique_path", MIGRATION_002),
    ("003_save_locations_nocase_index", MIGRATION_003),
    ("004_backups_drive_file_id", MIGRATION_004),
    ("005_games_unique_name_platform", MIGRATION_005),
    ("006_files_and_backups_unique", MIGRATION_006),
    ("007_games_platform_id_composite", MIGRATION_007),
    ("008_cleanup_savevault_phantom_backups", MIGRATION_008),
    ("009_save_location_remapping", MIGRATION_009),
    ("010_game_metadata", MIGRATION_010),
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
-- 1. Remove save_locations duplicados que causariam conflito ao fundir games duplicados
DELETE FROM save_locations
WHERE id IN (
    SELECT sl2.id
    FROM save_locations sl2
    JOIN games g2 ON sl2.game_id = g2.id
    JOIN games g1 ON LOWER(g1.name) = LOWER(g2.name) AND g1.platform = g2.platform AND g1.id != g2.id
    JOIN save_locations sl1 ON sl1.game_id = g1.id AND LOWER(sl1.path) = LOWER(sl2.path)
    WHERE g2.id NOT IN (SELECT MIN(id) FROM games GROUP BY LOWER(name), platform)
);

-- 2. Reassocia save_locations de games duplicados para o game mais antigo (canônico)
UPDATE save_locations
SET game_id = (
    SELECT MIN(g_target.id)
    FROM games g_target
    JOIN games g_cur ON g_cur.id = save_locations.game_id
    WHERE LOWER(g_target.name) = LOWER(g_cur.name) AND g_target.platform = g_cur.platform
)
WHERE game_id NOT IN (
    SELECT MIN(id) FROM games GROUP BY LOWER(name), platform
);

-- 3. Deleta games duplicados mantendo a entrada canônica MIN(id)
DELETE FROM games
WHERE id NOT IN (
    SELECT MIN(id) FROM games GROUP BY LOWER(name), platform
);

-- 4. Cria índice único por (name COLLATE NOCASE, platform)
CREATE UNIQUE INDEX IF NOT EXISTS idx_games_name_platform
    ON games(name COLLATE NOCASE, platform);
";

const MIGRATION_006: &str = "
-- 1. Remove arquivos duplicados mantendo o menor ID por (save_location_id, LOWER(path))
DELETE FROM files
WHERE id NOT IN (
    SELECT MIN(id) FROM files GROUP BY save_location_id, LOWER(path)
);

-- 2. Remove backups duplicados mantendo o menor ID por (file_id, version)
DELETE FROM backups
WHERE id NOT IN (
    SELECT MIN(id) FROM backups GROUP BY file_id, version
);

-- 3. Índice único para files (save_location_id, path case-insensitive)
CREATE UNIQUE INDEX IF NOT EXISTS idx_files_location_path
    ON files(save_location_id, path COLLATE NOCASE);

-- 4. Índice único para backups (file_id, version)
CREATE UNIQUE INDEX IF NOT EXISTS idx_backups_file_version
    ON backups(file_id, version);
";

const MIGRATION_007: &str = "
-- 1. Cria nova tabela games sem constraint UNIQUE global em platform_id
CREATE TABLE IF NOT EXISTS games_new (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    platform     TEXT NOT NULL,
    platform_id  TEXT,
    confidence   TEXT NOT NULL DEFAULT 'medium',
    created_at   TEXT NOT NULL
);

-- 2. Migra os dados existentes
INSERT OR IGNORE INTO games_new (id, name, platform, platform_id, confidence, created_at)
SELECT id, name, platform, platform_id, confidence, created_at FROM games;

-- 3. Desativa foreign keys para substituição da tabela
PRAGMA foreign_keys = OFF;
DROP TABLE games;
ALTER TABLE games_new RENAME TO games;
PRAGMA foreign_keys = ON;

-- 4. Recria o índice único por (name COLLATE NOCASE, platform)
CREATE UNIQUE INDEX IF NOT EXISTS idx_games_name_platform
    ON games(name COLLATE NOCASE, platform);

-- 5. Cria índice único composto para (platform, platform_id) permitindo IDs iguais em lojas distintas
CREATE UNIQUE INDEX IF NOT EXISTS idx_games_platform_platform_id
    ON games(platform, platform_id)
    WHERE platform_id IS NOT NULL;
";

const MIGRATION_008: &str = "
-- 1. Remove games identificados como pastas de versão ('v1', 'v2', etc.) ou cujo save_location apontava para pasta interna do SaveVault
DELETE FROM games
WHERE LOWER(name) IN ('v1', 'v2', 'v3', 'v4', 'v5', 'daily', 'latest')
   OR id IN (
       SELECT DISTINCT game_id FROM save_locations
       WHERE path LIKE '%SaveVault%backups%'
          OR path LIKE '%SaveVault/backups%'
          OR path LIKE '%SaveVault\\backups%'
          OR path LIKE '%AppData%SaveVault%'
   );

-- 2. Remove save_locations que apontam para o diretório de backups do SaveVault
DELETE FROM save_locations
WHERE path LIKE '%SaveVault%backups%'
   OR path LIKE '%SaveVault/backups%'
   OR path LIKE '%SaveVault\\backups%'
   OR path LIKE '%AppData%SaveVault%';

-- 3. Limpa arquivos e backups órfãos se houver
DELETE FROM files WHERE save_location_id NOT IN (SELECT id FROM save_locations);
DELETE FROM backups WHERE file_id NOT IN (SELECT id FROM files);
DELETE FROM sync_queue WHERE backup_id NOT IN (SELECT id FROM backups);
";

const MIGRATION_009: &str = "
-- Adiciona suporte a remapeamento multiplataforma de save_locations (AUD-03)
ALTER TABLE save_locations ADD COLUMN original_path TEXT;
ALTER TABLE save_locations ADD COLUMN mapping_status TEXT NOT NULL DEFAULT 'resolved';
ALTER TABLE save_locations ADD COLUMN mapping_source TEXT NOT NULL DEFAULT 'original';
ALTER TABLE save_locations ADD COLUMN resolution_reason TEXT;

CREATE INDEX IF NOT EXISTS idx_save_locations_mapping_status ON save_locations(mapping_status);
";

const MIGRATION_010: &str = "
-- Metadados visuais, identificadores externos e cache de capas de jogos
CREATE TABLE IF NOT EXISTS game_metadata (
    game_id         TEXT PRIMARY KEY REFERENCES games(id) ON DELETE CASCADE,
    steam_app_id    TEXT,
    cover_url       TEXT,
    source          TEXT NOT NULL DEFAULT 'none',
    updated_at      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_game_metadata_steam_app_id ON game_metadata(steam_app_id);
";
