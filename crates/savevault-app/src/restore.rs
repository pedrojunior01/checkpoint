//! Motor de Restauração de Snapshots e Safety Backups do SaveVault (RES-01).
//!
//! Permite restaurar um arquivo salvo anteriormente a partir de um snapshot versionado,
//! com garantias estritas de segurança:
//! 1. Verificação de existência no banco SQLite e integridade referencial;
//! 2. Proteção contra path traversal e validação contra diretórios de sistema autorizados;
//! 3. Validação de integridade via hash BLAKE3 antes de modificar o destino;
//! 4. Criação obrigatória de Safety Backup do arquivo atual com identificadores únicos
//!    resistentes a colisões;
//! 5. Escrita atômica via arquivo temporário de staging no mesmo filesystem/volume;
//! 6. Validação de integridade pós-escrita;
//! 7. Atualização coerente dos metadados de arquivo no SQLite.

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

use savevault_core::models::{FileEntry, Game, SaveLocation, SaveLocationType};
use savevault_db::repositories::{BackupsRepo, FilesRepo, SaveLocationsRepo};

/// Erros específicos da operação de restauração de snapshots.
#[derive(Debug, Error)]
pub enum RestoreError {
    #[error("Snapshot '{0}' não encontrado no banco de dados")]
    SnapshotNotFound(Uuid),

    #[error("Registro de arquivo (ID '{0}') associado ao snapshot não foi encontrado")]
    FileNotFound(Uuid),

    #[error("Local de save (ID '{0}') associado ao arquivo não foi encontrado")]
    SaveLocationNotFound(Uuid),

    #[error("Arquivo físico do snapshot não encontrado em '{0}'")]
    PhysicalSnapshotMissing(PathBuf),

    #[error("Caminho do snapshot inválido ou é um diretório: '{0}'")]
    InvalidSnapshotSource(PathBuf),

    #[error("Falha de segurança contra path traversal: {0}")]
    PathSecurityViolation(String),

    #[error("Integridade violada para snapshot '{0}': hash BLAKE3 calculado ('{2}') difere do esperado ('{1}')")]
    IntegrityCheckFailed(Uuid, String, String),

    #[error("Falha na validação de integridade pós-escrita em '{0}': hash pós-escrita difere do snapshot")]
    PostWriteValidationFailed(PathBuf),

    #[error("Falha na validação de integridade pós-escrita em '{path}'. Rollback realizado com sucesso a partir de '{safety_backup}'")]
    PostWriteValidationFailedWithRollback {
        path: PathBuf,
        safety_backup: PathBuf,
    },

    #[error("CRÍTICO: Falha na validação de integridade pós-escrita em '{path}' E falha ao aplicar rollback a partir de '{safety_backup}': {rollback_error}")]
    PostWriteValidationAndRollbackFailed {
        path: PathBuf,
        safety_backup: PathBuf,
        rollback_error: String,
    },

    #[error("Falha ao criar safety backup em '{0}': {1}")]
    SafetyBackupFailed(PathBuf, String),

    #[error("Falha na escrita atômica do arquivo em '{0}': {1}")]
    AtomicWriteFailed(PathBuf, String),

    #[error(
        "Falha em transação multi-arquivo de restore. Rollback global aplicado com sucesso: {0}"
    )]
    MultiFileTransactionFailedWithRollback(String),

    #[error("CRÍTICO: Falha em transação multi-arquivo de restore E falha catastrófica durante rollback: {0:?}")]
    MultiFileTransactionAndRollbackFailed(Box<MultiFileRollbackFailureReport>),

    #[error("Erro de banco de dados SQLite: {0}")]
    DatabaseError(#[from] rusqlite::Error),

    #[error("Erro de I/O no filesystem: {0}")]
    IoError(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

/// Detalhes de falha catastrófica onde um rollback multi-arquivo falhou parcialmente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiFileRollbackFailureReport {
    pub failed_file: PathBuf,
    pub primary_error: String,
    pub rolled_back_files: Vec<PathBuf>,
    pub failed_rollback_files: Vec<(PathBuf, String)>,
    pub preserved_safety_backups: Vec<PathBuf>,
}

/// Parâmetros para solicitação de restauração.
#[derive(Debug, Clone)]
pub struct RestoreRequest {
    /// Identificador único (UUID) do snapshot/backup a ser restaurado.
    pub snapshot_id: Uuid,
    /// Hash BLAKE3 esperado opcional para validação antecipada de integridade.
    pub expected_hash: Option<String>,
}

impl RestoreRequest {
    pub fn new(snapshot_id: Uuid) -> Self {
        Self {
            snapshot_id,
            expected_hash: None,
        }
    }

    pub fn with_expected_hash(snapshot_id: Uuid, expected_hash: impl Into<String>) -> Self {
        Self {
            snapshot_id,
            expected_hash: Some(expected_hash.into()),
        }
    }
}

/// Relatório com os detalhes e evidências da restauração realizada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    /// UUID do snapshot restaurado.
    pub snapshot_id: Uuid,
    /// UUID do arquivo correspondente na tabela `files`.
    pub file_id: Uuid,
    /// Número da versão restaurada.
    pub version: u32,
    /// Caminho do arquivo físico do snapshot de origem.
    pub source_path: PathBuf,
    /// Caminho do arquivo de save de destino restaurado.
    pub dest_path: PathBuf,
    /// Caminho do safety backup criado antes da sobrescrita (se o arquivo anterior existia).
    pub safety_backup_path: Option<PathBuf>,
    /// Hash BLAKE3 do arquivo restaurado no destino.
    pub restored_hash: String,
    /// Tamanho em bytes do arquivo restaurado.
    pub bytes_restored: u64,
}

/// Valida a segurança do caminho de destino contra path traversal e escape do `SaveLocation`.
pub fn validate_destination_security(
    dest_path: &Path,
    save_location: &SaveLocation,
) -> Result<(), RestoreError> {
    if !dest_path.is_absolute() {
        return Err(RestoreError::PathSecurityViolation(format!(
            "Caminho de destino deve ser absoluto: '{}'",
            dest_path.display()
        )));
    }

    // Proibir expressamente componentes de travessia '..'
    for comp in dest_path.components() {
        if matches!(comp, Component::ParentDir) {
            return Err(RestoreError::PathSecurityViolation(format!(
                "Caminho de destino contém componentes de travessia '..': '{}'",
                dest_path.display()
            )));
        }
    }

    // Normalizar caminhos para análise consistente no Windows
    let norm_dest = savevault_core::path::normalize_windows_path(dest_path)
        .map_err(|e| RestoreError::PathSecurityViolation(format!("Caminho inválido: {}", e)))?;
    let norm_loc = savevault_core::path::normalize_windows_path(&save_location.path)
        .map_err(|e| RestoreError::PathSecurityViolation(format!("Caminho inválido: {}", e)))?;

    let dest_str = norm_dest.to_string_lossy();
    let lower_dest = dest_str.to_lowercase();

    // Bloquear raízes de unidade de disco (ex.: C:\)
    if lower_dest.len() <= 3 && lower_dest.ends_with(r":\") {
        return Err(RestoreError::PathSecurityViolation(format!(
            "Destino não pode ser a raiz da unidade: '{}'",
            dest_path.display()
        )));
    }

    // Bloquear pastas de sistema protegidas do Windows a menos que o SaveLocation esteja expressamente lá
    if lower_dest.starts_with(r"c:\windows")
        || lower_dest.starts_with(r"c:\program files")
        || lower_dest.starts_with(r"c:\program files (x86)")
    {
        let loc_str = norm_loc.to_string_lossy().to_lowercase();
        if !loc_str.starts_with(r"c:\windows")
            && !loc_str.starts_with(r"c:\program files")
            && !loc_str.starts_with(r"c:\program files (x86)")
        {
            return Err(RestoreError::PathSecurityViolation(format!(
                "Destino aponta para diretório de sistema protegido: '{}'",
                dest_path.display()
            )));
        }
    }

    // Validar enquadramento no SaveLocation autorizado
    match save_location.location_type {
        SaveLocationType::File => {
            let canon_dest = savevault_core::path::canonicalize_windows_path_str(&dest_str)
                .map_err(|e| RestoreError::PathSecurityViolation(e.to_string()))?;
            let canon_loc =
                savevault_core::path::canonicalize_windows_path_str(&norm_loc.to_string_lossy())
                    .map_err(|e| RestoreError::PathSecurityViolation(e.to_string()))?;

            if canon_dest != canon_loc {
                return Err(RestoreError::PathSecurityViolation(format!(
                    "Caminho do arquivo '{}' diverge do SaveLocation do tipo File '{}'",
                    dest_path.display(),
                    save_location.path.display()
                )));
            }
        }
        SaveLocationType::Directory => {
            let canon_dest = savevault_core::path::canonicalize_windows_path_str(&dest_str)
                .map_err(|e| RestoreError::PathSecurityViolation(e.to_string()))?;
            let canon_loc =
                savevault_core::path::canonicalize_windows_path_str(&norm_loc.to_string_lossy())
                    .map_err(|e| RestoreError::PathSecurityViolation(e.to_string()))?;

            let prefix = if canon_loc.ends_with('\\') {
                canon_loc.clone()
            } else {
                format!("{}\\", canon_loc)
            };

            if !canon_dest.starts_with(&prefix) && canon_dest != canon_loc {
                return Err(RestoreError::PathSecurityViolation(format!(
                    "Caminho de destino '{}' escapa do SaveLocation autorizado '{}'",
                    dest_path.display(),
                    save_location.path.display()
                )));
            }
        }
        SaveLocationType::Glob => {
            let loc_str = norm_loc.to_string_lossy();
            let base_part = if let Some(idx) = loc_str.find(['*', '?']) {
                let prefix = &loc_str[..idx];
                if let Some(last_slash) = prefix.rfind('\\') {
                    &prefix[..last_slash]
                } else {
                    prefix
                }
            } else {
                &loc_str
            };

            if !base_part.is_empty() {
                let canon_dest = savevault_core::path::canonicalize_windows_path_str(&dest_str)
                    .map_err(|e| RestoreError::PathSecurityViolation(e.to_string()))?;
                let canon_base = savevault_core::path::canonicalize_windows_path_str(base_part)
                    .map_err(|e| RestoreError::PathSecurityViolation(e.to_string()))?;
                let prefix = if canon_base.ends_with('\\') {
                    canon_base.clone()
                } else {
                    format!("{}\\", canon_base)
                };

                if !canon_dest.starts_with(&prefix) && canon_dest != canon_base {
                    return Err(RestoreError::PathSecurityViolation(format!(
                        "Caminho de destino '{}' escapa da base autorizada do padrão glob '{}'",
                        dest_path.display(),
                        base_part
                    )));
                }
            }
        }
        SaveLocationType::Registry => {
            return Err(RestoreError::PathSecurityViolation(
                "Restauração de SaveLocation do tipo Registry não é suportada como arquivo".into(),
            ));
        }
    }

    // 7. Validação física contra escape via Reparse Points, Junções NTFS e Symlinks
    verify_no_reparse_point_escape(dest_path, &save_location.path)?;

    Ok(())
}

/// Verifica se qualquer componente existente no caminho de destino (ou ancestrais até a raiz autorizada)
/// é um ponto de reanálise (Reparse Point), junção NTFS ou link simbólico que desvie para fora da raiz autorizada.
/// Inspeciona se um caminho individual no disco é um Reparse Point/Junction que escapa da raiz autorizada.
fn check_single_reparse_point(
    path: &Path,
    norm_root: &str,
    root_prefix: &str,
) -> Result<(), RestoreError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            let mut is_reparse = meta.file_type().is_symlink();
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if (meta.file_attributes() & 0x400) != 0 {
                    is_reparse = true;
                }
            }

            if is_reparse {
                match std::fs::canonicalize(path) {
                    Ok(target_canon) => {
                        let norm_target = target_canon.to_string_lossy().to_lowercase();
                        if !norm_target.starts_with(root_prefix) && norm_target != norm_root {
                            return Err(RestoreError::PathSecurityViolation(format!(
                                "Tentativa de escape via Reparse Point/Junction detectada: '{}' aponta para fora da raiz autorizada ('{}')",
                                path.display(),
                                target_canon.display()
                            )));
                        }
                    }
                    Err(_) => {
                        return Err(RestoreError::PathSecurityViolation(format!(
                            "Reparse point inválido ou inacessível no destino: '{}'",
                            path.display()
                        )));
                    }
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(RestoreError::PathSecurityViolation(format!(
                "Falha ao inspecionar metadados de '{}': {}",
                path.display(),
                e
            )));
        }
    }
    Ok(())
}

/// Verifica se qualquer componente existente no caminho de destino (ou ancestrais até a raiz autorizada)
/// é um ponto de reanálise (Reparse Point), junção NTFS ou link simbólico que desvie para fora da raiz autorizada.
pub fn verify_no_reparse_point_escape(
    dest_path: &Path,
    allowed_root: &Path,
) -> Result<(), RestoreError> {
    // 1. Determinar o caminho canônico da raiz autorizada (ou projetado, caso ainda não exista no disco)
    let (canon_allowed_root, existing_root_depth) = if allowed_root.exists() {
        let canon = match std::fs::canonicalize(allowed_root) {
            Ok(p) => p,
            Err(e) => {
                return Err(RestoreError::PathSecurityViolation(format!(
                    "Falha ao canonicalizar raiz autorizada '{}': {}",
                    allowed_root.display(),
                    e
                )));
            }
        };
        (canon, allowed_root.components().count())
    } else {
        // Se a raiz autorizada não existe ainda, encontrar seu ancestral mais profundo existente
        let mut existing_ancestor = allowed_root.to_path_buf();
        let mut missing_suffix = Vec::new();
        while !existing_ancestor.exists() {
            if let Some(filename) = existing_ancestor.file_name() {
                missing_suffix.push(filename.to_os_string());
            }
            if let Some(parent) = existing_ancestor.parent() {
                existing_ancestor = parent.to_path_buf();
            } else {
                break;
            }
        }

        if existing_ancestor.exists() {
            // Se qualquer ancestral existente da raiz autorizada for um Reparse Point / Junction,
            // isso desvia o caminho antes mesmo da raiz existir!
            let mut check_ancestor = existing_ancestor.clone();
            while let Some(parent) = check_ancestor.parent() {
                if let Ok(meta) = std::fs::symlink_metadata(&check_ancestor) {
                    let mut is_reparse = meta.file_type().is_symlink();
                    #[cfg(windows)]
                    {
                        use std::os::windows::fs::MetadataExt;
                        if (meta.file_attributes() & 0x400) != 0 {
                            is_reparse = true;
                        }
                    }
                    if is_reparse {
                        return Err(RestoreError::PathSecurityViolation(format!(
                            "Ancestral da raiz autorizada inexistente '{}' é um Reparse Point que escapa do caminho esperado",
                            check_ancestor.display()
                        )));
                    }
                }
                check_ancestor = parent.to_path_buf();
            }

            let canon_ancestor = match std::fs::canonicalize(&existing_ancestor) {
                Ok(p) => p,
                Err(e) => {
                    return Err(RestoreError::PathSecurityViolation(format!(
                        "Falha ao canonicalizar ancestral da raiz autorizada '{}': {}",
                        existing_ancestor.display(),
                        e
                    )));
                }
            };

            missing_suffix.reverse();
            let mut projected = canon_ancestor;
            for part in missing_suffix {
                projected.push(part);
            }
            (projected, allowed_root.components().count())
        } else {
            return Ok(());
        }
    };

    let norm_root = canon_allowed_root.to_string_lossy().to_lowercase();
    let root_prefix = if norm_root.ends_with('\\') {
        norm_root.clone()
    } else {
        format!("{}\\", norm_root)
    };

    // 2. Inspecionar componentes existentes a partir do destino subindo até a raiz autorizada
    let mut current = dest_path.to_path_buf();
    while let Some(parent) = current.parent() {
        // Parar se já alcançamos ou ultrapassamos a raiz autorizada (comparação segura por profundidade e casing)
        if current.components().count() <= existing_root_depth
            || current
                .to_string_lossy()
                .eq_ignore_ascii_case(&allowed_root.to_string_lossy())
        {
            if current.exists() {
                check_single_reparse_point(&current, &norm_root, &root_prefix)?;
            }
            break;
        }

        if current.exists() {
            check_single_reparse_point(&current, &norm_root, &root_prefix)?;

            // Também verificar a canonicalização física do componente existente
            if let Ok(canon_existing) = std::fs::canonicalize(&current) {
                let norm_existing = canon_existing.to_string_lossy().to_lowercase();
                if !norm_existing.starts_with(&root_prefix) && norm_existing != norm_root {
                    return Err(RestoreError::PathSecurityViolation(format!(
                        "Caminho físico de '{}' ('{}') escapa da raiz autorizada ('{}')",
                        current.display(),
                        canon_existing.display(),
                        canon_allowed_root.display()
                    )));
                }
            }
        }

        current = parent.to_path_buf();
    }

    Ok(())
}

/// Valida que o caminho do snapshot físico está estritamente contido no diretório raiz de backups.
pub fn validate_snapshot_security(
    backup_root: &Path,
    file_id: Uuid,
    version: u32,
    filename: &str,
) -> Result<PathBuf, RestoreError> {
    if filename.is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains("..")
        || filename.contains('\0')
    {
        return Err(RestoreError::PathSecurityViolation(format!(
            "Nome de arquivo de snapshot inseguro: '{}'",
            filename
        )));
    }

    let version_dir = backup_root
        .join(file_id.to_string())
        .join(format!("v{}", version));
    let source_path = version_dir.join(filename);

    if !source_path.starts_with(backup_root) {
        return Err(RestoreError::PathSecurityViolation(format!(
            "Caminho do snapshot escapa do backup_root: '{}'",
            source_path.display()
        )));
    }

    Ok(source_path)
}

/// Restaura um snapshot para o caminho de destino correspondente de forma atômica e segura.
///
/// Fluxo completo:
/// 1. Localiza metadados do snapshot na tabela `backups`;
/// 2. Localiza registro correspondente na tabela `files`;
/// 3. Localiza `save_locations` e valida segurança contra path traversal;
/// 4. Resolve e valida a existência do arquivo físico do snapshot;
/// 5. Valida integridade do snapshot via hash BLAKE3 (comparando com hash esperado se fornecido);
/// 6. Cria Safety Backup do arquivo existente no destino com identificador anti-colisão;
/// 7. Escreve os dados atomicamente através de um arquivo temporário de staging;
/// 8. Valida o hash pós-escrita no destino;
/// 9. Atualiza o registro em `files` para refletir o hash e estado restaurados.
///
/// Metadados necessários para autorização e execução da restauração.
#[derive(Debug, Clone)]
pub struct RestoreMetadata {
    pub backup: savevault_core::models::Backup,
    pub file_entry: FileEntry,
    pub save_location: SaveLocation,
}

/// Carrega e valida os metadados do snapshot a partir da conexão SQLite.
pub fn load_restore_metadata(
    conn: &Connection,
    snapshot_id: Uuid,
) -> Result<RestoreMetadata, RestoreError> {
    // 1. Localiza metadados do snapshot
    let backups_repo = BackupsRepo::new(conn);
    let backup = backups_repo
        .find_by_id(snapshot_id)
        .map_err(|e| RestoreError::Other(e.to_string()))?
        .ok_or(RestoreError::SnapshotNotFound(snapshot_id))?;

    // 2. Localiza metadados do arquivo original
    let files_repo = FilesRepo::new(conn);
    let file_entry = files_repo
        .find_by_id(backup.file_id)
        .map_err(|e| RestoreError::Other(e.to_string()))?
        .ok_or(RestoreError::FileNotFound(backup.file_id))?;

    // 3. Localiza o SaveLocation autorizado
    let locations_repo = SaveLocationsRepo::new(conn);
    let save_location = locations_repo
        .find_by_id(file_entry.save_location_id)
        .map_err(|e| RestoreError::Other(e.to_string()))?
        .ok_or(RestoreError::SaveLocationNotFound(
            file_entry.save_location_id,
        ))?;

    Ok(RestoreMetadata {
        backup,
        file_entry,
        save_location,
    })
}

/// Atualiza o registro do arquivo após uma restauração bem-sucedida.
pub fn update_restored_file_entry(
    conn: &Connection,
    file_entry: &FileEntry,
    restored_hash: &str,
    bytes_restored: u64,
) -> Result<(), RestoreError> {
    let updated_file_entry = FileEntry {
        id: file_entry.id,
        save_location_id: file_entry.save_location_id,
        path: file_entry.path.clone(),
        hash: restored_hash.to_string(),
        size_bytes: bytes_restored,
        modified_at: Utc::now(),
    };
    let files_repo = FilesRepo::new(conn);
    let _ = files_repo.upsert(&updated_file_entry);
    Ok(())
}

/// Executa os passos de filesystem para restauração de um snapshot.
pub async fn execute_restore_filesystem(
    backup_root: &Path,
    metadata: &RestoreMetadata,
    request: &RestoreRequest,
) -> Result<RestoreReport, RestoreError> {
    let dest_path = &metadata.file_entry.path;

    // Registra RAII Guard de restauração ativa para suprimir eventos do FileWatcher
    // durante a criação de safety backup, staging e substituição do arquivo (SV-ADV-010).
    let _guard = ActiveRestoreGuard::new(std::slice::from_ref(dest_path));

    // 4. Validação de segurança contra path traversal
    validate_destination_security(dest_path, &metadata.save_location)?;

    let filename = dest_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| {
            RestoreError::PathSecurityViolation(format!(
                "Caminho de destino não possui nome de arquivo válido: '{}'",
                dest_path.display()
            ))
        })?;

    let source_path = validate_snapshot_security(
        backup_root,
        metadata.backup.file_id,
        metadata.backup.version,
        filename,
    )?;

    // 5. Validação da existência do snapshot físico no disco
    if !source_path.exists() {
        return Err(RestoreError::PhysicalSnapshotMissing(source_path));
    }

    if source_path.is_dir() {
        return Err(RestoreError::InvalidSnapshotSource(source_path));
    }

    // 6. Validação de integridade do snapshot antes da restauração
    let source_hash = savevault_backup::hasher::hash_file(&source_path)
        .await
        .map_err(|e| RestoreError::Other(format!("Falha ao calcular hash do snapshot: {}", e)))?;

    if let Some(ref expected) = request.expected_hash {
        if &source_hash != expected {
            return Err(RestoreError::IntegrityCheckFailed(
                request.snapshot_id,
                expected.clone(),
                source_hash,
            ));
        }
    }

    let dest_parent = dest_path.parent().unwrap_or_else(|| Path::new("."));
    if !dest_parent.exists() {
        tokio::fs::create_dir_all(dest_parent).await?;
    }

    // 7. Safety Backup do estado atual se o arquivo de destino já existir
    let safety_backup_path = if dest_path.exists() {
        let orig_name = filename;
        let mut candidate;
        let mut counter = 0u32;
        loop {
            let unique_suffix = format!("{}_{:04x}", Uuid::new_v4().simple(), counter);
            let safety_name = format!("{}.savevault_safety_{}", orig_name, unique_suffix);
            candidate = dest_parent.join(safety_name);
            if !candidate.exists() {
                break;
            }
            counter += 1;
        }

        tokio::fs::copy(dest_path, &candidate).await.map_err(|e| {
            RestoreError::SafetyBackupFailed(
                candidate.clone(),
                format!(
                    "Não foi possível copiar arquivo atual para safety backup: {}",
                    e
                ),
            )
        })?;
        tracing::info!(
            "Safety backup criado com sucesso: {} -> {}",
            dest_path.display(),
            candidate.display()
        );
        Some(candidate)
    } else {
        None
    };

    // 8. Escrita atômica via arquivo temporário de staging no mesmo diretório
    let orig_name = filename;
    let tmp_path = dest_parent.join(format!(
        "{}.restore_tmp_{}",
        orig_name,
        Uuid::new_v4().simple()
    ));

    if let Err(e) = tokio::fs::copy(&source_path, &tmp_path).await {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(RestoreError::AtomicWriteFailed(
            tmp_path,
            format!("Falha ao copiar snapshot para staging: {}", e),
        ));
    }

    let tmp_hash = match savevault_backup::hasher::hash_file(&tmp_path).await {
        Ok(h) => h,
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(RestoreError::AtomicWriteFailed(
                tmp_path,
                format!("Falha ao calcular hash do arquivo temporário: {}", e),
            ));
        }
    };

    if tmp_hash != source_hash {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(RestoreError::IntegrityCheckFailed(
            request.snapshot_id,
            source_hash,
            tmp_hash,
        ));
    }

    // Renomeia o temporário sobre o destino (MoveFileExW com REPLACE_EXISTING no Windows)
    if let Err(e) = tokio::fs::rename(&tmp_path, dest_path).await {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(RestoreError::AtomicWriteFailed(
            dest_path.clone(),
            format!("Falha ao aplicar restauração atômica (rename): {}", e),
        ));
    }

    // 9. Validação final pós-escrita no destino
    let dest_hash_res = savevault_backup::hasher::hash_file(dest_path).await;
    let mut verification_failed = false;

    match dest_hash_res {
        Ok(ref h) => {
            if h != &source_hash {
                tracing::error!(
                    "Integridade violada pós-escrita em '{}': hash '{}' difere do snapshot '{}'",
                    dest_path.display(),
                    h,
                    source_hash
                );
                verification_failed = true;
            }
        }
        Err(ref e) => {
            tracing::error!(
                "Falha ao calcular hash pós-escrita em '{}': {}",
                dest_path.display(),
                e
            );
            verification_failed = true;
        }
    }

    if verification_failed {
        return apply_safety_rollback(dest_path, safety_backup_path.as_deref())
            .await
            .map(|_| unreachable!());
    }

    let dest_hash = dest_hash_res.unwrap();

    let metadata_fs = tokio::fs::metadata(dest_path).await?;
    let bytes_restored = metadata_fs.len();

    let report = RestoreReport {
        snapshot_id: metadata.backup.id,
        file_id: metadata.file_entry.id,
        version: metadata.backup.version,
        source_path,
        dest_path: dest_path.clone(),
        safety_backup_path,
        restored_hash: dest_hash,
        bytes_restored,
    };

    tracing::info!(
        "Restauração concluída: snapshot v{} restaurado em {} ({} bytes, hash: {})",
        report.version,
        report.dest_path.display(),
        report.bytes_restored,
        report.restored_hash
    );

    Ok(report)
}

/// Aplica rollback de segurança caso a validação pós-escrita falhe.
pub async fn apply_safety_rollback(
    dest_path: &Path,
    safety_backup_path: Option<&Path>,
) -> Result<(), RestoreError> {
    if let Some(sb_path) = safety_backup_path {
        tracing::warn!(
            "Iniciando rollback automático de '{}' a partir do safety backup '{}'...",
            dest_path.display(),
            sb_path.display()
        );
        match tokio::fs::copy(sb_path, dest_path).await {
            Ok(_) => {
                tracing::info!(
                    "Rollback concluído com sucesso. Estado anterior de '{}' foi restaurado.",
                    dest_path.display()
                );
                Err(RestoreError::PostWriteValidationFailedWithRollback {
                    path: dest_path.to_path_buf(),
                    safety_backup: sb_path.to_path_buf(),
                })
            }
            Err(rb_err) => {
                tracing::error!(
                    "CRÍTICO: Falha ao aplicar rollback do safety backup '{}' sobre '{}': {}",
                    sb_path.display(),
                    dest_path.display(),
                    rb_err
                );
                Err(RestoreError::PostWriteValidationAndRollbackFailed {
                    path: dest_path.to_path_buf(),
                    safety_backup: sb_path.to_path_buf(),
                    rollback_error: rb_err.to_string(),
                })
            }
        }
    } else {
        let _ = tokio::fs::remove_file(dest_path).await;
        Err(RestoreError::PostWriteValidationFailed(
            dest_path.to_path_buf(),
        ))
    }
}

/// Restaura um snapshot para o local de save original de forma segura e atômica.
pub async fn restore_snapshot(
    conn: &Connection,
    backup_root: &Path,
    request: &RestoreRequest,
) -> Result<RestoreReport, RestoreError> {
    let metadata = load_restore_metadata(conn, request.snapshot_id)?;
    let report = execute_restore_filesystem(backup_root, &metadata, request).await?;
    update_restored_file_entry(
        conn,
        &metadata.file_entry,
        &report.restored_hash,
        report.bytes_restored,
    )?;
    Ok(report)
}

/// Restaura um snapshot para o local de save original utilizando um Mutex compartilhado de conexão SQLite,
/// liberando o lock durante as operações assíncronas de I/O de arquivos.
pub async fn restore_snapshot_with_db(
    db: &tokio::sync::Mutex<Connection>,
    backup_root: &Path,
    request: &RestoreRequest,
) -> Result<RestoreReport, RestoreError> {
    restore_snapshot_with_db_and_drive(db, backup_root, None, request).await
}

/// Se o snapshot físico local estiver ausente do disco (ex: limpo/podado ou restaurando de novo dispositivo),
/// e houver uma pasta do Google Drive configurada, busca e valida o snapshot a partir do Drive
/// para a pasta local de staging/backup antes de aplicar a restauração transacional.
pub async fn stage_snapshot_from_drive_if_needed(
    backup_root: &Path,
    drive_dir: &Path,
    metadata: &RestoreMetadata,
    game: &Game,
    filename: &str,
) -> Result<PathBuf, RestoreError> {
    use savevault_backup::storage::{
        BackupStorage, FetchSnapshotParams, FileAvailability, FilesystemBackupStorage, StorageError,
    };

    let target_local_path = backup_root
        .join(metadata.backup.file_id.to_string())
        .join(format!("v{}", metadata.backup.version))
        .join(filename);

    if target_local_path.exists() {
        return Ok(target_local_path);
    }

    let storage = FilesystemBackupStorage::new(drive_dir.to_path_buf());
    let folder_status = storage.validate_folder();
    if !folder_status.is_available() {
        return Err(RestoreError::Other(format!(
            "Pasta do Google Drive indisponível para recuperar snapshot ausente: {:?}",
            folder_status
        )));
    }

    let staging_dir = backup_root
        .join(metadata.backup.file_id.to_string())
        .join(format!("v{}", metadata.backup.version));
    tokio::fs::create_dir_all(&staging_dir).await?;

    let fetch_params = FetchSnapshotParams {
        platform: game.platform.clone(),
        game_name: game.name.clone(),
        game_id: Some(game.id),
        version: metadata.backup.version,
        file_name: filename.to_string(),
        staging_dir: staging_dir.clone(),
        expected_hash: Some(metadata.file_entry.hash.clone()),
    };

    let staged_file = storage
        .fetch_snapshot_to_staging(fetch_params)
        .map_err(|e| match e {
            StorageError::SnapshotAvailabilityError { status, path } => match status {
                FileAvailability::NotFound => RestoreError::PhysicalSnapshotMissing(path),
                FileAvailability::Unavailable(reason) => RestoreError::Other(format!(
                    "Snapshot no Google Drive está em modo 'Online Only' ou indisponível: {}",
                    reason
                )),
                FileAvailability::AccessDenied(reason) => RestoreError::Other(format!(
                    "Acesso negado ao snapshot no Google Drive: {}",
                    reason
                )),
                FileAvailability::Available => unreachable!(),
            },
            StorageError::IntegrityMismatch { expected, actual } => {
                RestoreError::IntegrityCheckFailed(metadata.backup.id, expected, actual)
            }
            other => RestoreError::Other(format!("Falha ao obter snapshot do Drive: {}", other)),
        })?;

    let final_dest = staging_dir.join(filename);
    if staged_file != final_dest {
        tokio::fs::rename(&staged_file, &final_dest).await?;
    }

    Ok(final_dest)
}

/// Restaura um snapshot para o local de save original com suporte opcional à recuperação
/// transparente a partir da pasta sincronizada do Google Drive para computador.
pub async fn restore_snapshot_with_db_and_drive(
    db: &tokio::sync::Mutex<Connection>,
    backup_root: &Path,
    drive_dir: Option<&Path>,
    request: &RestoreRequest,
) -> Result<RestoreReport, RestoreError> {
    let (metadata, game) = {
        let conn = db.lock().await;
        let meta = load_restore_metadata(&conn, request.snapshot_id)?;
        let games_repo = savevault_db::repositories::GamesRepo::new(&conn);
        let g = games_repo
            .find_by_id(meta.save_location.game_id)
            .map_err(|e| RestoreError::Other(e.to_string()))?
            .ok_or_else(|| RestoreError::Other("Jogo associado não encontrado no banco".into()))?;
        (meta, g)
    };

    let filename = metadata
        .file_entry
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| {
            RestoreError::PathSecurityViolation(format!(
                "Caminho de destino não possui nome de arquivo válido: '{}'",
                metadata.file_entry.path.display()
            ))
        })?;

    // Se o snapshot local não existir e drive_dir for fornecido, busca do Drive com integridade e staging
    let local_expected = backup_root
        .join(metadata.backup.file_id.to_string())
        .join(format!("v{}", metadata.backup.version))
        .join(filename);

    if !local_expected.exists() {
        if let Some(drive) = drive_dir {
            let _ =
                stage_snapshot_from_drive_if_needed(backup_root, drive, &metadata, &game, filename)
                    .await?;
        }
    }

    let report = execute_restore_filesystem(backup_root, &metadata, request).await?;
    {
        let conn = db.lock().await;
        update_restored_file_entry(
            &conn,
            &metadata.file_entry,
            &report.restored_hash,
            report.bytes_restored,
        )?;
    }
    Ok(report)
}

/// Atalho ergonômico para restaurar diretamente pelo UUID do snapshot.
pub async fn restore_by_id(
    conn: &Connection,
    backup_root: &Path,
    snapshot_id: Uuid,
) -> Result<RestoreReport, RestoreError> {
    restore_snapshot(conn, backup_root, &RestoreRequest::new(snapshot_id)).await
}

use std::sync::RwLock;

static ACTIVE_RESTORE_PATHS: std::sync::LazyLock<RwLock<std::collections::HashSet<PathBuf>>> =
    std::sync::LazyLock::new(|| RwLock::new(std::collections::HashSet::new()));

/// RAII Guard que registra um conjunto de caminhos como sob restauração ativa
/// para suprimir eventos concorrentes de watcher e evitar condições de corrida (SV-ADV-010).
pub struct ActiveRestoreGuard {
    paths: Vec<PathBuf>,
}

impl ActiveRestoreGuard {
    pub fn new(paths: &[PathBuf]) -> Self {
        let mut canon_paths = Vec::new();
        if let Ok(mut set) = ACTIVE_RESTORE_PATHS.write() {
            for p in paths {
                let canon = savevault_core::path::canonicalize_windows_path(p)
                    .unwrap_or_else(|_| p.clone());
                set.insert(canon.clone());
                canon_paths.push(canon);
            }
        }
        Self { paths: canon_paths }
    }
}

impl Drop for ActiveRestoreGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = ACTIVE_RESTORE_PATHS.write() {
            for p in &self.paths {
                set.remove(p);
            }
        }
    }
}

/// Verifica se um determinado caminho está atualmente sob processo de restauração ativa (SV-ADV-010).
pub fn is_path_under_active_restore(path: &Path) -> bool {
    if let Ok(set) = ACTIVE_RESTORE_PATHS.read() {
        if let Ok(canon) = savevault_core::path::canonicalize_windows_path(path) {
            return set.contains(&canon);
        }
        return set.contains(path);
    }
    false
}

/// Varre um diretório recursivamente e remove com segurança arquivos de staging
/// órfãos decorrentes de encerramentos abruptos anteriores (SV-ADV-011).
pub fn cleanup_stale_staging_files(dir: &Path) -> std::io::Result<usize> {
    if !dir.exists() || !dir.is_dir() {
        return Ok(0);
    }
    let mut removed_count = 0;
    for entry in walkdir::WalkDir::new(dir)
        .min_depth(1)
        .max_depth(5)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() {
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                let is_staging = file_name.contains(".restore_tmp_")
                    || (file_name.starts_with('.') && file_name.contains(".download_tmp."))
                    || (file_name.starts_with('.') && file_name.contains(".tmp."))
                    || file_name.starts_with("manifest.yaml.tmp.");
                if is_staging {
                    if let Ok(meta) = std::fs::symlink_metadata(path) {
                        if !meta.file_type().is_symlink() && std::fs::remove_file(path).is_ok() {
                            tracing::info!(
                                "Arquivo de staging órfão removido com sucesso: {}",
                                path.display()
                            );
                            removed_count += 1;
                        }
                    }
                }
            }
        }
    }
    Ok(removed_count)
}

#[derive(Debug, Clone)]
pub struct RestoreTransactionItem {
    pub request: RestoreRequest,
    pub metadata: RestoreMetadata,
    pub source_path: PathBuf,
    pub source_hash: String,
    pub safety_backup_path: Option<PathBuf>,
    pub original_dest_hash: Option<String>,
    pub existed_prior: bool,
    pub committed: bool,
}

/// Transação atômica de restauração para múltiplos arquivos com Two-Phase Commit e Rollback Global (NEW-FINDING-001).
pub struct RestoreTransaction {
    backup_root: PathBuf,
    items: Vec<RestoreTransactionItem>,
}

impl RestoreTransaction {
    pub fn new(backup_root: impl Into<PathBuf>) -> Self {
        Self {
            backup_root: backup_root.into(),
            items: Vec::new(),
        }
    }

    pub async fn add_request(
        &mut self,
        conn: &Connection,
        request: RestoreRequest,
    ) -> Result<(), RestoreError> {
        let metadata = load_restore_metadata(conn, request.snapshot_id)?;
        let dest_path = &metadata.file_entry.path;

        // 1. Validação de segurança de caminho
        validate_destination_security(dest_path, &metadata.save_location)?;

        // 2. Previne destinos duplicados ou conflitantes na mesma transação
        if self.items.iter().any(|item| {
            savevault_core::path::paths_are_equivalent(&item.metadata.file_entry.path, dest_path)
        }) {
            return Err(RestoreError::PathSecurityViolation(format!(
                "Destino duplicado na mesma transação de restore: '{}'",
                dest_path.display()
            )));
        }

        let filename = dest_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| {
                RestoreError::PathSecurityViolation(format!(
                    "Caminho de destino não possui nome de arquivo válido: '{}'",
                    dest_path.display()
                ))
            })?;

        let source_path = validate_snapshot_security(
            &self.backup_root,
            metadata.backup.file_id,
            metadata.backup.version,
            filename,
        )?;

        if !source_path.exists() {
            return Err(RestoreError::PhysicalSnapshotMissing(source_path));
        }
        if source_path.is_dir() {
            return Err(RestoreError::InvalidSnapshotSource(source_path));
        }

        let source_hash = savevault_backup::hasher::hash_file(&source_path)
            .await
            .map_err(|e| {
                RestoreError::Other(format!("Falha ao calcular hash do snapshot: {}", e))
            })?;

        if let Some(ref expected) = request.expected_hash {
            if &source_hash != expected {
                return Err(RestoreError::IntegrityCheckFailed(
                    request.snapshot_id,
                    expected.clone(),
                    source_hash,
                ));
            }
        }

        self.items.push(RestoreTransactionItem {
            request,
            metadata,
            source_path,
            source_hash,
            safety_backup_path: None,
            original_dest_hash: None,
            existed_prior: false,
            committed: false,
        });

        Ok(())
    }

    /// Executa o Two-Phase Commit com proteção de rollback global.
    pub async fn execute(mut self, conn: &Connection) -> Result<Vec<RestoreReport>, RestoreError> {
        if self.items.is_empty() {
            return Ok(Vec::new());
        }

        let target_paths: Vec<PathBuf> = self
            .items
            .iter()
            .map(|it| it.metadata.file_entry.path.clone())
            .collect();
        let _guard = ActiveRestoreGuard::new(&target_paths);

        // FASE 1: PREPARE & BACKUP ALL
        for item in &mut self.items {
            let dest_path = &item.metadata.file_entry.path;
            if dest_path.exists() {
                item.existed_prior = true;
                let orig_hash = savevault_backup::hasher::hash_file(dest_path).await.ok();
                item.original_dest_hash = orig_hash;

                let dest_parent = dest_path.parent().unwrap_or_else(|| Path::new("."));
                let orig_name = dest_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("save.dat");

                let mut candidate;
                let mut counter = 0u32;
                loop {
                    let unique_suffix = format!("{}_{:04x}", Uuid::new_v4().simple(), counter);
                    let safety_name = format!("{}.savevault_safety_{}", orig_name, unique_suffix);
                    candidate = dest_parent.join(safety_name);
                    if !candidate.exists() {
                        break;
                    }
                    counter += 1;
                }

                if let Err(e) = tokio::fs::copy(dest_path, &candidate).await {
                    return Err(RestoreError::SafetyBackupFailed(
                        candidate,
                        format!("Falha ao criar safety backup pré-transacional: {}", e),
                    ));
                }
                item.safety_backup_path = Some(candidate);
            }
        }

        // FASE 2: COMMIT ALL (Escritas atômicas de staging)
        for i in 0..self.items.len() {
            let dest_path = self.items[i].metadata.file_entry.path.clone();
            let source_path = self.items[i].source_path.clone();
            let source_hash = self.items[i].source_hash.clone();
            let dest_parent = dest_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf();

            if !dest_parent.exists() {
                if let Err(e) = tokio::fs::create_dir_all(&dest_parent).await {
                    return Err(self
                        .rollback_all(&dest_path, format!("Falha ao criar diretório pai: {}", e))
                        .await);
                }
            }

            let orig_name = dest_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("save.dat");
            let tmp_path = dest_parent.join(format!(
                "{}.restore_tmp_{}",
                orig_name,
                Uuid::new_v4().simple()
            ));

            if let Err(e) = tokio::fs::copy(&source_path, &tmp_path).await {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(self
                    .rollback_all(&dest_path, format!("Falha ao copiar para staging: {}", e))
                    .await);
            }

            let tmp_hash = match savevault_backup::hasher::hash_file(&tmp_path).await {
                Ok(h) => h,
                Err(e) => {
                    let _ = tokio::fs::remove_file(&tmp_path).await;
                    return Err(self
                        .rollback_all(&dest_path, format!("Falha ao calcular hash staging: {}", e))
                        .await);
                }
            };

            if tmp_hash != source_hash {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(self
                    .rollback_all(
                        &dest_path,
                        format!(
                            "Integridade do staging falhou: {} != {}",
                            tmp_hash, source_hash
                        ),
                    )
                    .await);
            }

            if let Err(e) = tokio::fs::rename(&tmp_path, &dest_path).await {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(self
                    .rollback_all(&dest_path, format!("Falha no rename atômico: {}", e))
                    .await);
            }

            self.items[i].committed = true;
        }

        // FASE 3: VERIFY ALL (Integridade física pós-escrita)
        for i in 0..self.items.len() {
            let dest_path = self.items[i].metadata.file_entry.path.clone();
            let source_hash = self.items[i].source_hash.clone();
            match savevault_backup::hasher::hash_file(&dest_path).await {
                Ok(h) => {
                    if h != source_hash {
                        return Err(self
                            .rollback_all(
                                &dest_path,
                                format!("Hash pós-escrita difere em '{}'", dest_path.display()),
                            )
                            .await);
                    }
                }
                Err(e) => {
                    return Err(self
                        .rollback_all(
                            &dest_path,
                            format!(
                                "Falha ao ler hash pós-escrita em '{}': {}",
                                dest_path.display(),
                                e
                            ),
                        )
                        .await);
                }
            }
        }

        // FASE 4: DATABASE PERSISTENCE (Transação SQLite Única)
        let mut reports = Vec::new();
        {
            let tx = conn.unchecked_transaction()?;
            let files_repo = FilesRepo::new(&tx);
            for item in &self.items {
                let file_entry = FileEntry {
                    id: item.metadata.file_entry.id,
                    save_location_id: item.metadata.file_entry.save_location_id,
                    path: item.metadata.file_entry.path.clone(),
                    hash: item.source_hash.clone(),
                    size_bytes: std::fs::metadata(&item.metadata.file_entry.path)
                        .map(|m| m.len())
                        .unwrap_or(0),
                    modified_at: Utc::now(),
                };
                files_repo
                    .upsert(&file_entry)
                    .map_err(|e| RestoreError::Other(e.to_string()))?;

                reports.push(RestoreReport {
                    snapshot_id: item.request.snapshot_id,
                    file_id: item.metadata.backup.file_id,
                    version: item.metadata.backup.version,
                    source_path: item.source_path.clone(),
                    dest_path: item.metadata.file_entry.path.clone(),
                    safety_backup_path: item.safety_backup_path.clone(),
                    restored_hash: item.source_hash.clone(),
                    bytes_restored: file_entry.size_bytes,
                });
            }
            tx.commit()?;
        }

        Ok(reports)
    }

    async fn rollback_all(&self, failed_file: &Path, primary_error: String) -> RestoreError {
        let mut rolled_back = Vec::new();
        let mut failed_rollbacks = Vec::new();
        let mut preserved_backups = Vec::new();

        for item in self.items.iter().rev() {
            let dest_path = &item.metadata.file_entry.path;
            if let Some(ref sb) = item.safety_backup_path {
                preserved_backups.push(sb.clone());
            }

            if item.committed {
                if item.existed_prior {
                    if let Some(ref sb) = item.safety_backup_path {
                        if let Err(e) = tokio::fs::copy(sb, dest_path).await {
                            failed_rollbacks.push((dest_path.clone(), e.to_string()));
                        } else {
                            rolled_back.push(dest_path.clone());
                        }
                    } else {
                        failed_rollbacks.push((
                            dest_path.clone(),
                            "Safety backup ausente para arquivo pré-existente".into(),
                        ));
                    }
                } else {
                    let _ = tokio::fs::remove_file(dest_path).await;
                    rolled_back.push(dest_path.clone());
                }
            }
        }

        if failed_rollbacks.is_empty() {
            RestoreError::MultiFileTransactionFailedWithRollback(format!(
                "Falha em '{}': {}. Rollback global completado para {} arquivo(s).",
                failed_file.display(),
                primary_error,
                rolled_back.len()
            ))
        } else {
            RestoreError::MultiFileTransactionAndRollbackFailed(Box::new(
                MultiFileRollbackFailureReport {
                    failed_file: failed_file.to_path_buf(),
                    primary_error,
                    rolled_back_files: rolled_back,
                    failed_rollback_files: failed_rollbacks,
                    preserved_safety_backups: preserved_backups,
                },
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use savevault_core::models::{
        Backup, Confidence, Game, Platform, SaveLocation, SaveLocationType, SyncStatus,
    };
    use savevault_db::repositories::{BackupsRepo, FilesRepo, GamesRepo, SaveLocationsRepo};
    use tempfile::tempdir;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        savevault_db::migrations::run(&conn).unwrap();
        conn
    }

    #[tokio::test]
    async fn test_restore_valid_snapshot_replaces_destination_and_creates_safety_backup() {
        let conn = setup_test_db();
        let dir = tempdir().unwrap();
        let backup_root = dir.path().join("backups");
        let save_dir = dir.path().join("saves");
        std::fs::create_dir_all(&save_dir).unwrap();

        let save_file = save_dir.join("player.sav");
        std::fs::write(&save_file, "save_conteudo_atual_v3").unwrap();

        let games_repo = GamesRepo::new(&conn);
        let loc_repo = SaveLocationsRepo::new(&conn);
        let files_repo = FilesRepo::new(&conn);
        let backups_repo = BackupsRepo::new(&conn);

        let game = Game::new("RPG Hero", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();

        let loc = SaveLocation::new(game.id, &save_dir, SaveLocationType::Directory);
        let saved_loc = loc_repo.upsert(&loc).unwrap();

        let file_id = Uuid::new_v4();
        let file_entry = FileEntry {
            id: file_id,
            save_location_id: saved_loc.id,
            path: save_file.clone(),
            hash: "dummy_hash_v3".into(),
            size_bytes: 20,
            modified_at: Utc::now(),
        };
        files_repo.upsert(&file_entry).unwrap();

        // Cria snapshot v1 no disco
        let v1_dir = backup_root.join(file_id.to_string()).join("v1");
        std::fs::create_dir_all(&v1_dir).unwrap();
        let v1_file = v1_dir.join("player.sav");
        std::fs::write(&v1_file, "save_conteudo_original_v1").unwrap();

        let backup = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 1,
            cloud_path: None,
            drive_file_id: None,
            sync_status: SyncStatus::Pending,
            created_at: Utc::now(),
        };
        backups_repo.insert(&backup).unwrap();

        // Executa restauração
        let report = restore_by_id(&conn, &backup_root, backup.id).await.unwrap();

        assert_eq!(report.snapshot_id, backup.id);
        assert_eq!(report.version, 1);
        assert_eq!(report.dest_path, save_file);

        // Valida que o destino tem o conteúdo de v1
        let restored_content = std::fs::read_to_string(&save_file).unwrap();
        assert_eq!(restored_content, "save_conteudo_original_v1");

        // Valida que o safety backup foi criado com o conteúdo atual
        assert!(report.safety_backup_path.is_some());
        let safety_path = report.safety_backup_path.unwrap();
        assert!(safety_path.exists());
        let safety_content = std::fs::read_to_string(&safety_path).unwrap();
        assert_eq!(safety_content, "save_conteudo_atual_v3");

        // Valida que o snapshot original de v1 continua intacto
        assert!(v1_file.exists());
        assert_eq!(
            std::fs::read_to_string(&v1_file).unwrap(),
            "save_conteudo_original_v1"
        );

        // Valida que o registro no SQLite foi atualizado
        let updated_file = files_repo.find_by_id(file_id).unwrap().unwrap();
        assert_eq!(updated_file.hash, report.restored_hash);
        assert_eq!(
            updated_file.size_bytes,
            "save_conteudo_original_v1".len() as u64
        );
    }

    #[tokio::test]
    async fn test_restore_with_nonexistent_destination_creates_file_without_safety_backup() {
        let conn = setup_test_db();
        let dir = tempdir().unwrap();
        let backup_root = dir.path().join("backups");
        let save_dir = dir.path().join("saves_lost");

        let save_file = save_dir.join("lost_save.dat");
        assert!(!save_file.exists());

        let games_repo = GamesRepo::new(&conn);
        let loc_repo = SaveLocationsRepo::new(&conn);
        let files_repo = FilesRepo::new(&conn);
        let backups_repo = BackupsRepo::new(&conn);

        let game = Game::new("Lost Save Game", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();

        let loc = SaveLocation::new(game.id, &save_dir, SaveLocationType::Directory);
        let saved_loc = loc_repo.upsert(&loc).unwrap();

        let file_id = Uuid::new_v4();
        let file_entry = FileEntry {
            id: file_id,
            save_location_id: saved_loc.id,
            path: save_file.clone(),
            hash: "hash_zero".into(),
            size_bytes: 0,
            modified_at: Utc::now(),
        };
        files_repo.upsert(&file_entry).unwrap();

        let v1_dir = backup_root.join(file_id.to_string()).join("v1");
        std::fs::create_dir_all(&v1_dir).unwrap();
        let v1_file = v1_dir.join("lost_save.dat");
        std::fs::write(&v1_file, "conteudo_recuperado").unwrap();

        let backup = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 1,
            cloud_path: None,
            drive_file_id: None,
            sync_status: SyncStatus::Pending,
            created_at: Utc::now(),
        };
        backups_repo.insert(&backup).unwrap();

        let report = restore_by_id(&conn, &backup_root, backup.id).await.unwrap();

        assert_eq!(report.dest_path, save_file);
        assert!(save_file.exists());
        assert_eq!(
            std::fs::read_to_string(&save_file).unwrap(),
            "conteudo_recuperado"
        );
        assert!(
            report.safety_backup_path.is_none(),
            "Nenhum safety backup se arquivo não existia"
        );
    }

    #[tokio::test]
    async fn test_restore_nonexistent_snapshot_id_returns_snapshot_not_found() {
        let conn = setup_test_db();
        let dir = tempdir().unwrap();
        let backup_root = dir.path().join("backups");

        let missing_id = Uuid::new_v4();
        let err = restore_by_id(&conn, &backup_root, missing_id)
            .await
            .unwrap_err();

        match err {
            RestoreError::SnapshotNotFound(id) => assert_eq!(id, missing_id),
            other => panic!("Esperado SnapshotNotFound, obtido: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_restore_missing_physical_snapshot_returns_physical_snapshot_missing() {
        let conn = setup_test_db();
        let dir = tempdir().unwrap();
        let backup_root = dir.path().join("backups");
        let save_dir = dir.path().join("saves");
        std::fs::create_dir_all(&save_dir).unwrap();
        let save_file = save_dir.join("save.dat");
        std::fs::write(&save_file, "atual").unwrap();

        let games_repo = GamesRepo::new(&conn);
        let loc_repo = SaveLocationsRepo::new(&conn);
        let files_repo = FilesRepo::new(&conn);
        let backups_repo = BackupsRepo::new(&conn);

        let game = Game::new("Game", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();
        let loc = SaveLocation::new(game.id, &save_dir, SaveLocationType::Directory);
        let saved_loc = loc_repo.upsert(&loc).unwrap();

        let file_id = Uuid::new_v4();
        let file_entry = FileEntry {
            id: file_id,
            save_location_id: saved_loc.id,
            path: save_file.clone(),
            hash: "h".into(),
            size_bytes: 5,
            modified_at: Utc::now(),
        };
        files_repo.upsert(&file_entry).unwrap();

        let backup = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 1,
            cloud_path: None,
            drive_file_id: None,
            sync_status: SyncStatus::Pending,
            created_at: Utc::now(),
        };
        backups_repo.insert(&backup).unwrap();

        // Não cria o arquivo de snapshot no disco
        let err = restore_by_id(&conn, &backup_root, backup.id)
            .await
            .unwrap_err();

        match err {
            RestoreError::PhysicalSnapshotMissing(_) => {}
            other => panic!("Esperado PhysicalSnapshotMissing, obtido: {:?}", other),
        }

        // Garante que o save atual e o diretório permanecem inalterados
        assert_eq!(std::fs::read_to_string(&save_file).unwrap(), "atual");
    }

    #[tokio::test]
    async fn test_restore_corrupted_snapshot_integrity_check_fails() {
        let conn = setup_test_db();
        let dir = tempdir().unwrap();
        let backup_root = dir.path().join("backups");
        let save_dir = dir.path().join("saves");
        std::fs::create_dir_all(&save_dir).unwrap();
        let save_file = save_dir.join("save.dat");
        std::fs::write(&save_file, "save_original_intacto").unwrap();

        let games_repo = GamesRepo::new(&conn);
        let loc_repo = SaveLocationsRepo::new(&conn);
        let files_repo = FilesRepo::new(&conn);
        let backups_repo = BackupsRepo::new(&conn);

        let game = Game::new("Game", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();
        let loc = SaveLocation::new(game.id, &save_dir, SaveLocationType::Directory);
        let saved_loc = loc_repo.upsert(&loc).unwrap();

        let file_id = Uuid::new_v4();
        let file_entry = FileEntry {
            id: file_id,
            save_location_id: saved_loc.id,
            path: save_file.clone(),
            hash: "h".into(),
            size_bytes: 20,
            modified_at: Utc::now(),
        };
        files_repo.upsert(&file_entry).unwrap();

        let v1_dir = backup_root.join(file_id.to_string()).join("v1");
        std::fs::create_dir_all(&v1_dir).unwrap();
        let v1_file = v1_dir.join("save.dat");
        std::fs::write(&v1_file, "conteudo_corrompido_ou_alterado").unwrap();

        let backup = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 1,
            cloud_path: None,
            drive_file_id: None,
            sync_status: SyncStatus::Pending,
            created_at: Utc::now(),
        };
        backups_repo.insert(&backup).unwrap();

        // Fornece um hash esperado divergente
        let req = RestoreRequest::with_expected_hash(backup.id, "hash_esperado_que_nao_bate");
        let err = restore_snapshot(&conn, &backup_root, &req)
            .await
            .unwrap_err();

        match err {
            RestoreError::IntegrityCheckFailed(id, exp, _) => {
                assert_eq!(id, backup.id);
                assert_eq!(exp, "hash_esperado_que_nao_bate");
            }
            other => panic!("Esperado IntegrityCheckFailed, obtido: {:?}", other),
        }

        // Destino não deve ser modificado
        assert_eq!(
            std::fs::read_to_string(&save_file).unwrap(),
            "save_original_intacto"
        );
    }

    #[tokio::test]
    async fn test_restore_consecutive_restores_create_distinct_safety_backups() {
        let conn = setup_test_db();
        let dir = tempdir().unwrap();
        let backup_root = dir.path().join("backups");
        let save_dir = dir.path().join("saves");
        std::fs::create_dir_all(&save_dir).unwrap();
        let save_file = save_dir.join("save.dat");
        std::fs::write(&save_file, "estado_inicial_0").unwrap();

        let games_repo = GamesRepo::new(&conn);
        let loc_repo = SaveLocationsRepo::new(&conn);
        let files_repo = FilesRepo::new(&conn);
        let backups_repo = BackupsRepo::new(&conn);

        let game = Game::new("Game", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();
        let loc = SaveLocation::new(game.id, &save_dir, SaveLocationType::Directory);
        let saved_loc = loc_repo.upsert(&loc).unwrap();

        let file_id = Uuid::new_v4();
        files_repo
            .upsert(&FileEntry {
                id: file_id,
                save_location_id: saved_loc.id,
                path: save_file.clone(),
                hash: "h0".into(),
                size_bytes: 16,
                modified_at: Utc::now(),
            })
            .unwrap();

        // Snapshots v1 e v2
        let v1_dir = backup_root.join(file_id.to_string()).join("v1");
        std::fs::create_dir_all(&v1_dir).unwrap();
        std::fs::write(v1_dir.join("save.dat"), "dados_versao_1").unwrap();
        let b1 = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 1,
            cloud_path: None,
            drive_file_id: None,
            sync_status: SyncStatus::Pending,
            created_at: Utc::now(),
        };
        backups_repo.insert(&b1).unwrap();

        let v2_dir = backup_root.join(file_id.to_string()).join("v2");
        std::fs::create_dir_all(&v2_dir).unwrap();
        std::fs::write(v2_dir.join("save.dat"), "dados_versao_2").unwrap();
        let b2 = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 2,
            cloud_path: None,
            drive_file_id: None,
            sync_status: SyncStatus::Pending,
            created_at: Utc::now(),
        };
        backups_repo.insert(&b2).unwrap();

        // 1. Restaura v1
        let rep1 = restore_by_id(&conn, &backup_root, b1.id).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&save_file).unwrap(),
            "dados_versao_1"
        );
        let safety1 = rep1.safety_backup_path.unwrap();
        assert_eq!(
            std::fs::read_to_string(&safety1).unwrap(),
            "estado_inicial_0"
        );

        // 2. Restaura v2
        let rep2 = restore_by_id(&conn, &backup_root, b2.id).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&save_file).unwrap(),
            "dados_versao_2"
        );
        let safety2 = rep2.safety_backup_path.unwrap();
        assert_eq!(std::fs::read_to_string(&safety2).unwrap(), "dados_versao_1");

        // Ambos os safety backups devem ser distintos e coexistir
        assert_ne!(safety1, safety2);
        assert!(safety1.exists());
        assert!(safety2.exists());
    }

    #[tokio::test]
    async fn test_restore_repeated_same_snapshot_idempotent() {
        let conn = setup_test_db();
        let dir = tempdir().unwrap();
        let backup_root = dir.path().join("backups");
        let save_dir = dir.path().join("saves");
        std::fs::create_dir_all(&save_dir).unwrap();
        let save_file = save_dir.join("save.dat");
        std::fs::write(&save_file, "save_atual").unwrap();

        let games_repo = GamesRepo::new(&conn);
        let loc_repo = SaveLocationsRepo::new(&conn);
        let files_repo = FilesRepo::new(&conn);
        let backups_repo = BackupsRepo::new(&conn);

        let game = Game::new("Game", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();
        let loc = SaveLocation::new(game.id, &save_dir, SaveLocationType::Directory);
        let saved_loc = loc_repo.upsert(&loc).unwrap();

        let file_id = Uuid::new_v4();
        files_repo
            .upsert(&FileEntry {
                id: file_id,
                save_location_id: saved_loc.id,
                path: save_file.clone(),
                hash: "h".into(),
                size_bytes: 10,
                modified_at: Utc::now(),
            })
            .unwrap();

        let v1_dir = backup_root.join(file_id.to_string()).join("v1");
        std::fs::create_dir_all(&v1_dir).unwrap();
        std::fs::write(v1_dir.join("save.dat"), "conteudo_v1").unwrap();

        let b1 = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 1,
            cloud_path: None,
            drive_file_id: None,
            sync_status: SyncStatus::Pending,
            created_at: Utc::now(),
        };
        backups_repo.insert(&b1).unwrap();

        // Restauração 1
        let rep1 = restore_by_id(&conn, &backup_root, b1.id).await.unwrap();
        assert_eq!(std::fs::read_to_string(&save_file).unwrap(), "conteudo_v1");

        // Restauração 2 (mesmo snapshot)
        let rep2 = restore_by_id(&conn, &backup_root, b1.id).await.unwrap();
        assert_eq!(std::fs::read_to_string(&save_file).unwrap(), "conteudo_v1");
        assert_eq!(rep1.restored_hash, rep2.restored_hash);
    }

    #[tokio::test]
    async fn test_restore_blocks_path_traversal_parent_dir() {
        let conn = setup_test_db();
        let dir = tempdir().unwrap();
        let backup_root = dir.path().join("backups");
        let save_dir = dir.path().join("saves");

        let games_repo = GamesRepo::new(&conn);
        let loc_repo = SaveLocationsRepo::new(&conn);
        let files_repo = FilesRepo::new(&conn);
        let backups_repo = BackupsRepo::new(&conn);

        let game = Game::new("Game", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();
        let loc = SaveLocation::new(game.id, &save_dir, SaveLocationType::Directory);
        let saved_loc = loc_repo.upsert(&loc).unwrap();

        let file_id = Uuid::new_v4();
        // Caminho malicioso com '..'
        let malicious_path = save_dir.join("..").join("secret.txt");
        files_repo
            .upsert(&FileEntry {
                id: file_id,
                save_location_id: saved_loc.id,
                path: malicious_path,
                hash: "h".into(),
                size_bytes: 1,
                modified_at: Utc::now(),
            })
            .unwrap();

        let b = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 1,
            cloud_path: None,
            drive_file_id: None,
            sync_status: SyncStatus::Pending,
            created_at: Utc::now(),
        };
        backups_repo.insert(&b).unwrap();

        let err = restore_by_id(&conn, &backup_root, b.id).await.unwrap_err();
        match err {
            RestoreError::PathSecurityViolation(reason) => {
                assert!(reason.contains(".."));
            }
            other => panic!("Esperado PathSecurityViolation, obtido: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_restore_blocks_destination_escaping_save_location() {
        let conn = setup_test_db();
        let dir = tempdir().unwrap();
        let backup_root = dir.path().join("backups");
        let save_dir = dir.path().join("game_a");
        let other_dir = dir.path().join("game_b");

        let games_repo = GamesRepo::new(&conn);
        let loc_repo = SaveLocationsRepo::new(&conn);
        let files_repo = FilesRepo::new(&conn);
        let backups_repo = BackupsRepo::new(&conn);

        let game = Game::new("Game", Platform::Generic, None, Confidence::High);
        games_repo.insert(&game).unwrap();
        let loc = SaveLocation::new(game.id, &save_dir, SaveLocationType::Directory);
        let saved_loc = loc_repo.upsert(&loc).unwrap();

        let file_id = Uuid::new_v4();
        // Arquivo pertence ao save_location game_a, mas o path aponta para game_b
        let escaping_path = other_dir.join("save.dat");
        files_repo
            .upsert(&FileEntry {
                id: file_id,
                save_location_id: saved_loc.id,
                path: escaping_path,
                hash: "h".into(),
                size_bytes: 1,
                modified_at: Utc::now(),
            })
            .unwrap();

        let b = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: 1,
            cloud_path: None,
            drive_file_id: None,
            sync_status: SyncStatus::Pending,
            created_at: Utc::now(),
        };
        backups_repo.insert(&b).unwrap();

        let err = restore_by_id(&conn, &backup_root, b.id).await.unwrap_err();
        match err {
            RestoreError::PathSecurityViolation(reason) => {
                assert!(reason.contains("escapa do SaveLocation"));
            }
            other => panic!("Esperado PathSecurityViolation, obtido: {:?}", other),
        }
    }
}
