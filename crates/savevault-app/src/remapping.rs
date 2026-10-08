//! Motor de Remapeamento Interativo de Saves Multiplataforma (AUD-03).
//!
//! Permite detectar, resolver automaticamente e remapear interativamente locais de saves
//! em máquinas onde o caminho original (ex.: usuário diferente ou outro sistema operacional)
//! não existe ou diverge da máquina de origem.
//!
//! Princípios de Segurança e Integridade:
//! 1. Todo caminho remoto/manifest é tratado como dado não-confiável.
//! 2. Proteção rigorosa contra path traversal (`..`), caracteres nulos e injeção de volume.
//! 3. Bloqueio de raízes de disco e diretórios de sistema protegidos.
//! 4. Detecção de conflitos no destino sem sobrescrita silenciosa.
//! 5. Idempotência: remapeamentos manuais do usuário são preservados entre sincronizações.

use anyhow::Result;
use rusqlite::Connection;
use savevault_core::models::{MappingSource, MappingStatus, SaveLocation, SaveLocationType};
use savevault_core::path::{
    extract_username_from_path, get_current_system_username, normalize_windows_path,
    replace_username_in_path, validate_remapping_path_security,
};
use savevault_db::repositories::{GamesRepo, SaveLocationsRepo};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::info;
use uuid::Uuid;

/// Visão de um `SaveLocation` que aguarda remapeamento ou auditoria de caminho.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingMappingView {
    pub location_id: Uuid,
    pub game_id: Uuid,
    pub game_name: String,
    pub original_path: PathBuf,
    pub current_path: PathBuf,
    pub location_type: SaveLocationType,
    pub mapping_status: MappingStatus,
    pub mapping_source: MappingSource,
    pub resolution_reason: Option<String>,
    pub suggested_path: Option<PathBuf>,
    pub has_conflicts: bool,
    pub existing_files_count: usize,
}

/// Resultado detalhado da validação de um caminho de remapeamento candidato.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub reason: Option<String>,
    pub normalized_path: Option<PathBuf>,
    pub has_conflicts: bool,
    pub existing_files_count: usize,
    pub existing_files: Vec<String>,
}

/// Valida um caminho candidato selecionado pelo usuário ou sugerido pelo sistema.
pub fn validate_candidate_path(candidate: &Path) -> ValidationResult {
    if let Err(e) = validate_remapping_path_security(candidate) {
        return ValidationResult {
            valid: false,
            reason: Some(e),
            normalized_path: None,
            has_conflicts: false,
            existing_files_count: 0,
            existing_files: Vec::new(),
        };
    }

    let norm_path = match normalize_windows_path(candidate) {
        Ok(p) => p,
        Err(e) => {
            return ValidationResult {
                valid: false,
                reason: Some(format!("Falha na normalização do caminho: {}", e)),
                normalized_path: None,
                has_conflicts: false,
                existing_files_count: 0,
                existing_files: Vec::new(),
            }
        }
    };

    let mut has_conflicts = false;
    let mut existing_files_count = 0;
    let mut existing_files = Vec::new();

    if norm_path.exists() {
        if norm_path.is_file() {
            has_conflicts = true;
            existing_files_count = 1;
            if let Some(name) = norm_path.file_name().and_then(|n| n.to_str()) {
                existing_files.push(name.to_string());
            }
        } else if norm_path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&norm_path) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_file() {
                        existing_files_count += 1;
                        if existing_files.len() < 5 {
                            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                                existing_files.push(name.to_string());
                            }
                        }
                    }
                }
            }
            if existing_files_count > 0 {
                has_conflicts = true;
            }
        }
    }

    ValidationResult {
        valid: true,
        reason: None,
        normalized_path: Some(norm_path),
        has_conflicts,
        existing_files_count,
        existing_files,
    }
}

/// Tenta resolver automaticamente o caminho do save na máquina atual com base em:
/// 1. Caminho original existente no disco;
/// 2. Tradução de username (`C:\Users\Alice\...` -> `C:\Users\Bob\...`);
/// 3. Conversão para pastas conhecidas do sistema operacional;
/// 4. Se não houver evidência segura, retorna status `NeedsUserMapping` com sugestão se disponível.
pub fn resolve_path_for_game(
    game_name: &str,
    original_path: &Path,
    override_user: Option<&str>,
) -> (
    PathBuf,
    MappingStatus,
    MappingSource,
    Option<String>,
    Option<PathBuf>,
) {
    // Caso A: caminho original continua válido e existe no disco
    if original_path.exists() {
        return (
            original_path.to_path_buf(),
            MappingStatus::Resolved,
            MappingSource::Original,
            None,
            None,
        );
    }

    // Identifica o usuário da máquina atual
    let target_user = override_user
        .map(|s| s.to_string())
        .or_else(get_current_system_username);

    // Caso B: username mudou (ex.: C:\Users\Alice\... -> C:\Users\Bob\...)
    if let (Some(orig_user), Some(ref curr_user)) = (
        extract_username_from_path(original_path),
        target_user.as_ref(),
    ) {
        if !orig_user.eq_ignore_ascii_case(curr_user) {
            if let Some(translated) = replace_username_in_path(original_path, curr_user) {
                // Se o diretório traduzido existe ou se a pasta mãe de usuário existe
                if translated.exists() {
                    info!(
                        "Auto-resolução segura de username para '{}': '{}' -> '{}'",
                        game_name,
                        original_path.display(),
                        translated.display()
                    );
                    return (
                        translated,
                        MappingStatus::AutoResolved,
                        MappingSource::Auto,
                        Some(format!(
                            "Caminho resolvido automaticamente para o usuário '{}'",
                            curr_user
                        )),
                        None,
                    );
                }

                // Se o diretório mãe (ex: Saved Games ou Documents) existe, temos evidência suficiente
                if let Some(parent) = translated.parent() {
                    if parent.exists() {
                        info!(
                            "Auto-resolução de subdiretório de usuário para '{}': '{}'",
                            game_name,
                            translated.display()
                        );
                        return (
                            translated.clone(),
                            MappingStatus::AutoResolved,
                            MappingSource::Auto,
                            Some(format!(
                                "Diretório padrão de usuário detectado para '{}'",
                                curr_user
                            )),
                            Some(translated),
                        );
                    }
                }

                // Caso não exista a pasta mãe, sugerimos o caminho ao usuário
                return (
                    translated.clone(),
                    MappingStatus::NeedsUserMapping,
                    MappingSource::Original,
                    Some(format!(
                        "Local de save original de '{}' não foi encontrado nesta máquina.",
                        orig_user
                    )),
                    Some(translated),
                );
            }
        }
    }

    // Caso C: Cross-Platform (Windows -> Linux ou Windows -> macOS)
    let orig_str = original_path.to_string_lossy();
    if cfg!(target_os = "linux") && (orig_str.contains('\\') || orig_str.contains(':')) {
        if let Ok(home) = std::env::var("HOME") {
            let safe_game_name = savevault_backup::snapshot::sanitize_name(game_name);
            let linux_candidate = PathBuf::from(&home)
                .join(".local")
                .join("share")
                .join(&safe_game_name);
            return (
                linux_candidate.clone(),
                MappingStatus::NeedsUserMapping,
                MappingSource::Original,
                Some("Caminho original do Windows convertido para padrão Linux.".into()),
                Some(linux_candidate),
            );
        }
    } else if cfg!(target_os = "macos") && (orig_str.contains('\\') || orig_str.contains(':')) {
        if let Ok(home) = std::env::var("HOME") {
            let safe_game_name = savevault_backup::snapshot::sanitize_name(game_name);
            let mac_candidate = PathBuf::from(&home)
                .join("Library")
                .join("Application Support")
                .join(&safe_game_name);
            return (
                mac_candidate.clone(),
                MappingStatus::NeedsUserMapping,
                MappingSource::Original,
                Some("Caminho original do Windows convertido para padrão macOS.".into()),
                Some(mac_candidate),
            );
        }
    }

    // Caso D: Não há evidência suficiente -> Requer mapeamento do usuário
    (
        original_path.to_path_buf(),
        MappingStatus::NeedsUserMapping,
        MappingSource::Original,
        Some("Local de save original não encontrado nesta máquina.".to_string()),
        None,
    )
}

/// Lista todas as save locations que necessitam de intervenção ou possuem status pendente.
pub fn list_pending_remappings(conn: &Connection) -> Result<Vec<PendingMappingView>> {
    let loc_repo = SaveLocationsRepo::new(conn);
    let games_repo = GamesRepo::new(conn);

    let pending_locs = loc_repo.list_pending_remappings()?;
    let mut views = Vec::new();

    for loc in pending_locs {
        let game_name = games_repo
            .find_by_id(loc.game_id)?
            .map(|g| g.name)
            .unwrap_or_else(|| "Desconhecido".to_string());

        let orig = loc
            .original_path
            .clone()
            .unwrap_or_else(|| loc.path.clone());

        let (_, _, _, _, suggested) = resolve_path_for_game(&game_name, &orig, None);

        let validation = validate_candidate_path(&loc.path);

        views.push(PendingMappingView {
            location_id: loc.id,
            game_id: loc.game_id,
            game_name,
            original_path: orig,
            current_path: loc.path,
            location_type: loc.location_type,
            mapping_status: loc.mapping_status,
            mapping_source: loc.mapping_source,
            resolution_reason: loc.resolution_reason,
            suggested_path: suggested,
            has_conflicts: validation.has_conflicts,
            existing_files_count: validation.existing_files_count,
        });
    }

    Ok(views)
}

/// Aplica o novo caminho escolhido pelo usuário para um `SaveLocation`,
/// validando segurança, criando a pasta se necessário e atualizando os registros de arquivo.
pub fn apply_remapping(
    conn: &Connection,
    location_id: Uuid,
    new_path: &Path,
) -> Result<SaveLocation, String> {
    let validation = validate_candidate_path(new_path);
    if !validation.valid {
        return Err(validation
            .reason
            .unwrap_or_else(|| "Caminho inválido para remapeamento".to_string()));
    }

    let norm_path = validation
        .normalized_path
        .ok_or_else(|| "Falha ao normalizar o caminho de destino".to_string())?;

    let loc_repo = SaveLocationsRepo::new(conn);
    let existing = loc_repo
        .find_by_id(location_id)
        .map_err(|e| format!("Erro ao consultar local: {}", e))?
        .ok_or_else(|| format!("SaveLocation '{}' não encontrado", location_id))?;

    // Garante que o diretório de destino existe fisicamente
    if existing.location_type == SaveLocationType::File {
        if let Some(parent) = norm_path.parent() {
            if !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return Err(format!(
                        "Falha ao criar diretório pai '{}': {}",
                        parent.display(),
                        e
                    ));
                }
            }
        }
    } else if !norm_path.exists() {
        if let Err(e) = std::fs::create_dir_all(&norm_path) {
            return Err(format!(
                "Falha ao criar diretório de destino '{}': {}",
                norm_path.display(),
                e
            ));
        }
    }

    let updated = loc_repo
        .update_mapping(
            location_id,
            &norm_path,
            MappingStatus::UserMapped,
            MappingSource::User,
            None,
        )
        .map_err(|e| format!("Falha ao persistir remapeamento no banco: {}", e))?;

    info!(
        "SaveLocation '{}' remapeado com sucesso para '{}'",
        location_id,
        norm_path.display()
    );

    Ok(updated)
}

/// Reseta o remapeamento de um `SaveLocation` para seu caminho original.
pub fn reset_remapping(conn: &Connection, location_id: Uuid) -> Result<SaveLocation, String> {
    let loc_repo = SaveLocationsRepo::new(conn);
    loc_repo
        .reset_mapping(location_id)
        .map_err(|e| format!("Falha ao resetar mapeamento: {}", e))
}
