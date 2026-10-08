use anyhow::Result;
use chrono::Utc;
use savevault_core::models::{Backup, SyncStatus};
use std::path::{Path, PathBuf};
use tracing::{debug, info};
use uuid::Uuid;

/// Motor de snapshots/versões de saves.
#[allow(dead_code)]
pub struct SnapshotManager {
    /// Diretório raiz de backups locais.
    backup_root: PathBuf,
    /// Número máximo de versões a manter por arquivo.
    max_versions: u32,
}

impl SnapshotManager {
    pub fn new(backup_root: PathBuf, max_versions: u32) -> Self {
        Self {
            backup_root,
            max_versions,
        }
    }

    /// Cria um snapshot do arquivo de save, retornando o Backup criado.
    /// Preserva a estrutura relativa de diretórios caso `rel_path` seja fornecido.
    /// Nunca sobrescreve versões anteriores.
    pub async fn create_snapshot_with_relpath(
        &self,
        file_id: Uuid,
        game_name: &str,
        source_path: &Path,
        rel_path: Option<&Path>,
        next_version: u32,
    ) -> Result<Backup> {
        let relative = match rel_path {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from(
                source_path
                    .file_name()
                    .ok_or_else(|| anyhow::anyhow!("Caminho inválido: sem nome de arquivo"))?,
            ),
        };

        let safe_game_name = sanitize_name(game_name);

        let version_dir = self
            .backup_root
            .join(&safe_game_name)
            .join(format!("v{}", next_version));

        let latest_dir = self.backup_root.join(&safe_game_name).join("latest");
        let daily_dir = self.backup_root.join(&safe_game_name).join("daily");

        let dest = version_dir.join(&relative);
        let dest_latest = latest_dir.join(&relative);
        let dest_daily = daily_dir.join(&relative);

        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        if let Some(parent) = dest_latest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        if let Some(parent) = dest_daily.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::copy(source_path, &dest).await?;
        tokio::fs::copy(source_path, &dest_latest).await?;

        let mut update_daily = true;
        if let Ok(meta) = tokio::fs::metadata(&dest_daily).await {
            if let Ok(mod_time) = meta.modified() {
                let mod_dt: chrono::DateTime<chrono::Local> = mod_time.into();
                if mod_dt.date_naive() == chrono::Local::now().date_naive() {
                    update_daily = false;
                }
            }
        }
        if update_daily {
            let _ = tokio::fs::copy(source_path, &dest_daily).await;
        }

        info!(
            "Snapshot criado: {} → {}",
            source_path.display(),
            dest.display()
        );

        let backup = Backup {
            id: Uuid::new_v4(),
            file_id,
            version: next_version,
            cloud_path: None,
            drive_file_id: None,
            sync_status: SyncStatus::Pending,
            created_at: Utc::now(),
        };

        Ok(backup)
    }

    /// Cria um snapshot do arquivo de save com fallback para o nome base do arquivo.
    pub async fn create_snapshot(
        &self,
        file_id: Uuid,
        game_name: &str,
        source_path: &Path,
        next_version: u32,
    ) -> Result<Backup> {
        self.create_snapshot_with_relpath(file_id, game_name, source_path, None, next_version)
            .await
    }

    /// Restaura um backup para o caminho de destino com suporte a caminho relativo.
    pub async fn restore_snapshot_with_relpath(
        &self,
        file_id: Uuid,
        game_name: &str,
        version: u32,
        dest_path: &Path,
        rel_path: Option<&Path>,
    ) -> Result<Option<PathBuf>> {
        let filename = dest_path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Caminho de destino inválido: sem nome de arquivo"))?;

        let relative = match rel_path {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from(filename),
        };

        let dest_parent = dest_path.parent().unwrap_or_else(|| Path::new("."));
        if !dest_parent.exists() {
            tokio::fs::create_dir_all(dest_parent).await?;
        }

        let safe_game_name = sanitize_name(game_name);

        let explicit_version = self
            .backup_root
            .join(&safe_game_name)
            .join(format!("v{}", version))
            .join(&relative);

        let flat_version = self
            .backup_root
            .join(&safe_game_name)
            .join(format!("v{}", version))
            .join(filename);

        let legacy_version = self
            .backup_root
            .join(file_id.to_string())
            .join(format!("v{}", version))
            .join(&relative);

        let legacy_flat = self
            .backup_root
            .join(file_id.to_string())
            .join(format!("v{}", version))
            .join(filename);

        let latest_source = self
            .backup_root
            .join(&safe_game_name)
            .join("latest")
            .join(&relative);

        let flat_latest = self
            .backup_root
            .join(&safe_game_name)
            .join("latest")
            .join(filename);

        let daily_source = self
            .backup_root
            .join(&safe_game_name)
            .join("daily")
            .join(&relative);

        let flat_daily = self
            .backup_root
            .join(&safe_game_name)
            .join("daily")
            .join(filename);

        let source = if explicit_version.exists() {
            explicit_version
        } else if flat_version.exists() {
            flat_version
        } else if legacy_version.exists() {
            legacy_version
        } else if legacy_flat.exists() {
            legacy_flat
        } else if version == 1 && daily_source.exists() {
            daily_source
        } else if version == 1 && flat_daily.exists() {
            flat_daily
        } else if version == 1 && latest_source.exists() {
            latest_source
        } else if version == 1 && flat_latest.exists() {
            flat_latest
        } else {
            explicit_version
        };

        if !source.exists() {
            anyhow::bail!(
                "Versão {} do arquivo {} não encontrada em {}",
                version,
                file_id,
                source.display()
            );
        };

        if source.is_dir() {
            anyhow::bail!(
                "Restauração recursiva de diretórios não é suportada diretamente em restore_snapshot: {}",
                source.display()
            );
        }

        // Passo 1: backup de segurança com identificador único resistente a colisões
        let safety_path = if dest_path.exists() {
            let orig_name = filename.to_string_lossy();
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
                anyhow::anyhow!(
                    "Falha ao criar backup de segurança em '{}': {}",
                    candidate.display(),
                    e
                )
            })?;
            debug!("Backup de segurança criado: {}", candidate.display());
            Some(candidate)
        } else {
            None
        };

        // Passo 2: cópia atômica via arquivo temporário no mesmo diretório de destino
        let orig_name = filename.to_string_lossy();
        let tmp_path = dest_parent.join(format!(
            "{}.restore_tmp_{}",
            orig_name,
            Uuid::new_v4().simple()
        ));

        if let Err(e) = tokio::fs::copy(&source, &tmp_path).await {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            anyhow::bail!("Falha ao copiar versão {} para staging: {}", version, e);
        }

        // Move/renomeia atomicamente sobre o destino
        if let Err(e) = tokio::fs::rename(&tmp_path, dest_path).await {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            anyhow::bail!(
                "Falha ao aplicar restauração atômica em '{}': {}. Destino permanece intacto.",
                dest_path.display(),
                e
            );
        }

        info!(
            "Restauração concluída: {} → {}",
            source.display(),
            dest_path.display()
        );
        Ok(safety_path)
    }

    /// Restaura um backup para o caminho de destino (compatibilidade legada).
    pub async fn restore_snapshot(
        &self,
        file_id: Uuid,
        game_name: &str,
        version: u32,
        dest_path: &Path,
    ) -> Result<Option<PathBuf>> {
        self.restore_snapshot_with_relpath(file_id, game_name, version, dest_path, None)
            .await
    }

    /// Remove o arquivo físico versionado correspondente à versão, preservando outros arquivos do mesmo jogo.
    /// Remove diretórios pais caso fiquem vazios.
    pub async fn remove_snapshot_version_file(
        &self,
        file_id: Uuid,
        game_name: &str,
        rel_path: &Path,
        version: u32,
    ) -> Result<()> {
        let safe_game_name = sanitize_name(game_name);
        let version_dir = self
            .backup_root
            .join(&safe_game_name)
            .join(format!("v{}", version));

        let filename = rel_path.file_name().unwrap_or(rel_path.as_os_str());
        let targets = [version_dir.join(rel_path), version_dir.join(filename)];

        for target in targets {
            if target.exists() && target.is_file() {
                let _ = tokio::fs::remove_file(&target).await;
                debug!("Arquivo de snapshot físico removido: {}", target.display());
                // Tenta remover diretórios pais até version_dir se ficarem vazios
                let mut current = target.parent();
                while let Some(parent) = current {
                    if parent == self.backup_root || !parent.starts_with(&version_dir) {
                        break;
                    }
                    if tokio::fs::remove_dir(parent).await.is_err() {
                        break;
                    }
                    current = parent.parent();
                }
            }
        }

        let _ = tokio::fs::remove_dir(&version_dir).await;

        let legacy_dir = self
            .backup_root
            .join(file_id.to_string())
            .join(format!("v{}", version));

        if legacy_dir.exists() {
            let legacy_targets = [legacy_dir.join(rel_path), legacy_dir.join(filename)];
            for lt in legacy_targets {
                if lt.exists() && lt.is_file() {
                    let _ = tokio::fs::remove_file(&lt).await;
                }
            }
            let _ = tokio::fs::remove_dir(&legacy_dir).await;
        }

        Ok(())
    }

    /// Remove o diretório físico do snapshot versionado correspondente (compatibilidade legada).
    pub async fn remove_snapshot_version(
        &self,
        file_id: Uuid,
        game_name: &str,
        version: u32,
    ) -> Result<()> {
        let safe_game_name = sanitize_name(game_name);
        let version_dir = self
            .backup_root
            .join(&safe_game_name)
            .join(format!("v{}", version));

        if version_dir.exists() {
            let _ = tokio::fs::remove_dir_all(&version_dir).await;
            debug!(
                "Diretório de snapshot físico versionado removido: {}",
                version_dir.display()
            );
        }

        let legacy_dir = self
            .backup_root
            .join(file_id.to_string())
            .join(format!("v{}", version));

        if legacy_dir.exists() {
            let _ = tokio::fs::remove_dir_all(&legacy_dir).await;
            debug!(
                "Diretório de snapshot legado removido: {}",
                legacy_dir.display()
            );
        }

        Ok(())
    }
}

/// Sanitiza o nome do jogo para ser um nome de diretório válido em todos os sistemas operacionais.
pub fn sanitize_name(name: &str) -> String {
    name.replace(&['\\', '/', ':', '*', '?', '"', '<', '>', '|'][..], "_")
}
