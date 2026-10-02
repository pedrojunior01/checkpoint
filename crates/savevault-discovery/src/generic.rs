//! Detector Genérico: heurística para jogos fora do Ludusavi Manifest.
//!
//! ## Estratégia (Seção 10 do MVP Spec):
//! **Preferir falsos negativos a falsos positivos.**
//!
//! Varre pastas de alta probabilidade de saves (`Documents`, `AppData/Local`,
//! `AppData/Roaming`) buscando por subpastas que contenham arquivos com extensões
//! típicas de save game (`.sav`, `.dat`, `.save`, `.sg*`, `.gam`, `.slot`).
//!
//! Só retorna entradas quando a evidência é **forte**:
//! - A pasta contém ≥ 2 arquivos de save, **ou**
//! - A pasta contém um arquivo de save com nome que sugere progresso (`save01`, `slot_1`, etc.)
//!
//! Jogos já detectados por Steam, GOG ou Emuladores NÃO são re-detectados
//! (o caller é responsável por deduplicar).

use anyhow::Result;
use savevault_core::models::{Confidence, Game, Platform, SaveLocation, SaveLocationType};
use std::path::{Path, PathBuf};
use tracing::{debug, info};
use uuid::Uuid;
use walkdir::WalkDir;

/// Extensões de arquivo que indicam save games com alta probabilidade.
const SAVE_EXTENSIONS: &[&str] = &[
    "sav", "save", "sg", "sg0", "sg1", "sg2", "sg3", "slot", "gam", "savegame", "sfs",
];

/// Palavras-chave em nomes de pasta que indicam diretório de saves.
const SAVE_DIR_HINTS: &[&str] = &[
    "save",
    "saves",
    "savegame",
    "savegames",
    "savedgame",
    "savedgames",
    "slot",
    "slots",
    "progress",
    "checkpoint",
];

/// Número mínimo de arquivos de save em uma pasta para ser considerada candidata.
const MIN_SAVE_FILES: usize = 2;

/// Profundidade máxima de varredura dentro de cada pasta-raiz.
const MAX_SCAN_DEPTH: usize = 4;

/// Resultado de um jogo detectado pelo heuristic scanner.
pub struct GenericGameInfo {
    pub game: Game,
    pub save_locations: Vec<SaveLocation>,
}

#[derive(Default)]
pub struct GenericDetector {
    /// Pastas que já foram detectadas por outros detectors (para evitar duplicatas).
    known_install_paths: Vec<PathBuf>,
}

impl GenericDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cria o detector com caminhos de jogos já conhecidos (para evitar duplicatas).
    pub fn with_known_paths(known_install_paths: Vec<PathBuf>) -> Self {
        Self {
            known_install_paths,
        }
    }

    /// Detecta jogos genéricos retornando apenas `Game` (compatível com o orchestrator).
    pub async fn detect(&self) -> Result<Vec<Game>> {
        let infos = self.detect_with_locations().await?;
        Ok(infos.into_iter().map(|i| i.game).collect())
    }

    /// Detecta jogos genéricos com seus caminhos de save.
    pub async fn detect_with_locations(&self) -> Result<Vec<GenericGameInfo>> {
        let scan_roots = build_scan_roots();
        let mut results = Vec::new();
        let mut seen_paths: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

        for root in &scan_roots {
            if !root.exists() {
                debug!(
                    "Generic: pasta de varredura '{}' não existe, pulando.",
                    root.display()
                );
                continue;
            }

            debug!("Generic: varrendo '{}'...", root.display());
            let candidates = find_save_candidates(root, MAX_SCAN_DEPTH);

            for candidate in candidates {
                // Evita duplicar a mesma pasta
                if seen_paths.contains(&candidate.path) {
                    continue;
                }

                // Evita re-detectar jogos já encontrados pelo Steam ou GOG
                if self.is_inside_known_install(&candidate.path) {
                    debug!(
                        "Generic: '{}' já coberto por outro detector, ignorando.",
                        candidate.path.display()
                    );
                    continue;
                }

                seen_paths.insert(candidate.path.clone());

                // Nome do jogo inferido pelo nome da pasta-pai da pasta de saves
                let game_name = infer_game_name(&candidate.path);
                let confidence = candidate.confidence;

                debug!(
                    "Generic: candidato '{}' ({} arquivos de save, confiança: {:?})",
                    game_name, candidate.save_file_count, confidence
                );

                let game = Game::new(game_name, Platform::Generic, None, confidence);
                let game_id = game.id;

                let loc = SaveLocation {
                    id: Uuid::new_v4(),
                    game_id,
                    path: candidate.path.clone(),
                    location_type: if candidate.is_directory {
                        SaveLocationType::Directory
                    } else {
                        SaveLocationType::File
                    },
                };

                results.push(GenericGameInfo {
                    game,
                    save_locations: vec![loc],
                });
            }
        }

        info!(
            "Generic: {} candidato(s) de save detectado(s).",
            results.len()
        );
        Ok(results)
    }

    fn is_inside_known_install(&self, path: &Path) -> bool {
        self.known_install_paths
            .iter()
            .any(|known| path.starts_with(known))
    }
}

// ---------------------------------------------------------------------------
// Candidate: pasta candidata a conter saves de um jogo
// ---------------------------------------------------------------------------

struct SaveCandidate {
    path: PathBuf,
    save_file_count: usize,
    confidence: Confidence,
    is_directory: bool,
}

// ---------------------------------------------------------------------------
// Lógica principal de varredura
// ---------------------------------------------------------------------------

/// Constrói a lista de pastas raiz para varredura baseada em variáveis de ambiente.
fn build_scan_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    let appdata = std::env::var("APPDATA").unwrap_or_default();
    let localappdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let userprofile = std::env::var("USERPROFILE").unwrap_or_default();

    // AppData/Roaming — localização mais comum para saves no Windows
    if !appdata.is_empty() {
        roots.push(PathBuf::from(&appdata));
    }

    // AppData/Local — usado por vários jogos indie e Unity
    if !localappdata.is_empty() {
        roots.push(PathBuf::from(&localappdata));
    }

    // Documents — jogos como Dark Souls, Witcher, etc.
    if !userprofile.is_empty() {
        roots.push(PathBuf::from(&userprofile).join("Documents"));
        roots.push(PathBuf::from(&userprofile).join("Saved Games"));
        roots.push(PathBuf::from(&userprofile).join("My Games"));
    }

    roots
}

/// Varre uma pasta raiz recursivamente até `max_depth` e retorna candidatas.
fn find_save_candidates(root: &Path, max_depth: usize) -> Vec<SaveCandidate> {
    let mut candidates = Vec::new();

    for entry in WalkDir::new(root)
        .max_depth(max_depth)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_dir())
    {
        let path = entry.path();

        // Ignora a própria raiz
        if path == root {
            continue;
        }

        // Descarta pastas de sistema ou temporárias conhecidas
        if should_skip_dir(path, root) {
            continue;
        }

        // Pontuação baseada no nome da pasta
        let name_score = score_dir_name(path);

        // Conta arquivos de save dentro desta pasta (sem entrar em subpastas)
        let (save_count, has_hint_name) = count_save_files_in_dir(path);

        // Critérios de elegibilidade:
        // 1. Pasta tem nome que sugere saves (saves/, savegames/) + pelo menos 1 arquivo
        // 2. Pasta tem ≥ MIN_SAVE_FILES arquivos de save
        // 3. Tem ≥ 1 arquivo com nome que sugere slot/save + extensão válida
        let qualifies = (name_score > 0 && save_count >= 1)
            || save_count >= MIN_SAVE_FILES
            || (has_hint_name && save_count >= 1);

        if !qualifies {
            continue;
        }

        let confidence = if name_score > 0 && save_count >= MIN_SAVE_FILES {
            Confidence::High
        } else if name_score > 0 || save_count >= MIN_SAVE_FILES {
            Confidence::Medium
        } else {
            Confidence::Low
        };

        candidates.push(SaveCandidate {
            path: path.to_path_buf(),
            save_file_count: save_count,
            confidence,
            is_directory: true,
        });
    }

    candidates
}

/// Conta arquivos de save na pasta (sem recursão) e verifica nomes sugestivos.
/// Retorna `(contagem, tem_nome_sugestivo)`.
fn count_save_files_in_dir(dir: &Path) -> (usize, bool) {
    let read_dir = match std::fs::read_dir(dir) {
        Ok(d) => d,
        Err(_) => return (0, false),
    };

    let mut count = 0usize;
    let mut has_hint = false;

    for entry in read_dir.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default();

        if SAVE_EXTENSIONS.contains(&ext.as_str()) {
            count += 1;

            // Verifica se o nome do arquivo sugere um slot de save
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();

            if stem.contains("save")
                || stem.contains("slot")
                || stem.contains("quicksave")
                || stem.contains("autosave")
                || stem
                    .chars()
                    .last()
                    .map(|c| c.is_ascii_digit())
                    .unwrap_or(false)
            {
                has_hint = true;
            }
        }
    }

    (count, has_hint)
}

/// Pontuação do nome do diretório: quantos hints de save o nome contém.
fn score_dir_name(path: &Path) -> usize {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();

    SAVE_DIR_HINTS
        .iter()
        .filter(|hint| name.contains(*hint))
        .count()
}

/// Decide se um diretório ou algum ancestral deve ser pulado na varredura.
fn should_skip_dir(path: &Path, root: &Path) -> bool {
    let path_str_lower = path.to_string_lossy().to_lowercase();
    if path_str_lower.contains("savevault") || path_str_lower.contains(".savevault") {
        return true;
    }

    let relative = path.strip_prefix(root).unwrap_or(path);

    let skip_list = [
        "savevault",
        ".savevault",
        "desktop_profile",
        "backups",
        "microsoft",
        "windows",
        "system32",
        "syswow64",
        "temp",
        "tmp",
        "cache",
        "log",
        "logs",
        "crash",
        "crashdumps",
        "nvidia",
        "amd",
        "intel",
        "discord",
        "slack",
        "teams",
        "zoom",
        "chrome",
        "firefox",
        "edge",
        "opera",
        "vivaldi",
        "spotify",
        "steam",
        "steamapps",
        "gog galaxy",
        "epic games",
        "origin",
        "ea",
        "ubisoft",
        "blizzard",
        "battlenet",
        "rockstar",
        "2k games",
        ".git",
        "node_modules",
        "target",
        "debug",
        "release",
        "assembly",
        "icloud",
        "dropbox",
        "onedrive",
        "utorrent",
        "bittorrent",
        "telegram",
        "obs-studio",
        "obs",
        "comfy",
        "wtablet",
        "whatsapp",
        "postman",
        "vlc",
        "docker",
    ];

    for comp in relative.components() {
        let name = comp.as_os_str().to_string_lossy().to_lowercase();
        if skip_list.iter().any(|s| name == *s || name.contains(s)) {
            return true;
        }
    }
    false
}

/// Infere o nome de um jogo pelo nome da pasta-pai da pasta de saves.
///
/// Ex: `Documents/The Witcher 3/saves/` → "The Witcher 3"
fn infer_game_name(save_path: &Path) -> String {
    // Tenta subir na hierarquia para encontrar um nome de jogo significativo
    let mut current = save_path;

    loop {
        let name = current
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        // Se o nome da pasta é um hint de save (ex: "saves"), sobe um nível
        if SAVE_DIR_HINTS
            .iter()
            .any(|h| name.to_lowercase().contains(h))
        {
            match current.parent() {
                Some(p) => {
                    current = p;
                    continue;
                }
                None => break,
            }
        }

        // Nome encontrado
        if !name.is_empty() {
            return name;
        }

        break;
    }

    // Fallback: usa o nome da pasta de saves diretamente
    save_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Jogo Desconhecido")
        .to_string()
}

// ---------------------------------------------------------------------------
// Testes
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn generic_detector_returns_ok_always() {
        let detector = GenericDetector::new();
        let result = detector.detect().await;
        assert!(result.is_ok());
    }

    #[test]
    fn score_dir_name_returns_zero_for_unrelated_dirs() {
        assert_eq!(score_dir_name(Path::new("/some/path/textures")), 0);
        assert_eq!(score_dir_name(Path::new("/some/path/audio")), 0);
    }

    #[test]
    fn score_dir_name_returns_nonzero_for_save_dirs() {
        assert!(score_dir_name(Path::new("/game/saves")) > 0);
        assert!(score_dir_name(Path::new("/game/savegames")) > 0);
        assert!(score_dir_name(Path::new("/game/SaveGames")) > 0);
    }

    #[test]
    fn should_skip_dir_returns_true_for_system_dirs() {
        let root = Path::new(r"C:\Users\foo\AppData");
        assert!(should_skip_dir(
            Path::new(r"C:\Users\foo\AppData\Roaming\Microsoft"),
            root
        ));
        assert!(should_skip_dir(
            Path::new(r"C:\Users\foo\AppData\Local\Discord"),
            root
        ));
        assert!(should_skip_dir(
            Path::new(r"C:\Users\foo\AppData\Steam\steamapps"),
            root
        ));
    }

    #[test]
    fn should_skip_dir_returns_false_for_game_dirs() {
        let root = Path::new(r"C:\Users\foo");
        assert!(!should_skip_dir(
            Path::new(r"C:\Users\foo\Documents\The Witcher 3"),
            root
        ));
        assert!(!should_skip_dir(
            Path::new(r"C:\Users\foo\AppData\Roaming\Hollow Knight"),
            root
        ));
    }

    #[test]
    fn count_save_files_finds_sav_and_dat_files() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("save01.sav"), b"data").unwrap();
        fs::write(dir.path().join("save02.sav"), b"data").unwrap();
        fs::write(dir.path().join("config.ini"), b"settings").unwrap();

        let (count, has_hint) = count_save_files_in_dir(dir.path());
        assert_eq!(count, 2);
        assert!(has_hint);
    }

    #[test]
    fn count_save_files_ignores_non_save_extensions() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("texture.png"), b"data").unwrap();
        fs::write(dir.path().join("shader.glsl"), b"data").unwrap();

        let (count, _) = count_save_files_in_dir(dir.path());
        assert_eq!(count, 0);
    }

    #[test]
    fn infer_game_name_strips_save_folder_name() {
        let path = PathBuf::from(r"C:\Users\foo\Documents\Hollow Knight\saves");
        assert_eq!(infer_game_name(&path), "Hollow Knight");
    }

    #[test]
    fn infer_game_name_returns_folder_when_no_parent() {
        let path = PathBuf::from("saves");
        // Não entra em pânico mesmo sem pasta pai
        let name = infer_game_name(&path);
        assert!(!name.is_empty());
    }

    #[test]
    fn find_save_candidates_detects_saves_dir() {
        let root = tempdir().unwrap();
        let game_dir = root.path().join("Hollow Knight");
        let saves_dir = game_dir.join("saves");
        fs::create_dir_all(&saves_dir).unwrap();

        // Cria arquivos de save suficientes para atender ao critério
        for i in 0..3 {
            fs::write(saves_dir.join(format!("save{}.sav", i)), b"data").unwrap();
        }

        let candidates = find_save_candidates(root.path(), 4);
        assert!(
            candidates.iter().any(|c| c.path == saves_dir),
            "Expected saves dir to be detected, candidates: {:?}",
            candidates.iter().map(|c| &c.path).collect::<Vec<_>>()
        );
    }

    #[test]
    fn find_save_candidates_skips_system_dirs() {
        let root = tempdir().unwrap();
        let ms_dir = root.path().join("Microsoft").join("saves");
        fs::create_dir_all(&ms_dir).unwrap();
        fs::write(ms_dir.join("save.sav"), b"data").unwrap();
        fs::write(ms_dir.join("save2.sav"), b"data").unwrap();

        let candidates = find_save_candidates(root.path(), 4);
        assert!(
            !candidates.iter().any(|c| c.path == ms_dir),
            "Microsoft dir should be skipped"
        );
    }

    #[test]
    fn generic_detector_skips_known_install_paths() {
        let root = tempdir().unwrap();
        let game_dir = root.path().join("MyGame").join("saves");
        std::fs::create_dir_all(&game_dir).unwrap();
        for i in 0..3 {
            fs::write(game_dir.join(format!("slot{}.sav", i)), b"data").unwrap();
        }

        let detector = GenericDetector::with_known_paths(vec![root.path().to_path_buf()]);
        let candidates = find_save_candidates(root.path(), 4);
        let filtered: Vec<_> = candidates
            .into_iter()
            .filter(|c| !detector.is_inside_known_install(&c.path))
            .collect();

        assert!(filtered.is_empty(), "Known install paths should be skipped");
    }

    #[test]
    fn find_save_candidates_skips_savevault_and_backups_dirs() {
        let root = tempdir().unwrap();
        let sv_dir = root.path().join("SaveVault").join("backups").join("v1");
        fs::create_dir_all(&sv_dir).unwrap();
        fs::write(sv_dir.join("save.sav"), b"data").unwrap();
        fs::write(sv_dir.join("save2.sav"), b"data").unwrap();

        let candidates = find_save_candidates(root.path(), 5);
        assert!(
            !candidates.iter().any(|c| c.path == sv_dir),
            "SaveVault backups dir must be skipped"
        );
    }
}

