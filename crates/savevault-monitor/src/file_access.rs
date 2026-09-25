//! Abstração de acesso ao sistema de arquivos.
//!
//! Permite desacoplar a verificação de estabilidade e o tratamento de `ERROR_SHARING_VIOLATION`
//! de chamadas físicas diretas, viabilizando testes unitários rápidos, determinísticos e sem race conditions.

use std::path::Path;
use std::time::SystemTime;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMetadata {
    pub size_bytes: u64,
    pub modified_at: SystemTime,
}

#[derive(Debug, Error)]
pub enum FileAccessError {
    #[error("Arquivo não encontrado")]
    NotFound,

    #[error("Violação de compartilhamento (arquivo bloqueado por outro processo)")]
    SharingViolation,

    #[error("Acesso negado")]
    PermissionDenied,

    #[error("Erro de I/O: {0}")]
    Other(std::io::Error),
}

/// Trait para operações de leitura e inspeção de arquivos.
pub trait FileAccess: Send + Sync {
    /// Obtém metadados do arquivo (tamanho e timestamp de modificação).
    fn metadata(&self, path: &Path) -> Result<FileMetadata, FileAccessError>;

    /// Tenta abrir o arquivo para leitura compartilhada a fim de confirmar
    /// se não está retido exclusivamente por uma aplicação de jogo (sharing violation).
    fn verify_readable(&self, path: &Path) -> Result<(), FileAccessError>;
}

/// Implementação padrão que consulta o sistema operacional real.
pub struct OsFileAccess;

impl FileAccess for OsFileAccess {
    fn metadata(&self, path: &Path) -> Result<FileMetadata, FileAccessError> {
        match std::fs::metadata(path) {
            Ok(meta) => {
                let size_bytes = meta.len();
                let modified_at = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                Ok(FileMetadata {
                    size_bytes,
                    modified_at,
                })
            }
            Err(e) => Err(map_io_error(e)),
        }
    }

    fn verify_readable(&self, path: &Path) -> Result<(), FileAccessError> {
        match std::fs::File::open(path) {
            Ok(_) => Ok(()),
            Err(e) => Err(map_io_error(e)),
        }
    }
}

/// Mapeia erros de I/O identificando com precisão códigos Win32 característicos de lock (`ERROR_SHARING_VIOLATION`)
/// versus negações permanentes de permissão / ACL (`ERROR_ACCESS_DENIED`).
fn map_io_error(e: std::io::Error) -> FileAccessError {
    if e.kind() == std::io::ErrorKind::NotFound {
        return FileAccessError::NotFound;
    }

    #[cfg(windows)]
    if let Some(code) = e.raw_os_error() {
        // ERROR_SHARING_VIOLATION = 32 (0x20)
        // ERROR_LOCK_VIOLATION = 33 (0x21)
        if code == 32 || code == 33 {
            return FileAccessError::SharingViolation;
        }

        // ERROR_ACCESS_DENIED = 5 (0x05)
        if code == 5 {
            return FileAccessError::PermissionDenied;
        }
    }

    if e.kind() == std::io::ErrorKind::PermissionDenied {
        return FileAccessError::PermissionDenied;
    }

    FileAccessError::Other(e)
}

