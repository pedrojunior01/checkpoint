use anyhow::Result;
use std::path::Path;

/// Calcula o hash BLAKE3 de um arquivo e retorna como string hexadecimal.
pub async fn hash_file(path: &Path) -> Result<String> {
    let content = tokio::fs::read(path).await?;
    let hash = blake3::hash(&content);
    Ok(hash.to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    // ------------------------------------------------------------------
    // Hasher tests
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn hash_is_deterministic() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp.as_file(), "save data").unwrap();
        let h1 = hash_file(tmp.path()).await.unwrap();
        let h2 = hash_file(tmp.path()).await.unwrap();
        assert_eq!(h1, h2, "Hash deve ser idêntico para o mesmo conteúdo");
    }

    #[tokio::test]
    async fn hash_changes_on_content_change() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp.as_file(), "versão 1").unwrap();
        let h1 = hash_file(tmp.path()).await.unwrap();

        let mut f = std::fs::OpenOptions::new().write(true).truncate(true).open(tmp.path()).unwrap();
        write!(f, "versão 2").unwrap();
        let h2 = hash_file(tmp.path()).await.unwrap();

        assert_ne!(h1, h2, "Hash deve mudar quando conteúdo muda");
    }

    #[tokio::test]
    async fn hash_empty_file_is_valid() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        // Arquivo criado vazio
        let hash = hash_file(tmp.path()).await.unwrap();
        assert!(!hash.is_empty(), "Hash de arquivo vazio não deve ser string vazia");
        assert_eq!(hash.len(), 64, "BLAKE3 hex deve ter 64 caracteres");
    }

    #[tokio::test]
    async fn hash_output_is_hex_string() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp.as_file(), "test").unwrap();
        let hash = hash_file(tmp.path()).await.unwrap();
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()), "Hash deve ser hex válido");
    }

    #[tokio::test]
    async fn hash_different_content_produces_different_hash() {
        let tmp1 = tempfile::NamedTempFile::new().unwrap();
        let tmp2 = tempfile::NamedTempFile::new().unwrap();
        write!(tmp1.as_file(), "conteúdo A").unwrap();
        write!(tmp2.as_file(), "conteúdo B").unwrap();

        let h1 = hash_file(tmp1.path()).await.unwrap();
        let h2 = hash_file(tmp2.path()).await.unwrap();
        assert_ne!(h1, h2);
    }

    // ------------------------------------------------------------------
    // SnapshotManager tests
    // ------------------------------------------------------------------
    #[tokio::test]
    async fn snapshot_creates_versioned_copy() {
        let dir = tempdir().unwrap();
        let backup_root = dir.path().join("backups");
        let manager = crate::SnapshotManager::new(backup_root.clone(), 10);

        // Cria arquivo de save temporário
        let save_dir = dir.path().join("saves");
        std::fs::create_dir_all(&save_dir).unwrap();
        let save_file = save_dir.join("save.dat");
        std::fs::write(&save_file, "game save data v1").unwrap();

        let file_id = uuid::Uuid::new_v4();
        let backup = manager.create_snapshot(file_id, &save_file, 1).await.unwrap();

        assert_eq!(backup.version, 1);
        assert_eq!(backup.file_id, file_id);

        // Verificar que o arquivo foi copiado
        let expected_path = backup_root.join(file_id.to_string()).join("v1").join("save.dat");
        assert!(expected_path.exists(), "Arquivo de backup deve existir em {}", expected_path.display());

        let content = std::fs::read_to_string(&expected_path).unwrap();
        assert_eq!(content, "game save data v1");
    }

    #[tokio::test]
    async fn snapshot_multiple_versions_coexist() {
        let dir = tempdir().unwrap();
        let backup_root = dir.path().join("backups");
        let manager = crate::SnapshotManager::new(backup_root.clone(), 10);

        let save_dir = dir.path().join("saves");
        std::fs::create_dir_all(&save_dir).unwrap();
        let save_file = save_dir.join("progress.sav");

        for version in 1..=3u32 {
            std::fs::write(&save_file, format!("data v{}", version)).unwrap();
            manager.create_snapshot(uuid::Uuid::new_v4(), &save_file, version).await.unwrap();
        }

        // Cada versão vai para um uuid diferente (pois usamos file_id diferente no loop)
        // Vamos usar o mesmo file_id para testar coexistência real
        let file_id = uuid::Uuid::new_v4();
        for version in 1..=3u32 {
            std::fs::write(&save_file, format!("shared data v{}", version)).unwrap();
            manager.create_snapshot(file_id, &save_file, version).await.unwrap();
        }

        for version in 1..=3u32 {
            let path = backup_root.join(file_id.to_string()).join(format!("v{}", version)).join("progress.sav");
            assert!(path.exists(), "Versão {} deve existir", version);
            let content = std::fs::read_to_string(&path).unwrap();
            assert_eq!(content, format!("shared data v{}", version));
        }
    }

    #[tokio::test]
    async fn snapshot_restore_with_safety_backup() {
        let dir = tempdir().unwrap();
        let backup_root = dir.path().join("backups");
        let manager = crate::SnapshotManager::new(backup_root, 10);

        let save_dir = dir.path().join("saves");
        std::fs::create_dir_all(&save_dir).unwrap();
        let save_file = save_dir.join("save.dat");

        // Versão 1 — cria snapshot
        std::fs::write(&save_file, "versão 1 do save").unwrap();
        let file_id = uuid::Uuid::new_v4();
        manager.create_snapshot(file_id, &save_file, 1).await.unwrap();

        // Simula progresso posterior (versão atual)
        std::fs::write(&save_file, "versão atual (pode estar corrompida)").unwrap();

        // Restaura a versão 1
        manager.restore_snapshot(file_id, 1, &save_file).await.unwrap();

        // Verifica que o save foi restaurado
        let restored = std::fs::read_to_string(&save_file).unwrap();
        assert_eq!(restored, "versão 1 do save");

        // Verifica que o backup de segurança foi criado
        let safety_bak = save_file.with_extension("savevault_safety_bak");
        assert!(safety_bak.exists(), "Backup de segurança deve ser criado antes da restauração");
        let safety_content = std::fs::read_to_string(&safety_bak).unwrap();
        assert_eq!(safety_content, "versão atual (pode estar corrompida)");
    }
}
