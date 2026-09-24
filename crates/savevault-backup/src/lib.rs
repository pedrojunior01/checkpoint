//! savevault-backup
//!
//! Hashing (BLAKE3) e gerenciamento de snapshots/versões de saves.

pub mod hasher;
pub mod snapshot;

pub use hasher::hash_file;
pub use snapshot::SnapshotManager;
