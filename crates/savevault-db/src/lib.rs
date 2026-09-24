//! savevault-db
//!
//! Camada de persistência SQLite para o SaveVault.
//! Usa rusqlite com o feature "bundled" para incluir o SQLite estaticamente.

pub mod connection;
pub mod migrations;
pub mod repositories;

pub use connection::Database;

#[cfg(test)]
mod tests;
