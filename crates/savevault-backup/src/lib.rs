//! savevault-backup
//!
//! Hashing (BLAKE3) e gerenciamento de snapshots/versões de saves.

pub mod hasher;
pub mod snapshot;
pub mod storage;

pub use hasher::hash_file;
pub use snapshot::SnapshotManager;
pub use storage::{
    BackupSnapshotParams, BackupStorage, DriveFolderStatus, FetchSnapshotParams, FileAvailability,
    FilesystemBackupStorage, GameDrivePaths, StorageError, StoredGameIdentity,
    StoredSnapshotMetadata,
};
