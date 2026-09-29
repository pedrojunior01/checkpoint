use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::ludusavi::LudusaviManifest;

pub const MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/mtkennerly/ludusavi-manifest/master/data/manifest.yaml";

/// Número de dias antes de revalidar o manifest em cache.
pub const CACHE_MAX_AGE_DAYS: u64 = 7;

/// Valida o conteúdo bruto recebido do manifesto.
///
/// Exige:
/// 1. Bytes não-vazios e UTF-8 válido.
/// 2. Parsing sintático válido do formato YAML.
/// 3. Estrutura de manifesto não-vazia contendo pelo menos um jogo com entradas estruturadas
///    (ex.: arquivos de save, chave de registro ou referências a lojas como Steam/GOG).
///    Isso previne que respostas como `{"message": "Rate limit"}` ou `{"error": "..."}`
///    sejam promovidas a cache.
pub fn validate_manifest_content(bytes: &[u8]) -> Result<LudusaviManifest> {
    if bytes.is_empty() {
        bail!("Resposta do manifesto está vazia (0 bytes)");
    }

    let text =
        std::str::from_utf8(bytes).context("Conteúdo do manifesto não é texto UTF-8 válido")?;

    let manifest = LudusaviManifest::from_yaml_str(text)
        .map_err(|e| anyhow::anyhow!("Erro ao analisar YAML do manifesto: {}", e))?;

    if manifest.is_empty() {
        bail!("Manifesto não contém nenhum jogo cadastrado (estrutura vazia)");
    }

    let has_valid_game_entries = manifest.games.values().any(|g| {
        !g.files.is_empty()
            || !g.registry.is_empty()
            || g.steam.is_some()
            || g.gog.is_some()
            || !g.install_dir.is_empty()
    });

    if !has_valid_game_entries {
        bail!("Manifesto não contém definições válidas de jogos, arquivos ou plataformas");
    }

    Ok(manifest)
}

/// Baixa o manifesto de uma URL, valida o status HTTP, integridade sintática/estrutural
/// e realiza gravação atômica via arquivo temporário para não corromper o cache definitivo.
pub async fn download_and_validate_manifest(manifest_url: &str, dest: &Path) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    info!(
        "Conectando a {} para download do manifesto...",
        manifest_url
    );
    let response = client.get(manifest_url).send().await?;

    let status = response.status();
    if !status.is_success() {
        bail!("Servidor remoto retornou status HTTP de falha: {}", status);
    }

    let bytes = response.bytes().await?;
    let _validated_manifest = validate_manifest_content(&bytes)?;

    let parent = dest
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Caminho de destino não possui diretório pai"))?;
    std::fs::create_dir_all(parent)?;

    // Gravação atômica: escreve no mesmo diretório em arquivo .tmp e renomeia
    let tmp_filename = format!("manifest_{}.tmp", uuid::Uuid::new_v4().simple());
    let tmp_path = parent.join(tmp_filename);

    if let Err(e) = std::fs::write(&tmp_path, &bytes) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e.into());
    }

    if let Err(e) = std::fs::rename(&tmp_path, dest) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e).context("Falha ao mover arquivo temporário para destino final do cache");
    }

    info!(
        "Manifest validado e salvo com sucesso ({:.1} MB).",
        bytes.len() as f64 / 1_048_576.0
    );
    Ok(())
}

/// Retorna o caminho para o manifest, baixando se necessário com URL e idade máxima customizáveis.
pub async fn get_manifest_path_with_options(
    cache_dir: &Path,
    manifest_url: &str,
    max_age_secs: u64,
) -> Result<PathBuf> {
    let manifest_path = cache_dir.join("manifest.yaml");

    if manifest_path.exists() {
        let age_secs = std::fs::metadata(&manifest_path)?
            .modified()
            .ok()
            .and_then(|m| std::time::SystemTime::now().duration_since(m).ok())
            .map(|d| d.as_secs())
            .unwrap_or(u64::MAX);

        if age_secs < max_age_secs {
            info!(
                "Manifest em cache ({} segundo(s) de idade). Usando versão local.",
                age_secs
            );
            return Ok(manifest_path);
        }
        info!("Manifest em cache expirado. Atualizando...");
    } else {
        info!("Manifest não encontrado em cache. Baixando pela primeira vez...");
    }

    match download_and_validate_manifest(manifest_url, &manifest_path).await {
        Ok(_) => {
            info!("Manifest atualizado com sucesso.");
            Ok(manifest_path)
        }
        Err(e) => {
            warn!("Falha ao baixar/validar manifest: {}.", e);
            if manifest_path.exists() {
                warn!("Preservando e usando versão válida em cache anterior.");
                Ok(manifest_path)
            } else {
                Err(e)
            }
        }
    }
}

/// Retorna o caminho para o manifest, baixando se necessário utilizando URL específica.
/// Em caso de falha de download ou validação, preserva o cache antigo se existente.
pub async fn get_manifest_path_with_url(cache_dir: &Path, manifest_url: &str) -> Result<PathBuf> {
    get_manifest_path_with_options(cache_dir, manifest_url, CACHE_MAX_AGE_DAYS * 86_400).await
}

/// Retorna o caminho para o manifest oficial, baixando se necessário.
pub async fn get_manifest_path(cache_dir: &Path) -> Result<PathBuf> {
    get_manifest_path_with_url(cache_dir, MANIFEST_URL).await
}
