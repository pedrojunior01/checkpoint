//! savevault-app
//!
//! Camada de aplicação e ponto de entrada do executável SaveVault.

pub mod orchestrator;
pub mod pipeline;

pub use orchestrator::{discover_and_persist_saves, DiscoverySyncReport};
pub use pipeline::{handle_stable_event, run_pipeline};

