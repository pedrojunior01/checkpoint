use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use savevault_core::models::{MappingSource, MappingStatus, SaveLocation, SaveLocationType};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub struct SaveLocationsRepo<'a> {
    pub conn: &'a Connection,
}

impl<'a> SaveLocationsRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn insert(&self, loc: &SaveLocation) -> Result<()> {
        let norm_path = savevault_core::path::normalize_windows_path(&loc.path)
            .map_err(|e| anyhow::anyhow!("Caminho de save inválido: {}", e))?;
        let path_str = norm_path
            .to_str()
            .context("Caminho de save contém caracteres não-UTF8")?;

        let orig_path_str = loc
            .original_path
            .as_ref()
            .and_then(|p| p.to_str())
            .unwrap_or(path_str);

        self.conn.execute(
            "INSERT INTO save_locations (id, game_id, path, location_type, original_path, mapping_status, mapping_source, resolution_reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                loc.id.to_string(),
                loc.game_id.to_string(),
                path_str,
                location_type_to_str(&loc.location_type),
                orig_path_str,
                mapping_status_to_str(loc.mapping_status),
                mapping_source_to_str(loc.mapping_source),
                loc.resolution_reason,
            ],
        )?;
        Ok(())
    }

    /// Insere ou atualiza um `SaveLocation` de forma estritamente idempotente.
    ///
    /// Se um registro para `(game_id, path COLLATE NOCASE)` já existir:
    /// - Atualiza campos não-identitários (`location_type`);
    /// - PRESERVA o ID (UUID) original do registro no banco (não gera novo UUID silenciosamente);
    /// - Preserva o mapeamento manual do usuário caso já esteja em `UserMapped`;
    /// - Retorna o `SaveLocation` persistido refletindo o estado final no banco.
    pub fn upsert(&self, loc: &SaveLocation) -> Result<SaveLocation> {
        let norm_path = savevault_core::path::normalize_windows_path(&loc.path)
            .map_err(|e| anyhow::anyhow!("Caminho de save inválido: {}", e))?;
        let path_str = norm_path
            .to_str()
            .context("Caminho de save contém caracteres não-UTF8")?;

        let orig_path_str = loc
            .original_path
            .as_ref()
            .and_then(|p| p.to_str())
            .unwrap_or(path_str);

        let mut stmt = self.conn.prepare(
            "INSERT INTO save_locations (id, game_id, path, location_type, original_path, mapping_status, mapping_source, resolution_reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(game_id, path COLLATE NOCASE) DO UPDATE SET
                 location_type = excluded.location_type,
                 original_path = COALESCE(save_locations.original_path, excluded.original_path),
                 mapping_status = CASE
                     WHEN save_locations.mapping_status = 'user_mapped' THEN save_locations.mapping_status
                     ELSE excluded.mapping_status
                 END,
                 mapping_source = CASE
                     WHEN save_locations.mapping_status = 'user_mapped' THEN save_locations.mapping_source
                     ELSE excluded.mapping_source
                 END,
                 resolution_reason = CASE
                     WHEN save_locations.mapping_status = 'user_mapped' THEN save_locations.resolution_reason
                     ELSE excluded.resolution_reason
                 END
             RETURNING id, game_id, path, location_type, original_path, mapping_status, mapping_source, resolution_reason",
        )?;

        let saved = stmt.query_row(
            params![
                loc.id.to_string(),
                loc.game_id.to_string(),
                path_str,
                location_type_to_str(&loc.location_type),
                orig_path_str,
                mapping_status_to_str(loc.mapping_status),
                mapping_source_to_str(loc.mapping_source),
                loc.resolution_reason,
            ],
            row_to_location,
        )?;

        Ok(saved)
    }

    pub fn find_by_id(&self, id: Uuid) -> Result<Option<SaveLocation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, game_id, path, location_type, original_path, mapping_status, mapping_source, resolution_reason
             FROM save_locations WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id.to_string()])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_location(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn find_by_game_and_path(
        &self,
        game_id: Uuid,
        path: &Path,
    ) -> Result<Option<SaveLocation>> {
        let norm_path = savevault_core::path::normalize_windows_path(path)
            .map_err(|e| anyhow::anyhow!("Caminho de save inválido: {}", e))?;
        let path_str = norm_path
            .to_str()
            .context("Caminho de save contém caracteres não-UTF8")?;
        let mut stmt = self.conn.prepare(
            "SELECT id, game_id, path, location_type, original_path, mapping_status, mapping_source, resolution_reason
             FROM save_locations WHERE game_id = ?1 AND path = ?2 COLLATE NOCASE",
        )?;
        let mut rows = stmt.query(params![game_id.to_string(), path_str])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_location(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn list_by_game(&self, game_id: Uuid) -> Result<Vec<SaveLocation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, game_id, path, location_type, original_path, mapping_status, mapping_source, resolution_reason
             FROM save_locations WHERE game_id = ?1",
        )?;
        let mut rows = stmt.query(params![game_id.to_string()])?;
        let mut locs = Vec::new();
        while let Some(row) = rows.next()? {
            locs.push(row_to_location(row)?);
        }
        Ok(locs)
    }

    pub fn list_all(&self) -> Result<Vec<SaveLocation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, game_id, path, location_type, original_path, mapping_status, mapping_source, resolution_reason
             FROM save_locations",
        )?;
        let mut rows = stmt.query([])?;
        let mut locs = Vec::new();
        while let Some(row) = rows.next()? {
            locs.push(row_to_location(row)?);
        }
        Ok(locs)
    }

    /// Lista todos os `SaveLocation` que necessitam de intervenção ou possuem mapeamento pendente.
    pub fn list_pending_remappings(&self) -> Result<Vec<SaveLocation>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, game_id, path, location_type, original_path, mapping_status, mapping_source, resolution_reason
             FROM save_locations
             WHERE mapping_status IN ('needs_user_mapping', 'invalid')",
        )?;
        let mut rows = stmt.query([])?;
        let mut locs = Vec::new();
        while let Some(row) = rows.next()? {
            locs.push(row_to_location(row)?);
        }
        Ok(locs)
    }

    /// Atualiza o caminho de um `SaveLocation` e sincroniza todos os registros em `files` dependentes.
    pub fn update_mapping(
        &self,
        id: Uuid,
        new_path: &Path,
        status: MappingStatus,
        source: MappingSource,
        reason: Option<&str>,
    ) -> Result<SaveLocation> {
        let existing = self
            .find_by_id(id)?
            .ok_or_else(|| anyhow::anyhow!("SaveLocation '{}' não encontrado", id))?;

        let old_path = existing.path.clone();

        let norm_new_path = savevault_core::path::normalize_windows_path(new_path)
            .map_err(|e| anyhow::anyhow!("Novo caminho inválido: {}", e))?;
        let new_path_str = norm_new_path
            .to_str()
            .context("Caminho contém caracteres não-UTF8")?;

        let orig_path = existing
            .original_path
            .clone()
            .unwrap_or_else(|| old_path.clone());
        let orig_path_str = orig_path.to_string_lossy();

        self.conn.execute_batch("SAVEPOINT update_mapping;")?;

        let res = (|| -> Result<SaveLocation> {
            self.conn.execute(
                "UPDATE save_locations
                 SET path = ?1, original_path = ?2, mapping_status = ?3, mapping_source = ?4, resolution_reason = ?5
                 WHERE id = ?6",
                params![
                    new_path_str,
                    orig_path_str.as_ref(),
                    mapping_status_to_str(status),
                    mapping_source_to_str(source),
                    reason,
                    id.to_string(),
                ],
            )?;

            // Atualiza os registros correspondentes na tabela `files` para refletir o novo caminho base
            let mut select_stmt = self
                .conn
                .prepare("SELECT id, path FROM files WHERE save_location_id = ?1")?;
            let file_rows = select_stmt.query_map(params![id.to_string()], |row| {
                let fid: String = row.get(0)?;
                let fpath: String = row.get(1)?;
                Ok((fid, fpath))
            })?;

            let files: Vec<(String, String)> = file_rows.filter_map(|r| r.ok()).collect();
            for (fid, fpath_str) in files {
                let fpath = PathBuf::from(&fpath_str);
                let rel = savevault_core::path::compute_relative_path(&fpath, &old_path);
                let updated_fpath = norm_new_path.join(&rel);
                let updated_str = updated_fpath.to_string_lossy();
                self.conn.execute(
                    "UPDATE files SET path = ?1 WHERE id = ?2",
                    params![updated_str.as_ref(), fid],
                )?;
            }

            self.find_by_id(id)?
                .ok_or_else(|| anyhow::anyhow!("SaveLocation remapeado não pôde ser recuperado"))
        })();

        match res {
            Ok(loc) => {
                let _ = self.conn.execute_batch("RELEASE update_mapping;");
                Ok(loc)
            }
            Err(e) => {
                let _ = self
                    .conn
                    .execute_batch("ROLLBACK TO update_mapping; RELEASE update_mapping;");
                Err(e)
            }
        }
    }

    /// Reseta um `SaveLocation` para seu caminho original.
    pub fn reset_mapping(&self, id: Uuid) -> Result<SaveLocation> {
        let existing = self
            .find_by_id(id)?
            .ok_or_else(|| anyhow::anyhow!("SaveLocation '{}' não encontrado", id))?;

        let orig = existing
            .original_path
            .clone()
            .unwrap_or_else(|| existing.path.clone());

        let (status, reason) = if orig.exists() {
            (MappingStatus::Resolved, None)
        } else {
            (
                MappingStatus::NeedsUserMapping,
                Some("Local original não encontrado nesta máquina".to_string()),
            )
        };

        self.update_mapping(
            id,
            &orig,
            status,
            MappingSource::Original,
            reason.as_deref(),
        )
    }

    pub fn delete(&self, id: Uuid) -> Result<()> {
        self.conn.execute(
            "DELETE FROM save_locations WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }
}

fn row_to_location(row: &rusqlite::Row) -> rusqlite::Result<SaveLocation> {
    let id_str: String = row.get(0)?;
    let game_id_str: String = row.get(1)?;
    let path_str: String = row.get(2)?;
    let loc_type: String = row.get(3)?;
    let orig_path_str: Option<String> = row.get(4).unwrap_or(None);
    let mapping_status_str: Option<String> = row.get(5).unwrap_or(None);
    let mapping_source_str: Option<String> = row.get(6).unwrap_or(None);
    let resolution_reason: Option<String> = row.get(7).unwrap_or(None);

    let id = Uuid::parse_str(&id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let game_id = Uuid::parse_str(&game_id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
    })?;

    let path = PathBuf::from(&path_str);
    let original_path = orig_path_str
        .map(PathBuf::from)
        .or_else(|| Some(path.clone()));
    let mapping_status = mapping_status_str
        .as_deref()
        .map(str_to_mapping_status)
        .unwrap_or(MappingStatus::Resolved);
    let mapping_source = mapping_source_str
        .as_deref()
        .map(str_to_mapping_source)
        .unwrap_or(MappingSource::Original);

    Ok(SaveLocation {
        id,
        game_id,
        path,
        location_type: str_to_location_type(&loc_type),
        original_path,
        mapping_status,
        mapping_source,
        resolution_reason,
    })
}

fn mapping_status_to_str(s: MappingStatus) -> &'static str {
    match s {
        MappingStatus::Resolved => "resolved",
        MappingStatus::AutoResolved => "auto_resolved",
        MappingStatus::UserMapped => "user_mapped",
        MappingStatus::NeedsUserMapping => "needs_user_mapping",
        MappingStatus::Invalid => "invalid",
    }
}

fn str_to_mapping_status(s: &str) -> MappingStatus {
    match s {
        "auto_resolved" => MappingStatus::AutoResolved,
        "user_mapped" => MappingStatus::UserMapped,
        "needs_user_mapping" => MappingStatus::NeedsUserMapping,
        "invalid" => MappingStatus::Invalid,
        _ => MappingStatus::Resolved,
    }
}

fn mapping_source_to_str(s: MappingSource) -> &'static str {
    match s {
        MappingSource::Original => "original",
        MappingSource::Auto => "auto",
        MappingSource::Discovery => "discovery",
        MappingSource::User => "user",
    }
}

fn str_to_mapping_source(s: &str) -> MappingSource {
    match s {
        "auto" => MappingSource::Auto,
        "discovery" => MappingSource::Discovery,
        "user" => MappingSource::User,
        _ => MappingSource::Original,
    }
}

fn location_type_to_str(t: &SaveLocationType) -> &'static str {
    match t {
        SaveLocationType::File => "file",
        SaveLocationType::Directory => "directory",
        SaveLocationType::Registry => "registry",
        SaveLocationType::Glob => "glob",
    }
}

fn str_to_location_type(s: &str) -> SaveLocationType {
    match s {
        "file" => SaveLocationType::File,
        "registry" => SaveLocationType::Registry,
        "glob" => SaveLocationType::Glob,
        _ => SaveLocationType::Directory,
    }
}
