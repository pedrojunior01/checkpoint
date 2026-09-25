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
        source_path: &Path,
        next_version: u32,
    ) -> Result<Backup> {
        let filename = source_path.file_name()
            .ok_or_else(|| anyhow::anyhow!("Caminho inválido: sem nome de arquivo"))?;

        let version_dir = self.backup_root
            .join(file_id.to_string())
            .join(format!("v{}", next_version));

        tokio::fs::create_dir_all(&version_dir).await?;

        let dest = version_dir.join(filename);
        tokio::fs::copy(source_path, &dest).await?;

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
    /// Segurança: antes de restaurar, faz backup temporário do arquivo atual.
    pub async fn restore_snapshot(
        &self,
        file_id: Uuid,
        version: u32,
        dest_path: &Path,
    ) -> Result<()> {
        // Passo 1: backup de segurança do arquivo atual
        if dest_path.exists() {
            let safety_path = dest_path.with_extension("savevault_safety_bak");
            tokio::fs::copy(dest_path, &safety_path).await?;
            debug!("Backup de segurança criado: {}", safety_path.display());
        }

        // Passo 2: encontrar o arquivo da versão solicitada
        let version_dir = self.backup_root
            .join(file_id.to_string())
            .join(format!("v{}", version));

        let filename = dest_path.file_name()
            .ok_or_else(|| anyhow::anyhow!("Caminho de destino inválido"))?;

        let source = version_dir.join(filename);

        if !source.exists() {
            anyhow::bail!("Versão {} do arquivo {} não encontrada em {}", version, file_id, version_dir.display());
        }

        tokio::fs::copy(&source, dest_path).await?;
        info!("Restauração concluída: {} → {}", source.display(), dest_path.display());

        Ok(())
    }
}
