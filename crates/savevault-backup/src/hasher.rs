use anyhow::Result;
use std::path::Path;
use tokio::io::AsyncReadExt;

/// Calcula o hash BLAKE3 de um arquivo em streaming com buffer fixo de 64KB,
/// garantindo consumo de memória O(1) mesmo para arquivos de múltiplos gigabytes.
pub async fn hash_file(path: &Path) -> Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 65536];

    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
#[allow(deprecated)]
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
        let tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp.as_file(), "versão 1").unwrap();
        let h1 = hash_file(tmp.path()).await.unwrap();

        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(tmp.path())
            .unwrap();
        write!(f, "versão 2").unwrap();
        let h2 = hash_file(tmp.path()).await.unwrap();

        assert_ne!(h1, h2, "Hash deve mudar quando conteúdo muda");
    }

    #[tokio::test]
    async fn hash_empty_file_is_valid() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        // Arquivo criado vazio
        let hash = hash_file(tmp.path()).await.unwrap();
        assert!(
            !hash.is_empty(),
            "Hash de arquivo vazio não deve ser string vazia"
        );
        assert_eq!(hash.len(), 64, "BLAKE3 hex deve ter 64 caracteres");
    }

    #[tokio::test]
    async fn hash_output_is_hex_string() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp.as_file(), "test").unwrap();
        let hash = hash_file(tmp.path()).await.unwrap();
        assert!(
            hash.chars().all(|c| c.is_ascii_hexdigit()),
            "Hash deve ser hex válido"
        );
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

    #[tokio::test]
    async fn hash_streaming_matches_direct_hash_for_large_file() {
        // Gera arquivo de 200KB (maior que buffer de 64KB)
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut data = Vec::with_capacity(200 * 1024);
        for i in 0..(200 * 1024) {
            data.push((i % 251) as u8);
        }
        std::fs::write(tmp.path(), &data).unwrap();

        let streaming_hash = hash_file(tmp.path()).await.unwrap();
        let direct_hash = blake3::hash(&data).to_hex().to_string();

        assert_eq!(
            streaming_hash, direct_hash,
            "Hash em streaming (64KB chunks) deve ser idêntico ao hash direto"
        );
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
        let backup = manager
            .create_snapshot(file_id, &save_file, 1)
            .await
            .unwrap();

        assert_eq!(backup.version, 1);
        assert_eq!(backup.file_id, file_id);

        // Verificar que o arquivo foi copiado
        let expected_path = backup_root
            .join(file_id.to_string())
            .join("v1")
            .join("save.dat");
        assert!(
            expected_path.exists(),
            "Arquivo de backup deve existir em {}",
            expected_path.display()
        );

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

        let file_id = uuid::Uuid::new_v4();
        for version in 1..=3u32 {
            std::fs::write(&save_file, format!("shared data v{}", version)).unwrap();
            manager
                .create_snapshot(file_id, &save_file, version)
                .await
                .unwrap();
        }

        for version in 1..=3u32 {
            let path = backup_root
                .join(file_id.to_string())
                .join(format!("v{}", version))
                .join("progress.sav");
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
        manager
            .create_snapshot(file_id, &save_file, 1)
            .await
            .unwrap();

        // Simula progresso posterior (versão atual)
        std::fs::write(&save_file, "versão atual (pode estar corrompida)").unwrap();

        // Restaura a versão 1
        let safety_path = manager
            .restore_snapshot(file_id, 1, &save_file)
            .await
            .unwrap();

        // Verifica que o save foi restaurado
        let restored = std::fs::read_to_string(&save_file).unwrap();
        assert_eq!(restored, "versão 1 do save");

        // Verifica que o backup de segurança foi criado de forma identificável
        assert!(
            safety_path.is_some(),
            "Caminho do safety backup deve ser retornado"
        );
        let safety_path = safety_path.unwrap();
        assert!(
            safety_path.exists(),
            "Backup de segurança deve existir no disco"
        );
        let safety_content = std::fs::read_to_string(&safety_path).unwrap();
        assert_eq!(safety_content, "versão atual (pode estar corrompida)");
    }

    #[tokio::test]
    async fn snapshot_restore_consecutive_restores_preserve_all_safety_backups() {
        let dir = tempdir().unwrap();
        let backup_root = dir.path().join("backups");
        let manager = crate::SnapshotManager::new(backup_root, 10);

        let save_dir = dir.path().join("saves");
        std::fs::create_dir_all(&save_dir).unwrap();
        let save_file = save_dir.join("save.dat");
        let file_id = uuid::Uuid::new_v4();

        // Cria snapshots das versões 1, 2 e 3
        std::fs::write(&save_file, "conteúdo da versão 1").unwrap();
        manager
            .create_snapshot(file_id, &save_file, 1)
            .await
            .unwrap();

        std::fs::write(&save_file, "conteúdo da versão 2").unwrap();
        manager
            .create_snapshot(file_id, &save_file, 2)
            .await
            .unwrap();

        std::fs::write(&save_file, "conteúdo da versão 3").unwrap();
        manager
            .create_snapshot(file_id, &save_file, 3)
            .await
            .unwrap();

        // Estado inicial do save no disco antes dos restores (Save Original)
        std::fs::write(&save_file, "save original ativo").unwrap();

        // 1. Restore A (versão 1)
        let safety_a = manager
            .restore_snapshot(file_id, 1, &save_file)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&save_file).unwrap(),
            "conteúdo da versão 1"
        );

        // 2. Restore B (versão 2)
        let safety_b = manager
            .restore_snapshot(file_id, 2, &save_file)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&save_file).unwrap(),
            "conteúdo da versão 2"
        );

        // 3. Restore C (versão 3)
        let safety_c = manager
            .restore_snapshot(file_id, 3, &save_file)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&save_file).unwrap(),
            "conteúdo da versão 3"
        );

        // PROVA CRÍTICA: Os três safety backups devem existir simultaneamente e ser distintos!
        assert_ne!(
            safety_a, safety_b,
            "Safety backups A e B devem ter caminhos distintos"
        );
        assert_ne!(
            safety_b, safety_c,
            "Safety backups B e C devem ter caminhos distintos"
        );
        assert_ne!(
            safety_a, safety_c,
            "Safety backups A e C devem ter caminhos distintos"
        );

        assert!(
            safety_a.exists(),
            "Safety backup A deve continuar existindo após múltiplos restores"
        );
        assert!(
            safety_b.exists(),
            "Safety backup B deve continuar existindo após múltiplos restores"
        );
        assert!(
            safety_c.exists(),
            "Safety backup C deve continuar existindo após múltiplos restores"
        );

        // Conteúdos preservados
        assert_eq!(
            std::fs::read_to_string(&safety_a).unwrap(),
            "save original ativo"
        );
        assert_eq!(
            std::fs::read_to_string(&safety_b).unwrap(),
            "conteúdo da versão 1"
        );
        assert_eq!(
            std::fs::read_to_string(&safety_c).unwrap(),
            "conteúdo da versão 2"
        );
    }

    #[tokio::test]
    async fn snapshot_restore_failure_preserves_destination_untouched() {
        let dir = tempdir().unwrap();
        let backup_root = dir.path().join("backups");
        let manager = crate::SnapshotManager::new(backup_root, 10);

        let save_dir = dir.path().join("saves");
        std::fs::create_dir_all(&save_dir).unwrap();
        let save_file = save_dir.join("critical_save.dat");
        std::fs::write(&save_file, "dados intactos essenciais").unwrap();

        let file_id = uuid::Uuid::new_v4();

        // Tentar restaurar versão 999 que não existe
        let result = manager.restore_snapshot(file_id, 999, &save_file).await;
        assert!(
            result.is_err(),
            "Restauração de versão inexistente deve retornar erro"
        );

        // Destino deve permanecer completamente intacto
        assert_eq!(
            std::fs::read_to_string(&save_file).unwrap(),
            "dados intactos essenciais",
            "O arquivo de destino não deve ser modificado em caso de falha de restore"
        );

        // Nenhum safety backup espúrio deve ter sido criado
        let entries: Vec<_> = std::fs::read_dir(&save_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|name| name.contains("savevault_safety"))
            .collect();
        assert!(
            entries.is_empty(),
            "Nenhum safety backup deve ser gerado se a versão não existe: {:?}",
            entries
        );
    }
}
