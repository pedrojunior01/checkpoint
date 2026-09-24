use rusqlite::Connection;
use std::path::Path;
use anyhow::Result;

use crate::migrations;

/// Handle principal do banco de dados SQLite.
pub struct Database {
    pub conn: Connection,
}

impl Database {
    /// Abre (ou cria) o banco no caminho especificado e executa as migrações.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(path)?;

        // Ativa WAL mode para melhor performance em leituras concorrentes.
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

        let db = Database { conn };
        migrations::run(&db.conn)?;

        Ok(db)
    }

    /// Abre um banco em memória (útil para testes).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let db = Database { conn };
        migrations::run(&db.conn)?;
        Ok(db)
    }
}
