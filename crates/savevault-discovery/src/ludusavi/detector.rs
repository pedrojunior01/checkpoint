//! Implementação do `SaveDetector` para o Ludusavi Manifest.
//!
//! Converte entradas do manifest para o modelo canônico `SaveLocation`,
//! associando jogos por Store ID (Steam/GOG) ou título e resolvendo caminhos.

use std::path::Path;
use savevault_core::models::{Game, Platform, SaveLocation, SaveLocationType};
use savevault_core::path::{is_glob_path, is_glob_pattern};
use crate::save_detector::{SaveDetector, SaveDiscoveryError, SaveDiscoveryResult};
use super::manifest::{LudusaviGameEntry, LudusaviManifest};
use super::resolver::PathResolver;

enum GameLookupResult<'a> {
    Found(&'a LudusaviGameEntry),
    NotFound,
    ConflictingIdentity {
        game_name: String,
        platform: Platform,
        expected_platform_id: String,
        manifest_platform_id: String,
    },
}

pub struct LudusaviSaveDetector {
    manifest: LudusaviManifest,
    resolver: PathResolver,
}

impl LudusaviSaveDetector {
    /// Cria o detector carregando o manifest a partir de uma string YAML e usando o resolver padrão do Windows.
    pub fn from_yaml_str(yaml: &str) -> Result<Self, SaveDiscoveryError> {
        let manifest = LudusaviManifest::from_yaml_str(yaml)?;
        Ok(Self {
            manifest,
            resolver: PathResolver::default_windows(),
        })
    }

    /// Cria o detector carregando o manifest a partir de um arquivo no disco.
    pub fn from_file(path: &Path) -> Result<Self, SaveDiscoveryError> {
        let manifest = LudusaviManifest::from_path(path)?;
        Ok(Self {
            manifest,
            resolver: PathResolver::default_windows(),
        })
    }

    /// Construtor customizado permitindo injetar o manifest e o resolver (para testes isolados).
    pub fn new(manifest: LudusaviManifest, resolver: PathResolver) -> Self {
        Self { manifest, resolver }
    }

    /// Localiza uma entrada no manifest por correspondência de título (exato ou case-insensitive).
    fn find_entry_by_title<'a>(&'a self, title_query: &str) -> Option<&'a LudusaviGameEntry> {
        if let Some(entry) = self.manifest.games.get(title_query) {
            return Some(entry);
        }
        for (title, entry) in &self.manifest.games {
            if title.eq_ignore_ascii_case(title_query) {
                return Some(entry);
            }
        }
        None
    }

    /// Localiza a entrada do jogo no manifest respeitando a precedência de identificadores confiáveis
    /// e impedindo associações silenciosas incorretas quando houver conflito entre platform_id e nome:
    /// 1. Busca prioritária por Store ID (Steam AppID / GOG ID);
    /// 2. Se possuir Store ID e não for encontrado no manifest, verifica se a entrada encontrada por nome
    ///    possui um Store ID conflitante para essa mesma plataforma;
    /// 3. Se houver conflito evidente, retorna `ConflictingIdentity` em vez de associar silenciosamente;
    /// 4. Se não houver conflito, realiza o fallback seguro por nome.
    fn find_game_entry<'a>(&'a self, game: &Game) -> GameLookupResult<'a> {
        // 1. Tentar por Steam AppID
        if game.platform == Platform::Steam {
            if let Some(app_id) = &game.platform_id {
                for entry in self.manifest.games.values() {
                    if let Some(steam_ref) = &entry.steam {
                        if steam_ref.id_string() == *app_id {
                            return GameLookupResult::Found(entry);
                        }
                    }
                }

                // Não encontrado por Steam AppID: verificar se existe entrada com mesmo nome
                if let Some(entry_by_name) = self.find_entry_by_title(&game.name) {
                    if let Some(steam_ref) = &entry_by_name.steam {
                        let manifest_id = steam_ref.id_string();
                        if manifest_id != *app_id {
                            // Conflito crítico: o jogo na máquina possui AppID X, mas o manifest declara AppID Y para este título
                            return GameLookupResult::ConflictingIdentity {
                                game_name: game.name.clone(),
                                platform: Platform::Steam,
                                expected_platform_id: app_id.clone(),
                                manifest_platform_id: manifest_id,
                            };
                        }
                    }
                    // A entrada encontrada por nome não possui AppID conflitante declarado
                    return GameLookupResult::Found(entry_by_name);
                }

                return GameLookupResult::NotFound;
            }
        }

        // 2. Tentar por GOG ID
        if game.platform == Platform::Gog {
            if let Some(gog_id) = &game.platform_id {
                for entry in self.manifest.games.values() {
                    if let Some(gog_ref) = &entry.gog {
                        if gog_ref.id_string() == *gog_id {
                            return GameLookupResult::Found(entry);
                        }
                    }
                }

                // Não encontrado por GOG ID: verificar se existe entrada com mesmo nome
                if let Some(entry_by_name) = self.find_entry_by_title(&game.name) {
                    if let Some(gog_ref) = &entry_by_name.gog {
                        let manifest_id = gog_ref.id_string();
                        if manifest_id != *gog_id {
                            return GameLookupResult::ConflictingIdentity {
                                game_name: game.name.clone(),
                                platform: Platform::Gog,
                                expected_platform_id: gog_id.clone(),
                                manifest_platform_id: manifest_id,
                            };
                        }
                    }
                    return GameLookupResult::Found(entry_by_name);
                }

                return GameLookupResult::NotFound;
            }
        }

        // 3. Jogo sem platform_id ou plataforma genérica/emulador: busca nominal direta
        if let Some(entry) = self.find_entry_by_title(&game.name) {
            return GameLookupResult::Found(entry);
        }

        GameLookupResult::NotFound
    }

    /// Determina o tipo do save location sem exigir destrutivamente a existência no disco.
    ///
    /// Se o caminho contiver caracteres de padrão/glob (`*`, `?`), classifica imediatamente
    /// como `SaveLocationType::Glob` sem consultar o filesystem.
    fn determine_location_type(raw_path: &str, resolved_path: &Path) -> SaveLocationType {
        // Se contém caracteres de glob, NUNCA consultar o filesystem
        if is_glob_pattern(raw_path) || is_glob_path(resolved_path) {
            return SaveLocationType::Glob;
        }

        // Se já existe no disco, usa a verificação física direta
        if resolved_path.exists() {
            if resolved_path.is_file() {
                return SaveLocationType::File;
            }
            if resolved_path.is_dir() {
                return SaveLocationType::Directory;
            }
        }

        // Se ainda não foi criado fisicamente no disco:
        // Se termina com barra ou não possui extensão de arquivo, convenciona-se pasta de saves
        if raw_path.ends_with('/') || raw_path.ends_with('\\') || resolved_path.extension().is_none() {
            SaveLocationType::Directory
        } else {
            SaveLocationType::File
        }
    }
}

impl SaveDetector for LudusaviSaveDetector {
    fn name(&self) -> &'static str {
        "ludusavi-manifest"
    }

    fn detect_save_locations(&self, game: &Game) -> Result<SaveDiscoveryResult, SaveDiscoveryError> {
        let entry = match self.find_game_entry(game) {
            GameLookupResult::Found(res) => res,
            GameLookupResult::NotFound => return Ok(SaveDiscoveryResult::UnrecognizedGame),
            GameLookupResult::ConflictingIdentity {
                game_name,
                platform,
                expected_platform_id,
                manifest_platform_id,
            } => {
                return Ok(SaveDiscoveryResult::ConflictingGameIdentity {
                    game_name,
                    platform,
                    expected_platform_id,
                    manifest_platform_id,
                });
            }
        };

        let store_name = match game.platform {
            Platform::Steam => Some("steam"),
            Platform::Gog => Some("gog"),
            _ => None,
        };

        let mut discovered = Vec::new();
        let mut any_exists_on_disk = false;

        // Processar entradas de arquivos do manifest
        for (raw_path, file_meta) in &entry.files {
            // Filtrar apenas o que é save e aplicável ao Windows e loja
            if !file_meta.is_save() || !file_meta.is_applicable("windows", store_name) {
                continue;
            }

            let resolved_path = self.resolver.resolve(raw_path)?;
            let location_type = Self::determine_location_type(raw_path, &resolved_path);

            // Se for Glob, não verificar existência física direta (expansão ocorrerá na etapa de scan)
            if location_type != SaveLocationType::Glob && resolved_path.exists() {
                any_exists_on_disk = true;
            }

            discovered.push(SaveLocation::new(game.id, resolved_path, location_type));
        }

        if discovered.is_empty() {
            // O jogo é conhecido, mas não possui saves aplicáveis para a plataforma Windows
            return Ok(SaveDiscoveryResult::Found(vec![]));
        }

        if any_exists_on_disk {
            Ok(SaveDiscoveryResult::Found(discovered))
        } else {
            Ok(SaveDiscoveryResult::KnownNotFoundOnDisk {
                expected_locations: discovered,
            })
        }
    }
}
