use anyhow::Result;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/mtkennerly/ludusavi-manifest/master/data/manifest.yaml";

/// Número de dias antes de revalidar o manifest em cache.
const CACHE_MAX_AGE_DAYS: u64 = 7;

/// Retorna o caminho para o manifest, baixando se necessário.
pub async fn get_manifest_path(cache_dir: &Path) -> Result<PathBuf> {
    let manifest_path = cache_dir.join("manifest.yaml");

    if manifest_path.exists() {
        let age_secs = std::fs::metadata(&manifest_path)?
            .modified()
            .ok()
            .and_then(|m| std::time::SystemTime::now().duration_since(m).ok())
            .map(|d| d.as_secs())
            .unwrap_or(u64::MAX);

        if age_secs < CACHE_MAX_AGE_DAYS * 86_400 {
            info!(
                "Manifest em cache ({} dia(s) de idade). Usando versão local.",
                age_secs / 86_400
            );
            return Ok(manifest_path);
        }
        info!("Manifest em cache expirado. Atualizando...");
    } else {
        info!("Manifest não encontrado em cache. Baixando pela primeira vez...");
    }

    match download_manifest(&manifest_path).await {
        Ok(_) => {
            info!("Manifest baixado com sucesso.");
            Ok(manifest_path)
        }
        Err(e) => {
            warn!("Falha ao baixar manifest: {}.", e);
            if manifest_path.exists() {
                warn!("Usando versão em cache anterior.");
                Ok(manifest_path)
            } else {
                Err(e)
            }
        }
    }
}

async fn download_manifest(dest: &Path) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    info!("Conectando a {}...", MANIFEST_URL);
    let response = client.get(MANIFEST_URL).send().await?;
    let bytes = response.bytes().await?;

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(dest, &bytes)?;
    info!(
        "Manifest salvo ({:.1} MB).",
        bytes.len() as f64 / 1_048_576.0
    );
    Ok(())
}
