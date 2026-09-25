//! savevault-app
//!
//! Camada de aplicação e ponto de entrada do executável SaveVault.

pub mod orchestrator;

pub use orchestrator::{discover_and_persist_saves, DiscoverySyncReport};
