use anyhow::Result;
use std::path::{Path, PathBuf};
use tracing::{debug, info};
use savevault_core::models::{Backup, SyncStatus};
use uuid::Uuid;
use chrono::Utc;

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
        Self { backup_root, max_versions }
    }

    /// Cria um snapshot do arquivo de save, retornando o Backup criado.
    /// Nunca sobrescreve versões anteriores.
    ///
    /// Estrutura: `backup_root/file_id/v{version}/filename`
    pub async fn create_snapshot(
        &self,
        file_id: Uuid,
        game_name: &str,
        source_path: &Path,
        next_version: u32,
    ) -> Result<Backup> {
        let filename = source_path.file_name()
            .ok_or_else(|| anyhow::anyhow!("Caminho inválido: sem nome de arquivo"))?;

        let safe_game_name = sanitize_name(game_name);

        let version_dir = self.backup_root
            .join(&safe_game_name)
            .join(format!("v{}", next_version));

        let latest_dir = self.backup_root
            .join(&safe_game_name)
            .join("latest");

        let daily_dir = self.backup_root
            .join(&safe_game_name)
            .join("daily");

        tokio::fs::create_dir_all(&version_dir).await?;
        tokio::fs::create_dir_all(&latest_dir).await?;
        tokio::fs::create_dir_all(&daily_dir).await?;

        let dest = version_dir.join(filename);
        tokio::fs::copy(source_path, &dest).await?;

        let dest_latest = latest_dir.join(filename);
        tokio::fs::copy(source_path, &dest_latest).await?;

        let dest_daily = daily_dir.join(filename);
        let mut update_daily = true;
        if dest_daily.exists() {
            if let Ok(meta) = tokio::fs::metadata(&dest_daily).await {
                if let Ok(mod_time) = meta.modified() {
                    let mod_dt: chrono::DateTime<chrono::Local> = mod_time.into();
                    if mod_dt.date_naive() == chrono::Local::now().date_naive() {
                        update_daily = false;
                    }
                }
            }
        }
        if update_daily {
            let _ = tokio::fs::copy(source_path, &dest_daily).await;
        }

        info!("Snapshot criado: {} → {}", source_path.display(), dest.display());

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

    /// Restaura um backup para o caminho de destino.
    /// Segurança:
    /// 1. Valida a existência da versão de origem ANTES de qualquer modificação no disco;
    /// 2. Cria backup de segurança único (sem nunca sobrescrever backups anteriores);
    /// 3. Realiza a restauração de forma atômica via arquivo temporário e rename;
    /// 4. Preserva o destino original em caso de qualquer falha na cópia.
    pub async fn restore_snapshot(
        &self,
        file_id: Uuid,
        game_name: &str,
        version: u32,
        dest_path: &Path,
    ) -> Result<Option<PathBuf>> {
        let filename = dest_path.file_name()
            .ok_or_else(|| anyhow::anyhow!("Caminho de destino inválido: sem nome de arquivo"))?;

        let dest_parent = dest_path.parent().unwrap_or_else(|| Path::new("."));
        if !dest_parent.exists() {
            tokio::fs::create_dir_all(dest_parent).await?;
        }

        let safe_game_name = sanitize_name(game_name);

        // Validação antecipada da versão de origem (nunca alterar o disco se a versão não existir)
        let version_path = self.backup_root
            .join(&safe_game_name)
            .join(format!("v{}", version))
            .join(filename);

        let latest_path = self.backup_root
            .join(&safe_game_name)
            .join("latest")
            .join(filename);

        let daily_path = self.backup_root
            .join(&safe_game_name)
            .join("daily")
            .join(filename);

        let source = if version_path.exists() {
            version_path
        } else if version == 1 && daily_path.exists() {
            daily_path
        } else if version == 1 && latest_path.exists() {
            latest_path
        } else {
            version_path
        };

        if !source.exists() {
            anyhow::bail!(
                "Versão {} do arquivo {} não encontrada em {}",
                version, file_id, source.display()
            );
        }

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
                    candidate.display(), e
                )
            })?;
            debug!("Backup de segurança criado: {}", candidate.display());
            Some(candidate)
        } else {
            None
        };

        // Passo 2: cópia atômica via arquivo temporário no mesmo diretório de destino
        let orig_name = filename.to_string_lossy();
        let tmp_path = dest_parent.join(format!("{}.restore_tmp_{}", orig_name, Uuid::new_v4().simple()));

        if let Err(e) = tokio::fs::copy(&source, &tmp_path).await {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            anyhow::bail!("Falha ao copiar versão {} para staging: {}", version, e);
        }

        // Move/renomeia atomicamente sobre o destino
        if let Err(e) = tokio::fs::rename(&tmp_path, dest_path).await {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            anyhow::bail!(
                "Falha ao aplicar restauração atômica em '{}': {}. Destino permanece intacto.",
                dest_path.display(), e
            );
        }

        info!("Restauração concluída: {} → {}", source.display(), dest_path.display());
        Ok(safety_path)
    }

    /// Remove o diretório físico do snapshot correspondente à versão.
    /// Ex.: `backup_root/file_id/v{version}`
    pub async fn remove_snapshot_version(&self, _file_id: Uuid, game_name: &str, version: u32) -> Result<()> {
        let safe_game_name = sanitize_name(game_name);
        let version_dir = self.backup_root
            .join(&safe_game_name)
            .join(format!("v{}", version));

        if version_dir.exists() {
            tokio::fs::remove_dir_all(&version_dir).await?;
        }
        Ok(())
    }
}

/// Sanitiza o nome do jogo para ser um nome de diretório válido em todos os sistemas operacionais.
pub fn sanitize_name(name: &str) -> String {
    name.replace(&['\\', '/', ':', '*', '?', '"', '<', '>', '|'][..], "_")
}
