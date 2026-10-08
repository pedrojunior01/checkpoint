//! Motor de Resolução de Metadados e Capas Visuais de Jogos.
//!
//! Fornece identificação determinística de Steam AppID, geração de URLs seguras
//! de CDN oficial da Valve/Steam, persistência em cache local SQLite e fallback
//! resiliente para jogos não-Steam, emuladores e jogos sem correspondência.

use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;
use savevault_core::models::{Game, GameMetadata, MetadataSource, Platform};
use savevault_db::repositories::GameMetadataRepo;
use savevault_discovery::ludusavi::manifest::{LudusaviGameEntry, LudusaviManifest};

/// Domínio padrão de alta velocidade da Steam CDN via Cloudflare.
pub const STEAM_CDN_HEADER_BASE: &str =
    "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/";

/// Domínios CDN oficiais autorizados para imagens da Steam.
pub const ALLOWED_STEAM_DOMAINS: &[&str] = &[
    "https://shared.cloudflare.steamstatic.com/",
    "https://cdn.cloudflare.steamstatic.com/",
    "https://steamcdn-a.akamaihd.net/",
];

/// Verifica se um AppID é estritamente numérico e de tamanho seguro.
/// Rejeita vazios, "0", "000", zeros à esquerda ("000730"), caracteres não-dígito,
/// espaços, quebras de linha e valores que excedam u32::MAX.
pub fn is_valid_steam_app_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 10 || !id.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    match id.parse::<u32>() {
        Ok(val) => val > 0 && val.to_string() == id,
        Err(_) => false,
    }
}

/// Valida rigorosamente se uma URL de imagem provém de fontes oficiais seguras da Valve/Steam.
/// Rejeita schemas não-HTTPS, injeção de HTML/XSS, caminhos inseguros, query params, bypass de autoridade e domínios não autorizados.
pub fn is_valid_steam_cover_url(url: &str) -> bool {
    if url.is_empty() || url.len() > 256 {
        return false;
    }
    // Rejeita qualquer espaço, quebra de linha ou caractere de controle
    if url
        .chars()
        .any(|c| c.is_ascii_whitespace() || c.is_ascii_control())
    {
        return false;
    }
    // Domínio e scheme case-insensitive de acordo com a RFC 3986
    let matched_domain = ALLOWED_STEAM_DOMAINS.iter().find(|&&domain| {
        url.len() >= domain.len() && url[..domain.len()].eq_ignore_ascii_case(domain)
    });
    let domain = match matched_domain {
        Some(d) => d,
        None => return false,
    };
    let path = &url[domain.len()..];
    if path.is_empty()
        || path.contains('@')
        || path.contains('?')
        || path.contains('#')
        || path.contains(':')
        || path.contains("..")
        || path.contains("//")
    {
        return false;
    }
    if !path
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '.' || c == '_' || c == '-')
    {
        return false;
    }
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".png")
}

/// Constrói a URL canônica estável de cabeçalho (header capsule landscape) para um Steam AppID.
pub fn build_steam_header_url(app_id: &str) -> Option<String> {
    let trimmed = app_id.trim();
    if is_valid_steam_app_id(trimmed) {
        Some(format!("{}{}/header.jpg", STEAM_CDN_HEADER_BASE, trimmed))
    } else {
        None
    }
}

/// Resolvedor determinístico de metadados visuais de jogos.
pub struct GameMetadataResolver {
    manifest: Option<LudusaviManifest>,
}

impl Default for GameMetadataResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl GameMetadataResolver {
    /// Cria uma nova instância vazia (sem manifest em memória).
    pub fn new() -> Self {
        Self { manifest: None }
    }

    /// Cria uma nova instância com manifest pré-carregado.
    pub fn with_manifest(manifest: LudusaviManifest) -> Self {
        Self {
            manifest: Some(manifest),
        }
    }

    /// Tenta carregar o manifest do Ludusavi a partir do diretório de cache do SaveVault.
    /// Caso o arquivo não exista ou esteja indisponível, inicializa com `None` sem emitir erro fatal.
    pub fn from_cache_dir(cache_dir: &Path) -> Self {
        let manifest_path = cache_dir.join("manifest.yaml");
        let manifest = if manifest_path.exists() {
            LudusaviManifest::from_path(&manifest_path).ok()
        } else {
            None
        };
        Self { manifest }
    }

    /// Resolve o Steam AppID de um jogo de maneira determinística e conservadora.
    ///
    /// Hierarquia de prioridade:
    /// 1. Plataforma Steam com `platform_id` numérico válido.
    /// 2. Consulta no catálogo de títulos do Ludusavi com identificador Steam explícito.
    /// 3. Rejeita associações ambíguas ou conflitantes.
    pub fn resolve_app_id(&self, game: &Game) -> Option<String> {
        // Prioridade 1: Identificador Steam já presente no modelo
        if game.platform == Platform::Steam {
            if let Some(ref id) = game.platform_id {
                let trimmed = id.trim();
                if is_valid_steam_app_id(trimmed) {
                    return Some(trimmed.to_string());
                }
            }
        }

        // Prioridade 2: Consulta no Ludusavi Manifest se disponível
        if let Some(ref manifest) = self.manifest {
            if let Some(entry) = self.find_manifest_entry(manifest, &game.name) {
                if let Some(ref store_ref) = entry.steam {
                    let candidate_id = store_ref.id_string();
                    let trimmed = candidate_id.trim();
                    if is_valid_steam_app_id(trimmed) {
                        // Validação cruzada de integridade de identidade por plataforma:
                        if game.platform == Platform::Steam {
                            // Se o jogo for Steam e possuir platform_id, este deve coincidir com o manifest
                            if let Some(ref existing_id) = game.platform_id {
                                if existing_id.trim() != trimmed {
                                    tracing::warn!(
                                        "Conflito de Steam AppID para '{}': existente={}, manifest={}",
                                        game.name,
                                        existing_id,
                                        trimmed
                                    );
                                    return None;
                                }
                            }
                        } else if game.platform == Platform::Gog {
                            // Se o jogo for GOG e possuir platform_id, deve coincidir com o ID GOG declarado no manifest
                            if let Some(ref existing_id) = game.platform_id {
                                let gog_matches = entry
                                    .gog
                                    .as_ref()
                                    .map(|g| g.id_string().trim() == existing_id.trim())
                                    .unwrap_or(false);
                                if !gog_matches {
                                    tracing::warn!(
                                        "Rejeitada associação Steam para jogo GOG '{}' com platform_id='{}'",
                                        game.name,
                                        existing_id
                                    );
                                    return None;
                                }
                            }
                        } else if game.platform_id.is_some() {
                            // Jogos genéricos / emuladores com identificador próprio local não devem herdar AppID Steam
                            tracing::warn!(
                                "Jogo não-Steam '{}' possui platform_id próprio; associação de AppID rejeitada",
                                game.name
                            );
                            return None;
                        }

                        return Some(trimmed.to_string());
                    }
                }
            }
        }

        None
    }

    /// Localiza uma entrada no manifest por título exato ou case-insensitive unívoco.
    fn find_manifest_entry<'a>(
        &self,
        manifest: &'a LudusaviManifest,
        title: &str,
    ) -> Option<&'a LudusaviGameEntry> {
        // 1. Busca por igualdade exata de chave
        if let Some(entry) = manifest.games.get(title) {
            return Some(entry);
        }

        // 2. Busca case-insensitive
        let title_clean = title.trim().to_lowercase();
        let mut matches: Vec<&'a LudusaviGameEntry> = manifest
            .games
            .iter()
            .filter(|(k, _)| k.trim().to_lowercase() == title_clean)
            .map(|(_, v)| v)
            .collect();

        // Só aceita se houver correspondência unívoca (evita colisões de títulos homônimos)
        if matches.len() == 1 {
            matches.pop()
        } else {
            None
        }
    }

    /// Obtém os metadados do jogo a partir do cache local SQLite ou resolve deterministicamente.
    ///
    /// Garante que:
    /// - Se já estiver em cache com imagem válida e atualizada, retorna imediatamente sem I/O de rede.
    /// - Se os metadados forem manuais (`MetadataSource::Manual`), respeita com prioridade máxima.
    /// - Se o jogo mudou de plataforma ou AppID, invalida o cache antigo e atualiza.
    /// - Se estiver em negative cache (`MetadataSource::None`), tenta atualizar caso manifest ou AppID agora estejam disponíveis.
    /// - Executa de forma 100% offline, sem chamadas externas síncronas bloqueantes.
    pub fn resolve_or_get_cached(&self, conn: &Connection, game: &Game) -> Result<GameMetadata> {
        let meta_repo = GameMetadataRepo::new(conn);

        // 1. Verifica cache local existente
        if let Some(cached) = meta_repo.find_by_game_id(game.id)? {
            // Se metadados manuais foram explicitamente definidos, preserva com prioridade máxima
            if cached.source == MetadataSource::Manual {
                if let Some(ref url) = cached.cover_url {
                    if is_valid_steam_cover_url(url) {
                        return Ok(cached);
                    }
                } else {
                    // Usuário configurou manualmente sem capa — respeita a decisão manual
                    return Ok(cached);
                }
            }

            let expected_app_id = self.resolve_app_id(game);

            // Se o AppID resolvido atualmente coincide com o cache existente
            if cached.steam_app_id == expected_app_id {
                if let Some(ref url) = cached.cover_url {
                    if is_valid_steam_cover_url(url) {
                        return Ok(cached);
                    }
                } else if cached.source == MetadataSource::None {
                    return Ok(cached);
                }
            }
        }

        // 2. Não há cache ou o cache existente está desatualizado (mudança de ID, plataforma ou upgrade de manifest)
        if let Some(app_id) = self.resolve_app_id(game) {
            let cover_url = build_steam_header_url(&app_id);
            let source = if game.platform == Platform::Steam {
                MetadataSource::Steam
            } else {
                MetadataSource::Ludusavi
            };

            let meta = GameMetadata::new(game.id, Some(app_id), cover_url, source);
            meta_repo.upsert(&meta)?;
            Ok(meta)
        } else {
            // Jogo não-Steam sem correspondência confiável — persiste como None para negative cache
            let empty_meta = GameMetadata::empty(game.id);
            meta_repo.upsert(&empty_meta)?;
            Ok(empty_meta)
        }
    }
}
