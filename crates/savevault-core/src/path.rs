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
        return Ok(result);
    }

    // 4. Caminho relativo ou sem unidade
    let segments: Vec<&str> = converted.split('\\').filter(|s| !s.is_empty()).collect();

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
        return Ok(format!("{}:{}", drive, remainder.to_lowercase()));
    }

    // Caso contrário (UNC ou relativo), tudo em lowercase
    Ok(normalized.to_lowercase())
}

/// Retorna a versão canônica de um `Path` como `PathBuf`.
pub fn canonicalize_windows_path(path: &Path) -> Result<PathBuf, PathError> {
    let s = path.to_str().ok_or(PathError::NonUtf8)?;
    let canon = canonicalize_windows_path_str(s)?;
    Ok(PathBuf::from(canon))
}

/// Calcula o caminho relativo seguro entre um arquivo e seu diretório base.
/// Se o arquivo for igual à base ou não estiver dentro dela, retorna apenas o nome do arquivo.
pub fn compute_relative_path(file_path: &Path, base_path: &Path) -> PathBuf {
    if let (Ok(canon_file), Ok(canon_base)) = (
        canonicalize_windows_path(file_path),
        canonicalize_windows_path(base_path),
    ) {
        if let Ok(rel) = canon_file.strip_prefix(&canon_base) {
            if !rel.as_os_str().is_empty() {
                let base_comps = canon_base.components().count();
                let file_comps: Vec<_> = file_path.components().collect();
                if file_comps.len() > base_comps {
                    let mut rel_original = PathBuf::new();
                    for c in &file_comps[base_comps..] {
                        rel_original.push(c);
                    }
                    return rel_original;
                }
                return rel.to_path_buf();
            }
        }
    }
    if let Ok(rel) = file_path.strip_prefix(base_path) {
        if !rel.as_os_str().is_empty() {
            return rel.to_path_buf();
        }
    }
    file_path
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("save.dat"))
}

/// Verifica se um arquivo é de uso interno/temporário do SaveVault (backups de segurança, arquivos temporários de restore, etc.).
///
/// Arquivos internos devem ser ignorados por scanners e watchers para evitar loops de backup e poluição do banco.
pub fn is_internal_savevault_file(path: &Path) -> bool {
    let filename = match path.file_name().and_then(|n| n.to_str()) {
        Some(name) => name,
        None => return false,
    };

    filename.contains(".savevault_safety_")
        || filename.contains(".restore_tmp_")
        || filename.ends_with(".tmp")
        || filename.ends_with(".crswap")
}

/// Tenta obter o nome do usuário atual do sistema através de variáveis de ambiente do SO.
pub fn get_current_system_username() -> Option<String> {
    if let Ok(u) = std::env::var("USERNAME") {
        let trimmed = u.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Ok(u) = std::env::var("USER") {
        let trimmed = u.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Ok(up) = std::env::var("USERPROFILE") {
        let p = Path::new(&up);
        if let Some(n) = p.file_name().and_then(|s| s.to_str()) {
            let trimmed = n.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    if let Ok(h) = std::env::var("HOME") {
        let p = Path::new(&h);
        if let Some(n) = p.file_name().and_then(|s| s.to_str()) {
            let trimmed = n.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Extrai o nome de usuário presente em um caminho padrão de usuário (Windows, Linux ou macOS).
///
/// Exemplos reconhecidos:
/// - Windows: `C:\Users\Alice\Saved Games\...` -> Some("Alice")
/// - Linux: `/home/alice/.local/share/...` -> Some("alice")
/// - macOS: `/Users/alice/Library/...` -> Some("alice")
pub fn extract_username_from_path(path: &Path) -> Option<String> {
    let raw = path.to_string_lossy();
    let normalized = raw.replace('/', "\\");

    let parts: Vec<&str> = normalized
        .split('\\')
        .filter(|s| !s.is_empty() && *s != "\\\\?\\")
        .collect();

    for (i, part) in parts.iter().enumerate() {
        let lower = part.to_ascii_lowercase();
        if (lower == "users" || lower == "home") && i + 1 < parts.len() {
            let candidate = parts[i + 1];
            let cand_lower = candidate.to_ascii_lowercase();
            // Desconsiderar pastas genéricas que não são usuários reais
            if cand_lower != "public"
                && cand_lower != "default"
                && cand_lower != "all users"
                && cand_lower != "default user"
            {
                return Some(candidate.to_string());
            }
        }
    }

    None
}

/// Substitui o nome de usuário em um caminho de save mantendo a integridade da estrutura de subdiretórios.
///
/// Suporta caminhos Windows (`C:\Users\<user>\...`), Linux (`/home/<user>/...`) e macOS (`/Users/<user>/...`).
pub fn replace_username_in_path(path: &Path, target_user: &str) -> Option<PathBuf> {
    let raw = path.to_string_lossy();
    let is_windows_style = raw.contains('\\') || raw.find(':').is_some();
    let separator = if is_windows_style { '\\' } else { '/' };

    let normalized = raw.replace('/', "\\");
    let parts: Vec<&str> = normalized.split('\\').collect();

    let mut user_idx = None;
    for (i, part) in parts.iter().enumerate() {
        let lower = part.to_ascii_lowercase();
        if (lower == "users" || lower == "home") && i + 1 < parts.len() {
            let candidate = parts[i + 1];
            let cand_lower = candidate.to_ascii_lowercase();
            if cand_lower != "public"
                && cand_lower != "default"
                && cand_lower != "all users"
                && cand_lower != "default user"
            {
                user_idx = Some(i + 1);
                break;
            }
        }
    }

    let idx = user_idx?;
    let mut reconstructed = Vec::with_capacity(parts.len());
    for (i, part) in parts.iter().enumerate() {
        if i == idx {
            reconstructed.push(target_user);
        } else {
            reconstructed.push(*part);
        }
    }

    let result_str = if separator == '\\' {
        reconstructed.join("\\")
    } else {
        reconstructed.join("/")
    };

    Some(PathBuf::from(result_str))
}

/// Valida rigorosamente se um caminho de destino para remapeamento é seguro.
///
/// Rejeita:
/// - Caminho vazio;
/// - Componentes de escape ou travessia (`..`);
/// - Caracteres nulos ou bytes de controle;
/// - Raízes de unidade (ex.: `C:\` ou `/`);
/// - Pastas de sistema operacional protegidas (ex.: `C:\Windows`, `/bin`, etc.);
/// - Symlinks e junções existentes que apontem para locais protegidos ou raízes de disco.
pub fn validate_remapping_path_security(path: &Path) -> Result<(), String> {
    use std::path::Component;

    let path_str = path.to_string_lossy();
    let trimmed = path_str.trim();

    if trimmed.is_empty() {
        return Err("Caminho de destino não pode ser vazio".to_string());
    }

    if trimmed.contains('\0') {
        return Err("Caminho de destino contém bytes nulos proibidos".to_string());
    }

    for comp in path.components() {
        if matches!(comp, Component::ParentDir) {
            return Err("Tentativa de path traversal detectada ('..')".to_string());
        }
    }

    let norm_path_str = trimmed.replace('/', "\\");
    let lower = norm_path_str.to_ascii_lowercase();

    // Rejeitar raízes de unidade de disco (ex.: C:\ ou /)
    if lower.len() <= 3 && (lower.ends_with(r":\") || lower.ends_with(':') || lower == "\\") {
        return Err("O destino não pode ser a raiz da unidade de disco".to_string());
    }
    if trimmed == "/" || trimmed == "\\" {
        return Err("O destino não pode ser a raiz da unidade de disco".to_string());
    }

    // 1. Verificação sintática de diretórios protegidos do sistema operacional
    if is_system_protected_path(path) {
        return Err("O destino aponta para um diretório de sistema protegido".to_string());
    }

    // 2. Verificação física contra symlinks e junctions maliciosos:
    // Se o caminho (ou algum ancestral existente) for um link ou junção para um diretório protegido ou raiz de disco,
    // a resolução física (canonicalize) revelará o destino real.
    let mut cursor = path.to_path_buf();
    while !cursor.exists() {
        if let Some(parent) = cursor.parent() {
            if parent == cursor {
                break;
            }
            cursor = parent.to_path_buf();
        } else {
            break;
        }
    }

    if cursor.exists() {
        if let Ok(canon) = std::fs::canonicalize(&cursor) {
            if is_system_protected_path(&canon) {
                return Err(
                    "O destino físico (após resolução de links/junções) aponta para um diretório de sistema protegido".to_string(),
                );
            }

            let is_link = cursor
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);

            if cursor == path || is_link {
                let canon_norm = canon.to_string_lossy().replace('/', "\\");
                let mut canon_strip = canon_norm.as_str();
                if let Some(ext) = canon_strip.strip_prefix(r"\\?\") {
                    canon_strip = ext;
                }
                let canon_lower = canon_strip.to_ascii_lowercase();
                if canon_lower.len() <= 3
                    && (canon_lower.ends_with(r":\")
                        || canon_lower.ends_with(':')
                        || canon_lower == "\\")
                {
                    return Err(
                        "O destino físico resolvido aponta para a raiz da unidade de disco"
                            .to_string(),
                    );
                }
            }
        }
    }

    Ok(())
}

/// Determina se um caminho aponta para pastas ou arquivos protegidos do sistema operacional.
///
/// Suporta:
/// - Windows: qualquer letra de unidade (`C:\Windows`, `D:\Program Files`, etc.),
///   prefixos estendidos Win32 (`\\?\C:\...` e `\\?\UNC\...`),
///   compartilhamentos administrativos de rede (`\\server\c$`, `\\server\admin$`),
///   arquivos de sistema raiz (`pagefile.sys`, `swapfile.sys`, etc.).
/// - Unix / Linux / macOS: `/bin`, `/sbin`, `/etc`, `/usr`, `/boot`, `/sys`, `/proc`, `/dev`, `/root`,
///   e diretórios de sistema do macOS (`/System`, `/Library`, `/Applications`, `/private`, `/Network`).
pub fn is_system_protected_path(path: &Path) -> bool {
    let raw = path.to_string_lossy();
    let norm = raw.replace('/', "\\");
    let trimmed = norm.trim();

    // 1. Tratar prefixos estendidos do Windows (\\?\UNC\ -> \\ e \\?\)
    let stripped_unc;
    let mut check_str = trimmed;
    if let Some(unc) = trimmed.strip_prefix(r"\\?\UNC\") {
        stripped_unc = format!(r"\\{}", unc);
        check_str = &stripped_unc;
    } else if let Some(ext) = trimmed.strip_prefix(r"\\?\") {
        check_str = ext;
    }

    let lower = check_str.to_ascii_lowercase();

    // 2. Windows: Verificação em qualquer letra de unidade (A:, B:, C:, D:, etc.)
    let bytes = lower.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        let rest = if bytes.len() >= 3 && bytes[2] == b'\\' {
            &lower[3..]
        } else {
            &lower[2..]
        };

        if rest.starts_with("windows")
            || rest.starts_with("program files")
            || rest.starts_with("program files (x86)")
            || rest.starts_with("system volume information")
            || rest.starts_with("recovery")
            || rest.starts_with("boot")
            || rest.starts_with("perflogs")
            || rest == "pagefile.sys"
            || rest == "hiberfil.sys"
            || rest == "swapfile.sys"
        {
            return true;
        }
    }

    // 3. Windows UNC Administrative Shares (ex: \\server\c$, \\server\admin$)
    if lower.starts_with(r"\\") {
        let parts: Vec<&str> = lower.split('\\').filter(|s| !s.is_empty()).collect();
        if parts.len() >= 2 {
            let share = parts[1];
            if share.ends_with('$') || share == "admin$" || share == "ipc$" {
                return true;
            }
        }
    }

    // 4. Unix / Linux / macOS
    let unix_norm = raw.replace('\\', "/");
    let mut unix_check = unix_norm.trim();
    if let Some(ext) = unix_check.strip_prefix("//?/") {
        unix_check = ext;
    }
    let unix_lower = unix_check.to_ascii_lowercase();

    if unix_lower.starts_with("/bin")
        || unix_lower.starts_with("/sbin")
        || unix_lower.starts_with("/etc")
        || unix_lower.starts_with("/usr")
        || unix_lower.starts_with("/boot")
        || unix_lower.starts_with("/sys")
        || unix_lower.starts_with("/proc")
        || unix_lower.starts_with("/dev")
        || unix_lower.starts_with("/root")
        || unix_lower.starts_with("/system")
        || unix_lower.starts_with("/library")
        || unix_lower.starts_with("/applications")
        || unix_lower.starts_with("/private")
        || unix_lower.starts_with("/network")
    {
        return true;
    }

    false
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
        assert!(is_internal_savevault_file(Path::new(
            r"C:\Saves\slot1.sav.savevault_safety_1234"
        )));
        assert!(is_internal_savevault_file(Path::new(
            r"C:\Saves\slot1.sav.restore_tmp_abcd"
        )));
        assert!(is_internal_savevault_file(Path::new(r"C:\Saves\slot1.tmp")));
        assert!(is_internal_savevault_file(Path::new(
            r"C:\Saves\slot1.crswap"
        )));

        assert!(!is_internal_savevault_file(Path::new(
            r"C:\Saves\slot1.sav"
        )));
        assert!(!is_internal_savevault_file(Path::new(
            r"C:\Saves\slot1.dat"
        )));
        assert!(!is_internal_savevault_file(Path::new(r"C:\Saves\savegame")));
    }

    #[test]
    fn test_compute_relative_path() {
        let base = Path::new(r"C:\Users\Player\Saved Games\EldenRing");
        let file_sub = Path::new(r"C:\Users\Player\Saved Games\EldenRing\slot1\save.dat");
        let rel = compute_relative_path(file_sub, base);
        assert_eq!(rel, PathBuf::from(r"slot1\save.dat"));

        // Case insensitive match
        let base_lower = Path::new(r"c:\users\player\saved games\eldenring");
        let file_upper = Path::new(r"C:\Users\Player\Saved Games\EldenRing\slot2\deep\save.dat");
        let rel2 = compute_relative_path(file_upper, base_lower);
        assert_eq!(rel2, PathBuf::from(r"slot2\deep\save.dat"));

        // File equal to base (Single file save location)
        let single_file = Path::new(r"C:\Saves\save.dat");
        let rel_single = compute_relative_path(single_file, single_file);
        assert_eq!(rel_single, PathBuf::from("save.dat"));
    }

    #[test]
    fn test_unicode_case_folding() {
        let p1 = canonicalize_windows_path_str(r"C:\Users\JOÃO\Saved Games").unwrap();
        let p2 = canonicalize_windows_path_str(r"c:\users\joão\saved games").unwrap();
        assert_eq!(p1, p2);
    }

    #[test]
    fn test_extract_username_from_path() {
        // Windows
        assert_eq!(
            extract_username_from_path(Path::new(r"C:\Users\Alice\Saved Games\Game")),
            Some("Alice".to_string())
        );
        assert_eq!(
            extract_username_from_path(Path::new(r"C:\users\bob\AppData\Local\Game")),
            Some("bob".to_string())
        );
        assert_eq!(
            extract_username_from_path(Path::new(r"\\?\C:\Users\Charlie\Documents")),
            Some("Charlie".to_string())
        );
        // Linux
        assert_eq!(
            extract_username_from_path(Path::new("/home/alice/.local/share/game")),
            Some("alice".to_string())
        );
        // macOS
        assert_eq!(
            extract_username_from_path(Path::new("/Users/dave/Library/Application Support/game")),
            Some("dave".to_string())
        );
        // Pastas genéricas ignoradas
        assert_eq!(
            extract_username_from_path(Path::new(r"C:\Users\Public\Games")),
            None
        );
        assert_eq!(
            extract_username_from_path(Path::new(r"C:\Games\Direct")),
            None
        );
    }

    #[test]
    fn test_replace_username_in_path() {
        // Windows
        let orig_win = Path::new(r"C:\Users\Alice\Saved Games\MeuJogo");
        let rep_win = replace_username_in_path(orig_win, "Bob").unwrap();
        assert_eq!(rep_win, PathBuf::from(r"C:\Users\Bob\Saved Games\MeuJogo"));

        // Linux
        let orig_lin = Path::new("/home/alice/.local/share/meujogo");
        let rep_lin = replace_username_in_path(orig_lin, "bob").unwrap();
        assert_eq!(rep_lin, PathBuf::from("/home/bob/.local/share/meujogo"));

        // macOS
        let orig_mac = Path::new("/Users/alice/Library/Application Support/meujogo");
        let rep_mac = replace_username_in_path(orig_mac, "bob").unwrap();
        assert_eq!(
            rep_mac,
            PathBuf::from("/Users/bob/Library/Application Support/meujogo")
        );

        // Path sem username
        assert_eq!(
            replace_username_in_path(Path::new(r"D:\Games\MeuJogo"), "Bob"),
            None
        );
    }

    #[test]
    fn test_validate_remapping_path_security() {
        // Válidos
        assert!(validate_remapping_path_security(Path::new(r"C:\Games\MeuJogo\Saves")).is_ok());
        assert!(validate_remapping_path_security(Path::new(r"D:\Saves\Game")).is_ok());
        assert!(
            validate_remapping_path_security(Path::new(r"C:\Users\Player\Saved Games\Game"))
                .is_ok()
        );

        // Traversal
        assert!(validate_remapping_path_security(Path::new(r"C:\Games\..\Windows")).is_err());
        assert!(validate_remapping_path_security(Path::new(r"..\..\etc\passwd")).is_err());

        // Raiz de disco
        assert!(validate_remapping_path_security(Path::new(r"C:\")).is_err());
        assert!(validate_remapping_path_security(Path::new(r"C:")).is_err());

        // Pastas de sistema protegidas
        assert!(
            validate_remapping_path_security(Path::new(r"C:\Windows\System32\Drivers")).is_err()
        );
        assert!(
            validate_remapping_path_security(Path::new(r"C:\Program Files\Common Files")).is_err()
        );
        assert!(validate_remapping_path_security(Path::new("/bin/bash")).is_err());
        assert!(validate_remapping_path_security(Path::new("/etc/shadow")).is_err());

        // Prefixos estendidos do Windows (\\?\) e cross-drive
        assert!(validate_remapping_path_security(Path::new(r"\\?\C:\Windows\System32")).is_err());
        assert!(validate_remapping_path_security(Path::new(r"\\?\D:\Program Files\Game")).is_err());
        assert!(validate_remapping_path_security(Path::new(r"D:\Windows\System32")).is_err());
        assert!(
            validate_remapping_path_security(Path::new(r"E:\Program Files (x86)\App")).is_err()
        );

        // macOS
        assert!(
            validate_remapping_path_security(Path::new("/System/Library/CoreServices")).is_err()
        );
        assert!(validate_remapping_path_security(Path::new("/Library/Preferences")).is_err());
        assert!(validate_remapping_path_security(Path::new("/Applications/Utilities")).is_err());
        assert!(validate_remapping_path_security(Path::new("/private/etc/hosts")).is_err());

        // UNC Admin shares
        assert!(validate_remapping_path_security(Path::new(r"\\server\c$\secret")).is_err());
        assert!(validate_remapping_path_security(Path::new(r"\\server\admin$\system32")).is_err());
    }

    #[test]
    fn test_is_system_protected_path_extended() {
        assert!(is_system_protected_path(Path::new(r"C:\Windows")));
        assert!(is_system_protected_path(Path::new(r"\\?\C:\Windows")));
        assert!(is_system_protected_path(Path::new(r"d:\windows\system32")));
        assert!(is_system_protected_path(Path::new(
            r"E:\Program Files\Test"
        )));
        assert!(is_system_protected_path(Path::new(r"C:\pagefile.sys")));
        assert!(is_system_protected_path(Path::new(
            r"D:\System Volume Information"
        )));
        assert!(is_system_protected_path(Path::new("/bin/ls")));
        assert!(is_system_protected_path(Path::new("/System/Library")));
        assert!(is_system_protected_path(Path::new(
            "/Library/Application Support"
        )));
        assert!(is_system_protected_path(Path::new("/private/var/log")));

        // Pastas normais de save não devem ser bloqueadas
        assert!(!is_system_protected_path(Path::new(
            r"C:\Users\Alice\Saved Games\Game"
        )));
        assert!(!is_system_protected_path(Path::new(r"D:\Games\Saves")));
        assert!(!is_system_protected_path(Path::new(
            "/home/alice/.local/share/game"
        )));
        assert!(!is_system_protected_path(Path::new(
            "/Users/alice/Library/Application Support/game"
        )));
    }
}
