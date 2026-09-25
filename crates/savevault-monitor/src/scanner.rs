//! Scanner de arquivos de save.
//!
//! Descobre arquivos físicos relevantes para um determinado `SaveLocation`,
//! suportando arquivos individuais (`File`), diretórios (`Directory`) e padrões (`Glob`).
//!
//! Não segue reparse points / symlinks de forma ilimitada para segurança contra loops
//! e restringe a expansão de globs estritamente ao diretório base esperado,
//! rejeitando tentativas de path traversal com `..`.

use std::path::{Path, PathBuf};
use std::time::SystemTime;
use uuid::Uuid;
use savevault_core::models::{SaveLocation, SaveLocationType};
use savevault_core::path::is_glob_pattern;
use thiserror::Error;
use walkdir::WalkDir;

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("Erro de I/O ao escanear caminho '{path}': {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("Caminho de save não contém formato UTF-8 válido: '{0}'")]
    NonUtf8Path(String),

    #[error("Tentativa de path traversal detectada no caminho: '{0}'")]
    PathTraversal(PathBuf),
}

/// Representa um arquivo físico de save descoberto no disco.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFile {
    pub path: PathBuf,
    pub save_location_id: Uuid,
    pub size_bytes: u64,
    pub modified_at: SystemTime,
}

/// Scanner configurável para descobrir arquivos em `SaveLocation`s.
pub struct SaveScanner {
    max_depth: usize,
}

impl Default for SaveScanner {
    fn default() -> Self {
        Self::new(10)
    }
}

impl SaveScanner {
    pub fn new(max_depth: usize) -> Self {
        Self { max_depth }
    }

    /// Escaneia um `SaveLocation` e retorna os arquivos físicos encontrados.
    ///
    /// Se o caminho não existir no disco, retorna `Ok(vec![])` sem erro fatal.
    pub fn scan(&self, location: &SaveLocation) -> Result<Vec<DiscoveredFile>, ScanError> {
        match location.location_type {
            SaveLocationType::File => self.scan_file(location),
            SaveLocationType::Directory => self.scan_directory(location),
            SaveLocationType::Glob => self.scan_glob(location),
            SaveLocationType::Registry => {
                // Chaves de registro não representam arquivos no filesystem
                Ok(vec![])
            }
        }
    }

    /// Escaneia uma localização do tipo `File`.
    fn scan_file(&self, location: &SaveLocation) -> Result<Vec<DiscoveredFile>, ScanError> {
        if !location.path.exists() {
            return Ok(vec![]);
        }

        // Se existir mas for um diretório, não tratar como arquivo
        if !location.path.is_file() {
            return Ok(vec![]);
        }

        match std::fs::metadata(&location.path) {
            Ok(meta) => {
                let size_bytes = meta.len();
                let modified_at = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                Ok(vec![DiscoveredFile {
                    path: location.path.clone(),
                    save_location_id: location.id,
                    size_bytes,
                    modified_at,
                }])
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(source) => Err(ScanError::Io {
                path: location.path.clone(),
                source,
            }),
        }
    }

    /// Escaneia uma localização do tipo `Directory`.
    fn scan_directory(&self, location: &SaveLocation) -> Result<Vec<DiscoveredFile>, ScanError> {
        if !location.path.exists() || !location.path.is_dir() {
            return Ok(vec![]);
        }

        let mut files = Vec::new();
        let walker = WalkDir::new(&location.path)
            .follow_links(false)
            .max_depth(self.max_depth);

        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    // Erro de acesso em arquivo individual durante travessia não interrompe tudo
                    tracing::debug!("Aviso ao ler entrada no diretório: {}", err);
                    continue;
                }
            };

            // Ignorar diretórios e symlinks
            if entry.file_type().is_dir() || entry.path_is_symlink() {
                continue;
            }

            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    files.push(DiscoveredFile {
                        path: entry.into_path(),
                        save_location_id: location.id,
                        size_bytes: meta.len(),
                        modified_at: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    });
                }
            }
        }

        Ok(files)
    }

    /// Escaneia uma localização do tipo `Glob`.
    ///
    /// Separa o caminho base seguro do padrão e garante que a busca não ultrapasse
    /// a raiz base delimitada, suportando subdiretórios com wildcards.
    fn scan_glob(&self, location: &SaveLocation) -> Result<Vec<DiscoveredFile>, ScanError> {
        let (base_dir, pattern) = match split_glob_base_and_pattern(&location.path) {
            Ok(pair) => pair,
            Err(ScanError::PathTraversal(p)) => {
                tracing::warn!("Tentativa de path traversal em glob rejeitada: {}", p.display());
                return Err(ScanError::PathTraversal(p));
            }
            Err(e) => return Err(e),
        };

        if !base_dir.exists() || !base_dir.is_dir() {
            return Ok(vec![]);
        }

        let mut files = Vec::new();
        let walker = WalkDir::new(&base_dir)
            .follow_links(false)
            .max_depth(self.max_depth);

        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    tracing::debug!("Aviso ao ler entrada no glob: {}", err);
                    continue;
                }
            };

            if entry.file_type().is_dir() || entry.path_is_symlink() {
                continue;
            }

            let rel_path = match entry.path().strip_prefix(&base_dir) {
                Ok(p) => p,
                Err(_) => continue,
            };

            // Compara o caminho relativo contra o padrão glob
            if match_glob_relative(&pattern, rel_path) {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        files.push(DiscoveredFile {
                            path: entry.into_path(),
                            save_location_id: location.id,
                            size_bytes: meta.len(),
                            modified_at: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                        });
                    }
                }
            }
        }

        Ok(files)
    }
}

/// Separa o diretório base estático do padrão wildcard de um caminho,
/// protegendo estritamente contra path traversal com `..`.
///
/// Exemplo: `C:\Games\Example\*.sav` -> (`C:\Games\Example`, `*.sav`)
/// Exemplo: `C:\Games\*\*.sav` -> (`C:\Games`, `*\*.sav`)
pub fn split_glob_base_and_pattern(path: &Path) -> Result<(PathBuf, String), ScanError> {
    let mut in_pattern = false;
    let mut base_components: Vec<std::ffi::OsString> = Vec::new();
    let mut pattern_parts: Vec<String> = Vec::new();

    for comp in path.components() {
        match comp {
            std::path::Component::Prefix(p) => {
                if in_pattern {
                    return Err(ScanError::PathTraversal(path.to_path_buf()));
                }
                base_components.push(p.as_os_str().to_os_string());
            }
            std::path::Component::RootDir => {
                if in_pattern {
                    return Err(ScanError::PathTraversal(path.to_path_buf()));
                }
                base_components.push(comp.as_os_str().to_os_string());
            }
            std::path::Component::CurDir => {
                // '.' dentro do caminho pode ser ignorado de forma segura
            }
            std::path::Component::ParentDir => {
                if in_pattern {
                    // '..' após curinga é expressamente rejeitado como traversal
                    return Err(ScanError::PathTraversal(path.to_path_buf()));
                }
                // Colapsa componente normal anterior se existir
                if let Some(last) = base_components.last() {
                    let last_str = last.to_string_lossy();
                    if last_str != "\\" && !last_str.ends_with(':') {
                        base_components.pop();
                    } else {
                        // Tentativa de escapar acima da raiz de unidade (ex.: C:\..\)
                        return Err(ScanError::PathTraversal(path.to_path_buf()));
                    }
                } else {
                    return Err(ScanError::PathTraversal(path.to_path_buf()));
                }
            }
            std::path::Component::Normal(s) => {
                let s_str = s.to_str().ok_or_else(|| {
                    ScanError::NonUtf8Path(path.to_string_lossy().to_string())
                })?;

                if in_pattern {
                    pattern_parts.push(s_str.to_string());
                } else if is_glob_pattern(s_str) {
                    in_pattern = true;
                    pattern_parts.push(s_str.to_string());
                } else {
                    base_components.push(s.to_os_string());
                }
            }
        }
    }

    let mut base = PathBuf::new();
    for c in base_components {
        base.push(c);
    }

    let pattern = if pattern_parts.is_empty() {
        "*".to_string()
    } else {
        pattern_parts.join("\\")
    };

    Ok((base, pattern))
}

/// Compara um padrão glob (que pode conter subdiretórios como `*\*.sav` ou `Game*\*.sav`)
/// contra um caminho relativo dentro da raiz de busca.
pub fn match_glob_relative(pattern: &str, relative_path: &Path) -> bool {
    let p_norm = pattern.replace('/', "\\");
    let p_segments: Vec<&str> = p_norm.split('\\').filter(|s| !s.is_empty()).collect();

    let mut r_segments = Vec::new();
    for comp in relative_path.components() {
        if let std::path::Component::Normal(s) = comp {
            if let Some(s_str) = s.to_str() {
                r_segments.push(s_str);
            } else {
                return false;
            }
        }
    }

    match_segments(&p_segments, &r_segments)
}

fn match_segments(p_segs: &[&str], r_segs: &[&str]) -> bool {
    if p_segs.is_empty() {
        return r_segs.is_empty();
    }

    if p_segs[0] == "**" {
        for i in 0..=r_segs.len() {
            if match_segments(&p_segs[1..], &r_segs[i..]) {
                return true;
            }
        }
        return false;
    }

    if r_segs.is_empty() {
        return p_segs.iter().all(|&s| s == "**");
    }

    if wildcard_match(p_segs[0], r_segs[0]) {
        match_segments(&p_segs[1..], &r_segs[1..])
    } else {
        false
    }
}

/// Correspondência simples e determinística de wildcards (`*` e `?`).
///
/// - `*` corresponde a 0 ou mais caracteres.
/// - `?` corresponde a exatamente 1 caractere.
/// - Case-insensitive para compatibilidade nativa com arquivos Windows.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let mut p_idx = 0;
    let mut t_idx = 0;
    let mut star_idx = None;
    let mut match_idx = 0;

    while t_idx < t.len() {
        if p_idx < p.len() && (p[p_idx] == '?' || p[p_idx].eq_ignore_ascii_case(&t[t_idx])) {
            p_idx += 1;
            t_idx += 1;
        } else if p_idx < p.len() && p[p_idx] == '*' {
            star_idx = Some(p_idx);
            match_idx = t_idx;
            p_idx += 1;
        } else if let Some(star) = star_idx {
            p_idx = star + 1;
            match_idx += 1;
            t_idx = match_idx;
        } else {
            return false;
        }
    }

    while p_idx < p.len() && p[p_idx] == '*' {
        p_idx += 1;
    }

    p_idx == p.len()
}
