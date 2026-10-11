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

/// Normaliza um caminho no formato Windows para persistência e exibição:
/// 1. Converte `/` para `\`;
/// 2. Normaliza e colapsa separadores repetidos quando seguro (`C:\Games\\Save` -> `C:\Games\Save`);
/// 3. Remove separadores finais (trailing slashes) desnecessários;
/// 4. Preserva raízes de disco como `C:\` e prefixos UNC como `\\server\share`;
/// 5. Normaliza a letra da unidade de disco para maiúscula (ex.: `c:` -> `C:`);
/// 6. Preserva a caixa original (casing) dos segmentos para fins de exibição (`display_path`).
pub fn normalize_windows_path_str(raw: &str) -> Result<String, PathError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(PathError::EmptyPath);
    }

    // 1. Converter todas as barras normais para backslashes
    let mut converted = trimmed.replace('/', "\\");

    // 1.1 Remover prefixos de dispositivo/estendidos do Windows (\\?\UNC\ e \\?\)
    if let Some(unc_extended) = converted.strip_prefix(r"\\?\UNC\") {
        converted = format!(r"\\{}", unc_extended);
    } else if let Some(extended) = converted.strip_prefix(r"\\?\") {
        converted = extended.to_string();
    }

    // 2. Tratar caminhos de rede UNC (começam com \\)
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
        return Ok(result);
    }

    // 3. Tratar caminhos com letra de unidade Windows (ex.: C: ou c:)
    let bytes = converted.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        let drive = (bytes[0] as char).to_ascii_uppercase();
        let remainder = &converted[2..];

        // Componentes após a letra da unidade
        let segments: Vec<&str> = remainder
            .split('\\')
            .filter(|s| !s.is_empty())
            .collect();

        if segments.is_empty() {
            // Raiz de disco: C:\ ou C:/ ou C:\\
            return Ok(format!(r"{}:\", drive));
        }

        let mut result = format!(r"{}:", drive);
        for seg in segments {
            result.push('\\');
            result.push_str(seg);
        }
        return Ok(result);
    }

    // 4. Caminho relativo ou sem unidade
    let segments: Vec<&str> = converted
        .split('\\')
        .filter(|s| !s.is_empty())
        .collect();

    if segments.is_empty() {
        return Err(PathError::EmptyPath);
    }

    let mut result = String::new();
    if converted.starts_with('\\') {
        result.push('\\');
    }
    result.push_str(&segments.join("\\"));
    Ok(result)
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
/// - Normalização de casing (case-folding) para letras ASCII em minúsculas nos segmentos,
///   mantendo a letra da unidade em maiúscula (ex.: `C:\games\save`).
///
/// Isso garante que:
/// - `C:\Games\Save`
/// - `c:\games\save`
/// - `C:/Games/Save/`
/// - `C:\Games\\Save`
///
/// produzam a exata mesma string canônica: `C:\games\save`.
pub fn canonicalize_windows_path_str(raw: &str) -> Result<String, PathError> {
    let normalized = normalize_windows_path_str(raw)?;

    // Se começa com unidade (ex.: C:\)
    let bytes = normalized.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        let drive = (bytes[0] as char).to_ascii_uppercase();
        let remainder = &normalized[2..];
        return Ok(format!("{}:{}", drive, remainder.to_ascii_lowercase()));
    }

    // Caso contrário (UNC ou relativo), tudo em lowercase
    Ok(normalized.to_ascii_lowercase())
}

/// Retorna a versão canônica de um `Path` como `PathBuf`.
pub fn canonicalize_windows_path(path: &Path) -> Result<PathBuf, PathError> {
    let s = path.to_str().ok_or(PathError::NonUtf8)?;
    let canon = canonicalize_windows_path_str(s)?;
    Ok(PathBuf::from(canon))
}

/// Verifica se um arquivo é de uso interno/temporário do checkpoint (backups de segurança, arquivos temporários de restore, etc.).
///
/// Arquivos internos devem ser ignorados por scanners e watchers para evitar loops de backup e poluição do banco.
pub fn is_internal_savevault_file(path: &Path) -> bool {
    let filename = match path.file_name().and_then(|n| n.to_str()) {
        Some(name) => name,
        None => return false,
    };

    filename.contains(".checkpoint_safety_")
        || filename.contains(".savevault_safety_")
        || filename.contains(".restore_tmp_")
        || filename.ends_with(".tmp")
        || filename.ends_with(".crswap")
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
    fn test_extended_device_prefixes_stripped() {
        let p1 = normalize_windows_path_str(r"\\?\C:\Games\Save").unwrap();
        assert_eq!(p1, r"C:\Games\Save");
        let c1 = canonicalize_windows_path_str(r"\\?\c:\games\Save").unwrap();
        assert_eq!(c1, r"C:\games\save");

        let unc = normalize_windows_path_str(r"\\?\UNC\server\share\folder").unwrap();
        assert_eq!(unc, r"\\server\share\folder");
    }

    #[test]
    fn test_internal_savevault_file_detection() {
        assert!(is_internal_savevault_file(Path::new(r"C:\Saves\slot1.sav.savevault_safety_1234")));
        assert!(is_internal_savevault_file(Path::new(r"C:\Saves\slot1.sav.restore_tmp_abcd")));
        assert!(is_internal_savevault_file(Path::new(r"C:\Saves\slot1.tmp")));
        assert!(is_internal_savevault_file(Path::new(r"C:\Saves\slot1.crswap")));

        assert!(!is_internal_savevault_file(Path::new(r"C:\Saves\slot1.sav")));
        assert!(!is_internal_savevault_file(Path::new(r"C:\Saves\slot1.dat")));
        assert!(!is_internal_savevault_file(Path::new(r"C:\Saves\savegame")));
    }
}
