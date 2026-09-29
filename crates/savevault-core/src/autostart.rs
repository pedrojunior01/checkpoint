//! Gerenciador de inicialização automática com o Windows.
//!
//! Registra e remove a execução do SaveVault no Registro do Windows sob:
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Nome da chave no Registro do Windows.
pub const APP_NAME: &str = "SaveVault";

/// Subchave padrão de inicialização de aplicativos do usuário no Windows.
pub const REG_RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Representa o estado atual de inicialização automática com o sistema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutostartStatus {
    /// Se a inicialização automática está ativa no Registro do Windows.
    pub enabled: bool,
    /// O comando completo registrado (ex.: `"C:\Users\...\savevault.exe"`).
    pub command: Option<String>,
    /// Caminho do executável extraído do comando.
    pub executable_path: Option<PathBuf>,
}

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE};
    use winreg::RegKey;

    /// Verifica se o SaveVault está configurado para inicializar com o Windows.
    pub fn is_enabled() -> Result<bool> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let run_key = match hkcu.open_subkey_with_flags(REG_RUN_KEY, KEY_QUERY_VALUE) {
            Ok(k) => k,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e).context("Falha ao abrir chave de inicialização no Registro"),
        };

        match run_key.get_value::<String, _>(APP_NAME) {
            Ok(val) => Ok(!val.trim().is_empty()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e).context("Falha ao consultar valor de inicialização do SaveVault"),
        }
    }

    /// Retorna os detalhes da configuração atual de inicialização automática.
    pub fn status() -> Result<AutostartStatus> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let run_key = match hkcu.open_subkey_with_flags(REG_RUN_KEY, KEY_QUERY_VALUE) {
            Ok(k) => k,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(AutostartStatus {
                    enabled: false,
                    command: None,
                    executable_path: None,
                });
            }
            Err(e) => return Err(e).context("Falha ao abrir chave de inicialização no Registro"),
        };

        match run_key.get_value::<String, _>(APP_NAME) {
            Ok(cmd) if !cmd.trim().is_empty() => {
                let exe_path = extract_executable_path(&cmd);
                Ok(AutostartStatus {
                    enabled: true,
                    command: Some(cmd),
                    executable_path: exe_path,
                })
            }
            _ => Ok(AutostartStatus {
                enabled: false,
                command: None,
                executable_path: None,
            }),
        }
    }

    /// Habilita a inicialização automática do SaveVault registrando o executável em `HKCU\...\Run`.
    ///
    /// Se `custom_exe_path` for `None`, utiliza o caminho do executável do processo atual (`std::env::current_exe`).
    pub fn enable(custom_exe_path: Option<&Path>) -> Result<AutostartStatus> {
        let exe_path = match custom_exe_path {
            Some(p) => extract_executable_path(&p.to_string_lossy()).unwrap_or_else(|| p.to_path_buf()),
            None => std::env::current_exe()
                .context("Falha ao obter o caminho do executável atual do SaveVault")?,
        };

        // Normaliza o caminho do executável
        let canonical_path = if exe_path.exists() {
            std::fs::canonicalize(&exe_path).unwrap_or(exe_path)
        } else {
            exe_path
        };
        let normalized = crate::path::normalize_windows_path(&canonical_path).unwrap_or(canonical_path);

        // Formata o comando com aspas para suportar caminhos com espaços e flag --tray para iniciar silencioso na bandeja
        let command_str = format!("\"{}\" --tray", normalized.display());

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (run_key, _) = hkcu
            .create_subkey_with_flags(REG_RUN_KEY, KEY_SET_VALUE)
            .context("Falha ao abrir/criar chave de inicialização no Registro para escrita")?;

        run_key
            .set_value(APP_NAME, &command_str)
            .context("Falha ao gravar valor de inicialização do SaveVault no Registro")?;

        Ok(AutostartStatus {
            enabled: true,
            command: Some(command_str),
            executable_path: Some(normalized),
        })
    }

    /// Desabilita a inicialização automática do SaveVault, removendo o valor de `HKCU\...\Run`.
    pub fn disable() -> Result<AutostartStatus> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let run_key = match hkcu.open_subkey_with_flags(REG_RUN_KEY, KEY_SET_VALUE) {
            Ok(k) => k,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(AutostartStatus {
                    enabled: false,
                    command: None,
                    executable_path: None,
                });
            }
            Err(e) => return Err(e).context("Falha ao abrir chave de inicialização no Registro"),
        };

        match run_key.delete_value(APP_NAME) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(e).context("Falha ao remover valor de inicialização do SaveVault");
            }
        }

        Ok(AutostartStatus {
            enabled: false,
            command: None,
            executable_path: None,
        })
    }
}

#[cfg(not(windows))]
mod non_windows_impl {
    use super::*;

    pub fn is_enabled() -> Result<bool> {
        Ok(false)
    }

    pub fn status() -> Result<AutostartStatus> {
        Ok(AutostartStatus {
            enabled: false,
            command: None,
            executable_path: None,
        })
    }

    pub fn enable(custom_exe_path: Option<&Path>) -> Result<AutostartStatus> {
        let exe = match custom_exe_path {
            Some(p) => extract_executable_path(&p.to_string_lossy()).unwrap_or_else(|| p.to_path_buf()),
            None => PathBuf::from("savevault"),
        };
        Ok(AutostartStatus {
            enabled: true,
            command: Some(format!("\"{}\" --tray", exe.display())),
            executable_path: Some(exe),
        })
    }

    pub fn disable() -> Result<AutostartStatus> {
        Ok(AutostartStatus {
            enabled: false,
            command: None,
            executable_path: None,
        })
    }
}

#[cfg(windows)]
pub use windows_impl::*;

#[cfg(not(windows))]
pub use non_windows_impl::*;

/// Extrai o caminho do executável de uma linha de comando com ou sem aspas.
pub fn extract_executable_path(command: &str) -> Option<PathBuf> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(without_prefix) = trimmed.strip_prefix('"') {
        if let Some(end_quote) = without_prefix.find('"') {
            return Some(PathBuf::from(&without_prefix[..end_quote]));
        }
    }

    // Se não tiver aspas, pega a primeira palavra antes do espaço
    let first_part = trimmed.split_whitespace().next().unwrap_or(trimmed);
    Some(PathBuf::from(first_part))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_executable_path_with_quotes() {
        let cmd = r#""C:\Program Files\SaveVault\savevault.exe" --silent"#;
        let path = extract_executable_path(cmd).expect("Deveria extrair caminho com aspas");
        assert_eq!(
            path,
            PathBuf::from(r"C:\Program Files\SaveVault\savevault.exe")
        );
    }

    #[test]
    fn test_extract_executable_path_without_quotes() {
        let cmd = r"C:\SaveVault\savevault.exe --daemon";
        let path = extract_executable_path(cmd).expect("Deveria extrair caminho sem aspas");
        assert_eq!(path, PathBuf::from(r"C:\SaveVault\savevault.exe"));
    }

    #[test]
    fn test_extract_executable_path_empty() {
        assert_eq!(extract_executable_path(""), None);
        assert_eq!(extract_executable_path("   "), None);
    }

    #[test]
    fn test_autostart_status_serialization() {
        let status = AutostartStatus {
            enabled: true,
            command: Some(r#""C:\savevault.exe""#.into()),
            executable_path: Some(PathBuf::from(r"C:\savevault.exe")),
        };
        let json = serde_json::to_string(&status).expect("Serialization failed");
        let decoded: AutostartStatus =
            serde_json::from_str(&json).expect("Deserialization failed");
        assert_eq!(status, decoded);
    }

    #[cfg(windows)]
    #[test]
    fn test_autostart_roundtrip_windows() {
        // Testa o ciclo de status, enable com path fictício e disable
        let test_exe = PathBuf::from(r"C:\Windows\System32\cmd.exe");
        let prev_status = status().expect("Falha ao obter status");

        // Habilita
        let enabled_status = enable(Some(&test_exe)).expect("Falha ao habilitar");
        assert!(enabled_status.enabled);
        assert!(is_enabled().expect("Falha ao verificar is_enabled"));

        let curr_status = status().expect("Falha ao obter status apos enable");
        assert!(curr_status.enabled);

        // Desabilita
        let disabled_status = disable().expect("Falha ao desabilitar");
        assert!(!disabled_status.enabled);
        assert!(!is_enabled().expect("Falha ao verificar is_enabled apos disable"));

        // Restaura estado anterior caso já estivesse habilitado antes do teste
        if prev_status.enabled {
            if let Some(cmd) = prev_status.command {
                let _ = enable(Some(Path::new(&cmd)));
            }
        }
    }
}
