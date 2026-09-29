//! Normalização e canonicalização lógica de caminhos Windows.
//!
//! Fornece funções puramente lógicas para manipulação de caminhos no Windows
//! sem consultar o filesystem (sem depender de `std::fs::canonicalize()`), permitindo
//! registrar locais esperados de save que ainda não existem fisicamente no disco.

use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PathError {
    #[error("Caminho não pode ser vazio")]
    EmptyPath,

    #[error("Caminho contém caracteres não-UTF8")]
    NonUtf8,

    #[error("Caminho UNC incompleto ou inválido: {0}")]
    InvalidUncPath(String),
}

/// Detecta se um caminho ou string contém caracteres curinga/glob do Windows (`*`, `?`).
///
/// No Windows (NTFS/FAT), `*` e `?` são caracteres estritamente proibidos
/// em nomes literais de arquivos e pastas pela API Win32, tornando-os indicadores inequívocos de padrões/globs.
/// Caracteres como `[` e `]` são permitidos em nomes de arquivos no Windows e não são tratados
/// como globs implícitos aqui sem sintaxe explícita.
pub fn is_glob_pattern(path: &str) -> bool {
    path.contains('*') || path.contains('?')
}

/// Detecta se um `Path` contém caracteres curinga/glob do Windows.
pub fn is_glob_path(path: &Path) -> bool {
    if let Some(s) = path.to_str() {
        is_glob_pattern(s)
    } else {
        false
    }
}

use unicode_normalization::UnicodeNormalization;

/// Limite clássico do Windows API para caminhos sem prefixo estendido (`\\?\`).
pub const MAX_PATH_WINDOWS: usize = 260;

/// Remove prefixos estendidos do Windows (`\\?\` ou `\\?\UNC\`), retornando o formato padrão.
///
/// Exemplos:
/// - `\\?\C:\Games\Save` -> `C:\Games\Save`
/// - `\\?\UNC\server\share\file` -> `\\server\share\file`
/// - `C:\Games\Save` -> `C:\Games\Save`
pub fn strip_verbatim_prefix(path_str: &str) -> String {
    if let Some(without_unc) = path_str.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{}", without_unc)
    } else if let Some(without_prefix) = path_str.strip_prefix(r"\\?\") {
        without_prefix.to_string()
    } else if let Some(without_unc_slash) = path_str.strip_prefix("//?/UNC/") {
        format!(r"\\{}", without_unc_slash)
    } else if let Some(without_prefix_slash) = path_str.strip_prefix("//?/") {
        without_prefix_slash.to_string()
    } else {
        path_str.to_string()
    }
}

/// Garante que caminhos longos (>= 260 caracteres) no Windows possuam o prefixo `\\?\`
/// para contornar a limitação de MAX_PATH em chamadas Win32/NTFS.
pub fn ensure_extended_length_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s.starts_with(r"\\?\") || s.starts_with(r"\\.\") {
        return path.to_path_buf();
    }

    if s.len() >= MAX_PATH_WINDOWS {
        if let Some(without_slash) = s.strip_prefix(r"\\") {
            // Caminho UNC: \\server\share -> \\?\UNC\server\share
            return PathBuf::from(format!(r"\\?\UNC\{}", without_slash));
        } else if s.len() >= 2 && s.as_bytes()[1] == b':' {
            // Caminho local com drive: C:\... -> \\?\C:\...
            return PathBuf::from(format!(r"\\?\{}", s));
        }
    }

    path.to_path_buf()
}

/// Normaliza um caminho no formato Windows para persistência e exibição:
/// 1. Remove prefixos verbatim desnecessários para armazenamento uniforme (`\\?\C:\...` -> `C:\...`);
/// 2. Converte `/` para `\`;
/// 3. Normaliza e colapsa separadores repetidos quando seguro (`C:\Games\\Save` -> `C:\Games\Save`);
/// 4. Remove separadores finais (trailing slashes) desnecessários;
/// 5. Preserva raízes de disco como `C:\` e prefixos UNC como `\\server\share`;
/// 6. Normaliza a letra da unidade de disco para maiúscula (ex.: `c:` -> `C:`);
/// 7. Aplica normalização Unicode NFC para consistência entre caracteres compostos e decompostos;
/// 8. Preserva a caixa original (casing) dos segmentos para fins de exibição (`display_path`).
pub fn normalize_windows_path_str(raw: &str) -> Result<String, PathError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(PathError::EmptyPath);
    }

    // 1. Descascar prefixo estendido se presente
    let stripped = strip_verbatim_prefix(trimmed);

    // 2. Converter todas as barras normais para backslashes
    let converted = stripped.replace('/', "\\");

    // 3. Tratar caminhos de rede UNC (começam com \\)
    if let Some(without_prefix) = converted.strip_prefix(r"\\") {
        let segments: Vec<&str> = without_prefix
            .split('\\')
            .filter(|s| !s.is_empty())
            .collect();

        if segments.len() < 2 {
            return Err(PathError::InvalidUncPath(raw.to_string()));
        }

        let mut result = format!(r"\\{}\{}", segments[0], segments[1]);
        if segments.len() > 2 {
            for seg in &segments[2..] {
                result.push('\\');
                result.push_str(seg);
            }
        }
        let nfc_result: String = result.nfc().collect();
        return Ok(nfc_result);
    }

    // 4. Tratar caminhos com letra de unidade Windows (ex.: C: ou c:)
    let bytes = converted.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        let drive = (bytes[0] as char).to_ascii_uppercase();
        let remainder = &converted[2..];

        // Componentes após a letra da unidade
        let segments: Vec<&str> = remainder.split('\\').filter(|s| !s.is_empty()).collect();

        if segments.is_empty() {
            // Raiz de disco: C:\ ou C:/ ou C:\\
            return Ok(format!(r"{}:\", drive));
        }

        let mut result = format!(r"{}:", drive);
        for seg in segments {
            result.push('\\');
            result.push_str(seg);
        }
        let nfc_result: String = result.nfc().collect();
        return Ok(nfc_result);
    }

    // 5. Caminho relativo ou sem unidade
    let segments: Vec<&str> = converted.split('\\').filter(|s| !s.is_empty()).collect();

    if segments.is_empty() {
        return Err(PathError::EmptyPath);
    }

    let mut result = String::new();
    if converted.starts_with('\\') {
        result.push('\\');
    }
    result.push_str(&segments.join("\\"));
    let nfc_result: String = result.nfc().collect();
    Ok(nfc_result)
}

/// Normaliza um `Path` para o formato canônico de exibição Windows.
pub fn normalize_windows_path(path: &Path) -> Result<PathBuf, PathError> {
    let s = path.to_str().ok_or(PathError::NonUtf8)?;
    let norm = normalize_windows_path_str(s)?;
    Ok(PathBuf::from(norm))
}

/// Produz a representação canônica e determinística de identidade de um caminho Windows.
///
/// Aplica as mesmas regras de `normalize_windows_path`, acrescidas de:
/// - Normalização de casing (case-folding) via Unicode `to_lowercase()`,
///   mantendo a letra da unidade em maiúscula (ex.: `C:\games\save`).
/// - Normalização Unicode NFC (garante equivalência entre caracteres compostos e decompostos).
///
/// Isso garante que:
/// - `C:\Games\Save`
/// - `c:\games\save`
/// - `\\?\C:\Games\Save`
/// - `C:/Games/Save/`
/// - `C:\Games\\Save`
/// - `C:\Games\MÿGame` e `c:\games\mÿgame`
/// - Formas NFD e NFC de acentuação
///
/// produzam a exata mesma string canônica de identidade.
pub fn canonicalize_windows_path_str(raw: &str) -> Result<String, PathError> {
    let normalized = normalize_windows_path_str(raw)?;

    // Se começa com unidade (ex.: C:\)
    let bytes = normalized.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        let drive = (bytes[0] as char).to_ascii_uppercase();
        let remainder = &normalized[2..];
        let lower_remainder: String = remainder.to_lowercase().nfc().collect();
        return Ok(format!("{}:{}", drive, lower_remainder));
    }

    // Caso contrário (UNC ou relativo), tudo em lowercase normalizado em NFC
    let full_lower: String = normalized.to_lowercase().nfc().collect();
    Ok(full_lower)
}

/// Retorna a versão canônica de um `Path` como `PathBuf`.
pub fn canonicalize_windows_path(path: &Path) -> Result<PathBuf, PathError> {
    let s = path.to_str().ok_or(PathError::NonUtf8)?;
    let canon = canonicalize_windows_path_str(s)?;
    Ok(PathBuf::from(canon))
}

/// Compara dois caminhos determinando se são semanticamente equivalentes no Windows.
pub fn paths_are_equivalent(p1: &Path, p2: &Path) -> bool {
    match (canonicalize_windows_path(p1), canonicalize_windows_path(p2)) {
        (Ok(c1), Ok(c2)) => c1 == c2,
        _ => false,
    }
}

/// Nomes reservados em sistemas de arquivos Windows (FAT / NTFS / SMB / Win32).
/// Aplicáveis case-insensitivamente e independentemente de extensões (ex.: `CON.txt`, `aux.dat`).
pub const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Verifica se um componente de nome é um nome reservado do Windows (ex.: `CON`, `NUL`, `AUX.txt`).
pub fn is_windows_reserved_name(name: &str) -> bool {
    let stem = match name.find('.') {
        Some(pos) => &name[..pos],
        None => name,
    };
    WINDOWS_RESERVED_NAMES
        .iter()
        .any(|&reserved| stem.eq_ignore_ascii_case(reserved))
}

/// Sanitiza um nome de jogo para uso seguro como pasta no filesystem (NTFS / Google Drive / Windows).
///
/// Requisitos atendidos:
/// 1. Remove separadores de caminho (`/`, `\`);
/// 2. Remove caracteres estritamente proibidos no Windows (`<`, `>`, `:`, `"`, `|`, `?`, `*`, control chars 0x00..=0x1F);
/// 3. Normaliza Unicode para forma canônica NFC;
/// 4. Impede navegação relativa (`.` e `..`);
/// 5. Remove espaços e pontos finais residuais (proibidos pela API Win32);
/// 6. Trata e neutraliza nomes reservados do Windows (`CON`, `PRN`, `AUX`, `NUL`, etc.);
/// 7. Limita o tamanho a um valor razoável (máximo 80 caracteres) para prevenir estouro de MAX_PATH;
/// 8. Preserva legibilidade;
/// 9. Caso ocorra colisão ou nome vazio, adiciona identificador estável baseado em `game_id` determinístico.
pub fn sanitize_game_name(raw_name: &str, game_id: Option<uuid::Uuid>) -> String {
    // 1. Normalização Unicode NFC
    let nfc_name: String = raw_name.nfc().collect();

    // 2. Substituição de separadores e caracteres proibidos por '_'
    let mut cleaned = String::with_capacity(nfc_name.len());
    for ch in nfc_name.chars() {
        match ch {
            '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*' => {
                cleaned.push('_');
            }
            c if (c as u32) <= 0x1F => {
                cleaned.push('_');
            }
            c => {
                cleaned.push(c);
            }
        }
    }

    // 3. Trim de espaços e pontos nas extremidades
    let trimmed = cleaned.trim_matches(|c: char| c == ' ' || c == '.');

    // 4. Se vazio, ponto ou dois pontos, aplicar fallback seguro
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        if let Some(id) = game_id {
            return format!("Game_{}", &id.to_string()[..8]);
        } else {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            raw_name.hash(&mut hasher);
            return format!("Game_{:08x}", (hasher.finish() & 0xFFFF_FFFF) as u32);
        }
    }

    // 5. Truncamento com limite de 80 caracteres para evitar MAX_PATH
    let mut truncated: String = trimmed.chars().take(80).collect();
    truncated = truncated
        .trim_matches(|c: char| c == ' ' || c == '.')
        .to_string();

    // 6. Verificação de nomes reservados do Windows
    if is_windows_reserved_name(&truncated) {
        if let Some(id) = game_id {
            return format!("game_{}_{}", truncated, &id.to_string()[..4]);
        } else {
            return format!("game_{}", truncated);
        }
    }

    truncated
}

/// Produz o nome de pasta de jogo determinístico e desambiguado para o filesystem do Google Drive:
/// Exemplo: `The Witcher 3 [c1a8]`
pub fn disambiguated_game_folder_name(raw_name: &str, game_id: uuid::Uuid) -> String {
    let clean = sanitize_game_name(raw_name, Some(game_id));
    let short_id = &game_id.to_string()[..4];
    // Se o nome já termina com o short_id ou é um fallback seguro, não duplica
    if clean.ends_with(&format!("[{}]", short_id)) || clean.ends_with(short_id) {
        clean
    } else {
        format!("{} [{}]", clean, short_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonicalize_produces_identical_identity() {
        let p1 = canonicalize_windows_path_str(r"C:\Games\Save").unwrap();
        let p2 = canonicalize_windows_path_str(r"c:\games\save").unwrap();
        let p3 = canonicalize_windows_path_str("C:/Games/Save").unwrap();
        let p4 = canonicalize_windows_path_str(r"C:\Games\Save\").unwrap();
        let p5 = canonicalize_windows_path_str(r"C:\Games\\Save").unwrap();

        assert_eq!(p1, p2);
        assert_eq!(p2, p3);
        assert_eq!(p3, p4);
        assert_eq!(p4, p5);
        assert_eq!(p1, r"C:\games\save");
    }

    #[test]
    fn test_root_drive_is_preserved() {
        let r1 = canonicalize_windows_path_str(r"C:\").unwrap();
        let r2 = canonicalize_windows_path_str("c:/").unwrap();
        let r3 = canonicalize_windows_path_str("C:").unwrap();
        let r4 = canonicalize_windows_path_str(r"C:\\\").unwrap();

        assert_eq!(r1, r2);
        assert_eq!(r2, r3);
        assert_eq!(r3, r4);
        assert_eq!(r1, r"C:\");
    }

    #[test]
    fn test_unc_paths() {
        let u1 = canonicalize_windows_path_str(r"\\Server\Share\Folder\").unwrap();
        let u2 = canonicalize_windows_path_str("//server/share/folder").unwrap();
        let u3 = canonicalize_windows_path_str(r"\\SERVER\SHARE\\folder").unwrap();

        assert_eq!(u1, u2);
        assert_eq!(u2, u3);
        assert_eq!(u1, r"\\server\share\folder");
    }

    #[test]
    fn test_glob_detection() {
        assert!(is_glob_pattern(r"C:\Games\*.sav"));
        assert!(is_glob_pattern(r"C:\Games\slot?.dat"));
        assert!(!is_glob_pattern(r"C:\Games\Save.dat"));
        assert!(!is_glob_pattern(r"C:\Games\[Mods]\save.dat"));

        // Canonicalização preserva os caracteres curinga
        let glob = canonicalize_windows_path_str(r"C:\Games\*.sav").unwrap();
        assert_eq!(glob, r"C:\games\*.sav");
        assert!(is_glob_pattern(&glob));
    }

    #[test]
    fn test_empty_path_returns_error() {
        assert_eq!(normalize_windows_path_str("   "), Err(PathError::EmptyPath));
        assert_eq!(canonicalize_windows_path_str(""), Err(PathError::EmptyPath));
    }

    #[test]
    fn test_normalize_preserves_display_casing() {
        let norm = normalize_windows_path_str(r"c:/MyGames/CoolSave/").unwrap();
        assert_eq!(norm, r"C:\MyGames\CoolSave");
    }

    #[test]
    fn test_canonicalize_unicode_case_folding() {
        // Portuguese with accents and cedilla
        let p1 = canonicalize_windows_path_str(r"C:\Jogos\Ação\Save.dat").unwrap();
        let p2 = canonicalize_windows_path_str(r"c:\jogos\AÇÃO\save.dat").unwrap();
        assert_eq!(p1, p2);
        assert_eq!(p1, r"C:\jogos\ação\save.dat");

        // German umlauts
        let g1 = canonicalize_windows_path_str(r"D:\Games\MÜNCHEN\Save.dat").unwrap();
        let g2 = canonicalize_windows_path_str(r"d:\games\münchen\SAVE.DAT").unwrap();
        assert_eq!(g1, g2);
        assert_eq!(g1, r"D:\games\münchen\save.dat");

        // Cyrillic
        let c1 = canonicalize_windows_path_str(r"C:\Игры\Сохранение\save.dat").unwrap();
        let c2 = canonicalize_windows_path_str(r"c:\игры\СОХРАНЕНИЕ\SAVE.DAT").unwrap();
        assert_eq!(c1, c2);
        assert_eq!(c1, r"C:\игры\сохранение\save.dat");

        // CJK
        let j1 = canonicalize_windows_path_str(r"C:\ゲーム\セーブ\save.dat").unwrap();
        let j2 = canonicalize_windows_path_str(r"c:\ゲーム\セーブ\SAVE.DAT").unwrap();
        assert_eq!(j1, j2);
        assert_eq!(j1, r"C:\ゲーム\セーブ\save.dat");
    }

    #[test]
    fn test_canonicalize_unicode_nfc_nfd_equivalence() {
        // NFC (precomposto: é como \u{00E9})
        let nfc = "C:\\Games\\M\u{00E9}Game\\Save.dat";
        // NFD (decomposto: e + \u{0301})
        let nfd = "c:\\games\\me\u{0301}game\\save.dat";

        let c_nfc = canonicalize_windows_path_str(nfc).unwrap();
        let c_nfd = canonicalize_windows_path_str(nfd).unwrap();

        assert_eq!(
            c_nfc, c_nfd,
            "NFC e NFD devem convergir para a exata mesma forma canônica"
        );

        // Teste com y com trema (ÿ: U+00FF / Ÿ: U+0178)
        let y1 = canonicalize_windows_path_str(r"C:\Saves\MÿGame\Slot.sav").unwrap();
        let y2 = canonicalize_windows_path_str(r"c:\saves\MŸGAME\slot.sav").unwrap();
        assert_eq!(y1, y2, "ÿ e Ÿ devem equivaler após case-folding");
    }

    #[test]
    fn test_canonicalize_verbatim_and_long_paths() {
        // Prefixo estendido \\?\C:\
        let v1 = canonicalize_windows_path_str(r"\\?\C:\Games\Save").unwrap();
        let v2 = canonicalize_windows_path_str(r"c:\games\save").unwrap();
        assert_eq!(
            v1, v2,
            "Caminho verbatim \\?\\ deve equivaler a caminho padrão"
        );

        // Prefixo estendido UNC \\?\UNC\
        let unc1 = canonicalize_windows_path_str(r"\\?\UNC\Server\Share\Folder\file.dat").unwrap();
        let unc2 = canonicalize_windows_path_str(r"\\server\share\folder\file.dat").unwrap();
        assert_eq!(unc1, unc2, "UNC verbatim deve equivaler a UNC normal");

        // Long path (> 260 chars)
        let deep_subfolders = "nested_dir_name_with_many_characters_".repeat(10);
        let long_path_str = format!(r"C:\{}\save.dat", deep_subfolders);
        assert!(
            long_path_str.len() > 300,
            "Caminho de teste deve exceder MAX_PATH (260)"
        );

        let long_path = PathBuf::from(&long_path_str);
        let extended = ensure_extended_length_path(&long_path);
        assert!(
            extended.to_string_lossy().starts_with(r"\\?\"),
            "Caminho longo deve receber prefixo estendido \\?\\ para suporte em APIs Win32/NTFS"
        );

        // Canonicalização do caminho estendido longo deve coincidir com o caminho base
        let c_long = canonicalize_windows_path(&long_path).unwrap();
        let c_extended = canonicalize_windows_path(&extended).unwrap();
        assert_eq!(c_long, c_extended);
        assert!(paths_are_equivalent(&long_path, &extended));
    }

    #[test]
    fn test_is_windows_reserved_name() {
        assert!(is_windows_reserved_name("CON"));
        assert!(is_windows_reserved_name("con"));
        assert!(is_windows_reserved_name("PRN"));
        assert!(is_windows_reserved_name("AUX"));
        assert!(is_windows_reserved_name("NUL"));
        assert!(is_windows_reserved_name("COM1"));
        assert!(is_windows_reserved_name("com9"));
        assert!(is_windows_reserved_name("LPT1"));
        assert!(is_windows_reserved_name("lpt9"));
        assert!(is_windows_reserved_name("CON.txt"));
        assert!(is_windows_reserved_name("aux.sav"));

        assert!(!is_windows_reserved_name("Control"));
        assert!(!is_windows_reserved_name("NullGame"));
        assert!(!is_windows_reserved_name("Game"));
    }

    #[test]
    fn test_sanitize_game_name_rules() {
        let fake_uuid = uuid::Uuid::parse_str("c1a84f3e-8e5b-426b-9c1d-123456789abc").unwrap();

        // 1. Separadores e caracteres perigosos
        assert_eq!(
            sanitize_game_name("Game/Name", Some(fake_uuid)),
            "Game_Name"
        );
        assert_eq!(
            sanitize_game_name(r"Game\Name", Some(fake_uuid)),
            "Game_Name"
        );
        assert_eq!(
            sanitize_game_name("Game: Subtitle *Edition?*", Some(fake_uuid)),
            "Game_ Subtitle _Edition__"
        );

        // 2. Pontos e espaços nas extremidades
        assert_eq!(sanitize_game_name("Game..", Some(fake_uuid)), "Game");
        assert_eq!(sanitize_game_name("  Game.  ", Some(fake_uuid)), "Game");

        // 3. Nomes reservados do Windows
        assert_eq!(sanitize_game_name("CON", Some(fake_uuid)), "game_CON_c1a8");
        assert_eq!(sanitize_game_name("NUL", Some(fake_uuid)), "game_NUL_c1a8");
        assert_eq!(sanitize_game_name("AUX", None), "game_AUX");
        assert_eq!(
            sanitize_game_name("CON.txt", Some(fake_uuid)),
            "game_CON.txt_c1a8"
        );

        // 4. Vazio ou navegação relativa
        assert_eq!(sanitize_game_name(".", Some(fake_uuid)), "Game_c1a84f3e");
        assert_eq!(sanitize_game_name("..", Some(fake_uuid)), "Game_c1a84f3e");
        assert_eq!(sanitize_game_name("   ", Some(fake_uuid)), "Game_c1a84f3e");

        // 5. Unicode
        assert_eq!(sanitize_game_name("MÿGame", Some(fake_uuid)), "MÿGame");

        // 6. Desambiguação
        assert_eq!(
            disambiguated_game_folder_name("The Witcher 3", fake_uuid),
            "The Witcher 3 [c1a8]"
        );
    }
}
