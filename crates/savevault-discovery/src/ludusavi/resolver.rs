//! Resolvedor de variáveis e placeholders do Ludusavi Manifest para Windows.
//!
//! Centraliza a expansão de `<winAppData>`, `<winLocalAppData>`, `<winDocuments>`, etc.,
//! garantindo isolamento de ambiente, rejeição explícita de variáveis ausentes e
//! manipulação segura de caminhos sem o uso de `to_string_lossy`.

use crate::save_detector::SaveDiscoveryError;
use std::collections::HashMap;
use std::path::PathBuf;

/// Resolvedor de caminhos e placeholders do Ludusavi.
#[derive(Debug, Clone)]
pub struct PathResolver {
    variables: HashMap<String, PathBuf>,
}

impl PathResolver {
    /// Cria um resolvedor utilizando as variáveis de ambiente padrão do Windows.
    pub fn default_windows() -> Self {
        let mut vars = HashMap::new();

        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(PathBuf::from);

        if let Some(h) = &home {
            vars.insert("<home>".to_string(), h.clone());
            vars.insert("<winDocuments>".to_string(), h.join("Documents"));
            vars.insert(
                "<winLocalAppDataLow>".to_string(),
                h.join("AppData").join("LocalLow"),
            );
        }

        if let Ok(appdata) = std::env::var("APPDATA") {
            vars.insert("<winAppData>".to_string(), PathBuf::from(appdata));
        } else if let Some(h) = &home {
            vars.insert(
                "<winAppData>".to_string(),
                h.join("AppData").join("Roaming"),
            );
        }

        if let Ok(localappdata) = std::env::var("LOCALAPPDATA") {
            vars.insert("<winLocalAppData>".to_string(), PathBuf::from(localappdata));
        } else if let Some(h) = &home {
            vars.insert(
                "<winLocalAppData>".to_string(),
                h.join("AppData").join("Local"),
            );
        }

        if let Ok(programdata) = std::env::var("ProgramData") {
            vars.insert("<winProgramData>".to_string(), PathBuf::from(programdata));
        } else {
            vars.insert(
                "<winProgramData>".to_string(),
                PathBuf::from(r"C:\ProgramData"),
            );
        }

        if let Ok(public) = std::env::var("PUBLIC") {
            vars.insert("<winPublic>".to_string(), PathBuf::from(public));
        } else {
            vars.insert("<winPublic>".to_string(), PathBuf::from(r"C:\Users\Public"));
        }

        if let Ok(sysroot) = std::env::var("SystemRoot").or_else(|_| std::env::var("windir")) {
            vars.insert("<winDir>".to_string(), PathBuf::from(sysroot));
        } else {
            vars.insert("<winDir>".to_string(), PathBuf::from(r"C:\Windows"));
        }

        Self { variables: vars }
    }

    /// Cria um resolvedor vazio (essencial para testes herméticos com injeção de dependências).
    pub fn empty() -> Self {
        Self {
            variables: HashMap::new(),
        }
    }

    /// Adiciona ou substitui uma variável no resolvedor.
    pub fn with_variable(
        mut self,
        placeholder: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> Self {
        let mut key = placeholder.into();
        if !key.starts_with('<') {
            key = format!("<{}>", key);
        }
        self.variables.insert(key, path.into());
        self
    }

    /// Resolve todos os placeholders em um caminho (ex.: `<winAppData>/Game/Saves`).
    ///
    /// Retorna `Err(SaveDiscoveryError::MissingVariable)` se algum placeholder não estiver definido.
    pub fn resolve(&self, raw_path: &str) -> Result<PathBuf, SaveDiscoveryError> {
        let mut resolved_str = raw_path.to_string();

        while let Some(start) = resolved_str.find('<') {
            if let Some(end) = resolved_str[start..].find('>') {
                let full_end = start + end + 1;
                let placeholder = &resolved_str[start..full_end];

                if let Some(replacement) = self.variables.get(placeholder) {
                    let repl_str = replacement.to_str().ok_or_else(|| {
                        SaveDiscoveryError::ParseError(format!(
                            "Caminho mapeado para {} contém caracteres não-UTF8",
                            placeholder
                        ))
                    })?;
                    resolved_str.replace_range(start..full_end, repl_str);
                } else {
                    return Err(SaveDiscoveryError::MissingVariable(placeholder.to_string()));
                }
            } else {
                break;
            }
        }

        // No Windows, padroniza barras invertidas para caminhos de filesystem
        let normalized = resolved_str.replace('/', "\\");
        Ok(PathBuf::from(normalized))
    }
}
