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
    /// Nunca sobrescreve versões anteriores.
    ///
    /// Estrutura: `backup_root/file_id/v{version}/filename`
    pub async fn create_snapshot(
        &self,
        file_id: Uuid,
        source_path: &Path,
        next_version: u32,
    ) -> Result<Backup> {
        let filename = source_path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Caminho inválido: sem nome de arquivo"))?;

        let version_dir = self
            .backup_root
            .join(file_id.to_string())
            .join(format!("v{}", next_version));

        tokio::fs::create_dir_all(&version_dir).await?;

        let dest = version_dir.join(filename);
        tokio::fs::copy(source_path, &dest).await?;

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

    /// Restaura um backup para o caminho de destino (utilitário de baixo nível do filesystem).
    ///
    /// # OBSERVAÇÃO ARQUITETURAL:
    /// O motor oficial, seguro e transacional de restauração de produção é implementado
    /// em `savevault_app::restore` (`restore_snapshot_with_db` e `restore_by_id`), que
    /// garante integridade BLAKE3, validação contra path traversal, rollback de metadados
    /// e rastreabilidade total no SQLite.
    #[deprecated(
        since = "0.1.0",
        note = "Utilize o motor oficial e completo de restauração em `savevault_app::restore`, integrado ao banco de dados SQLite e com validação de hash BLAKE3."
    )]
    pub async fn restore_snapshot(
        &self,
        file_id: Uuid,
        version: u32,
        dest_path: &Path,
    ) -> Result<Option<PathBuf>> {
        let filename = dest_path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Caminho de destino inválido: sem nome de arquivo"))?;

        let dest_parent = dest_path.parent().unwrap_or_else(|| Path::new("."));
        if !dest_parent.exists() {
            tokio::fs::create_dir_all(dest_parent).await?;
        }

        // Validação antecipada da versão de origem (nunca alterar o disco se a versão não existir)
        let version_dir = self
            .backup_root
            .join(file_id.to_string())
            .join(format!("v{}", version));

        let source = version_dir.join(filename);

        if !source.exists() {
            anyhow::bail!(
                "Versão {} do arquivo {} não encontrada em {}",
                version,
                file_id,
                version_dir.display()
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
}
