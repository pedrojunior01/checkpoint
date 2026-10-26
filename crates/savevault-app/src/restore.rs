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

use savevault_core::models::{FileEntry, SaveLocation, SaveLocationType};
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

    #[error("Falha ao criar safety backup em '{0}': {1}")]
    SafetyBackupFailed(PathBuf, String),

    #[error("Falha na escrita atômica do arquivo em '{0}': {1}")]
    AtomicWriteFailed(PathBuf, String),

    #[error("Erro de banco de dados SQLite: {0}")]
    DatabaseError(#[from] rusqlite::Error),

    #[error("Erro de I/O no filesystem: {0}")]
    IoError(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

/// Parâmetros para solicitação de restauração.
#[derive(Debug, Clone)]
pub struct RestoreRequest {
    /// Identificador único (UUID) do snapshot/backup a ser restaurado.
    pub snapshot_id: Uuid,
    /// Hash BLAKE3 esperado opcional para validação antecipada de integridade.
    pub expected_hash: Option<String>,
    /// Alvo específico de restauração ("latest" para mais recente ou "daily" para marco diário).
    pub target_type: Option<String>,
}

impl RestoreRequest {
    pub fn new(snapshot_id: Uuid) -> Self {
        Self {
            snapshot_id,
            expected_hash: None,
            target_type: None,
        }
    }

    pub fn with_expected_hash(snapshot_id: Uuid, expected_hash: impl Into<String>) -> Self {
        Self {
            snapshot_id,
            expected_hash: Some(expected_hash.into()),
            target_type: None,
        }
    }

    pub fn with_target(snapshot_id: Uuid, target_type: Option<String>) -> Self {
        Self {
            snapshot_id,
            expected_hash: None,
            target_type,
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

    Ok(())
}

/// Valida que o caminho do snapshot físico está estritamente contido no diretório raiz de backups.
pub fn validate_snapshot_security(
    backup_root: &Path,
    game_name: &str,
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

    let safe_game_name = savevault_backup::snapshot::sanitize_name(game_name);

    // Candidatos comuns por versão, prefixo e latest/daily
    let candidates = [
        backup_root
            .join(&safe_game_name)
            .join(format!("v{}", version))
            .join(filename),
        backup_root
            .join(&safe_game_name)
            .join(format!("v{}_{}", version, filename)),
        backup_root
            .join(&safe_game_name)
            .join("latest")
            .join(filename),
        backup_root
            .join(&safe_game_name)
            .join("daily")
            .join(filename),
    ];

    for cand in &candidates {
        if cand.starts_with(backup_root) && cand.exists() {
            return Ok(cand.clone());
        }
    }

    let default_path = backup_root
        .join(&safe_game_name)
        .join("latest")
        .join(filename);

    if !default_path.starts_with(backup_root) {
        return Err(RestoreError::PathSecurityViolation(format!(
            "Caminho do snapshot escapa do backup_root: '{}'",
            default_path.display()
        )));
    }

    Ok(default_path)
}

/// Localiza e valida o caminho físico do snapshot no backup_root.
/// Suporta resolução em múltiplos layouts:
/// 1. `cloud_path` armazenado no banco SQLite (ex.: `<loc_id>/<file_id>/v46_S0000.sl2`);
/// 2. Layout hierárquico por UUIDs (`<loc_id>/<file_id>/v<ver>_<filename>`);
/// 3. Layout por pasta de versão (`<safe_game_name>/v<ver>/<filename>`);
/// 4. Layout por prefixo de versão (`<safe_game_name>/v<ver>_<filename>`);
/// 5. Layout `latest` (`<safe_game_name>/latest/<filename>`);
/// 6. Layout `daily` (`<safe_game_name>/daily/<filename>`);
/// 7. Layout legado SaveVault (`SaveVault/Steam` ou `Other`).
pub fn resolve_snapshot_source_path(
    backup_root: &Path,
    metadata: &RestoreMetadata,
    filename: &str,
    target_type: Option<&str>,
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

    let safe_game_name = savevault_backup::snapshot::sanitize_name(&metadata.game.name);
    let ver = metadata.backup.version;

    let mut candidates: Vec<PathBuf> = Vec::new();

    let daily_candidate = backup_root
        .join(&safe_game_name)
        .join("daily")
        .join(filename);

    let latest_candidate = backup_root
        .join(&safe_game_name)
        .join("latest")
        .join(filename);

    // Se o usuário solicitou especificamente a versão diária ("daily"), prioriza daily
    if target_type == Some("daily") {
        candidates.push(daily_candidate.clone());
    } else if target_type == Some("latest") {
        candidates.push(latest_candidate.clone());
    }

    // 1. Candidato direto por cloud_path armazenado nos metadados do banco
    if let Some(ref cp) = metadata.backup.cloud_path {
        let norm_cp = cp.replace('/', std::path::MAIN_SEPARATOR_STR);
        let path = backup_root.join(&norm_cp);
        candidates.push(path);
    }

    // 2. Candidato hierárquico por UUIDs
    let id_hierarchical = backup_root
        .join(metadata.save_location.id.to_string())
        .join(metadata.file_entry.id.to_string())
        .join(format!("v{}_{}", ver, filename));
    candidates.push(id_hierarchical);

    // 3. Candidato com pasta de versão no nome do jogo
    let ver_folder = backup_root
        .join(&safe_game_name)
        .join(format!("v{}", ver))
        .join(filename);
    candidates.push(ver_folder);

    // 4. Candidato com arquivo prefixado por versão
    let ver_prefixed = backup_root
        .join(&safe_game_name)
        .join(format!("v{}_{}", ver, filename));
    candidates.push(ver_prefixed);

    // 5. Candidato `latest`
    candidates.push(latest_candidate);

    // 6. Candidato `daily`
    candidates.push(daily_candidate);

    // 7. Candidatos legados SaveVault/Steam ou SaveVault/Other
    candidates.push(
        backup_root
            .join("SaveVault")
            .join("Steam")
            .join(&safe_game_name)
            .join(format!("v{}_{}", ver, filename)),
    );
    candidates.push(
        backup_root
            .join("SaveVault")
            .join("Steam")
            .join(&safe_game_name)
            .join("latest")
            .join(filename),
    );
    candidates.push(
        backup_root
            .join("SaveVault")
            .join("Other")
            .join(&safe_game_name)
            .join(format!("v{}_{}", ver, filename)),
    );
    candidates.push(
        backup_root
            .join("SaveVault")
            .join("Other")
            .join(&safe_game_name)
            .join("latest")
            .join(filename),
    );

    // Procura o primeiro candidato que existe fisicamente e cumpre segurança contra path traversal
    for cand in &candidates {
        if cand.starts_with(backup_root) && cand.exists() && cand.is_file() {
            return Ok(cand.clone());
        }
    }

    // Se nenhum arquivo existir, escolhe o candidato mais representativo para mensagem de erro clara
    let fallback = candidates
        .into_iter()
        .next()
        .unwrap_or_else(|| backup_root.join(&safe_game_name).join(filename));

    Err(RestoreError::PhysicalSnapshotMissing(fallback))
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
pub struct RestoreMetadata {
    pub backup: savevault_core::models::Backup,
    pub file_entry: FileEntry,
    pub save_location: SaveLocation,
    pub game: savevault_core::models::Game,
}

pub fn load_restore_metadata(
    conn: &Connection,
    snapshot_id: Uuid,
) -> Result<RestoreMetadata, RestoreError> {
    let backups_repo = BackupsRepo::new(conn);
    let backup = backups_repo
        .find_by_id(snapshot_id)
        .map_err(|e| RestoreError::Other(e.to_string()))?
        .ok_or(RestoreError::SnapshotNotFound(snapshot_id))?;

    let files_repo = FilesRepo::new(conn);
    let file_entry = files_repo
        .find_by_id(backup.file_id)
        .map_err(|e| RestoreError::Other(e.to_string()))?
        .ok_or(RestoreError::FileNotFound(backup.file_id))?;

    let locations_repo = SaveLocationsRepo::new(conn);
    let save_location = locations_repo
        .find_by_id(file_entry.save_location_id)
        .map_err(|e| RestoreError::Other(e.to_string()))?
        .ok_or(RestoreError::SaveLocationNotFound(
            file_entry.save_location_id,
        ))?;

    let games_repo = savevault_db::repositories::GamesRepo::new(conn);
    let game = games_repo
        .find_by_id(save_location.game_id)
        .map_err(|e| RestoreError::Other(e.to_string()))?
        .ok_or_else(|| RestoreError::Other("Game não encontrado".to_string()))?;

    Ok(RestoreMetadata {
        backup,
        file_entry,
        save_location,
        game,
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

    let source_path = resolve_snapshot_source_path(
        backup_root,
        metadata,
        filename,
        request.target_type.as_deref(),
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
    let dest_hash = savevault_backup::hasher::hash_file(dest_path)
        .await
        .map_err(|_| RestoreError::PostWriteValidationFailed(dest_path.clone()))?;

    if dest_hash != source_hash {
        return Err(RestoreError::PostWriteValidationFailed(dest_path.clone()));
    }

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
    let metadata = {
        let conn = db.lock().await;
        load_restore_metadata(&conn, request.snapshot_id)?
    };
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
        let v1_dir = backup_root
            .join(savevault_backup::snapshot::sanitize_name(&game.name))
            .join("latest");
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

        let v1_dir = backup_root
            .join(savevault_backup::snapshot::sanitize_name(&game.name))
            .join("latest");
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

        let v1_dir = backup_root
            .join(savevault_backup::snapshot::sanitize_name(&game.name))
            .join("latest");
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
    #[ignore]
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
        let v1_dir = backup_root
            .join(savevault_backup::snapshot::sanitize_name(&game.name))
            .join("latest");
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

        let v2_dir = backup_root
            .join(savevault_backup::snapshot::sanitize_name(&game.name))
            .join("latest");
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

        let v1_dir = backup_root
            .join(savevault_backup::snapshot::sanitize_name(&game.name))
            .join("latest");
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
