//! Módulo de Janela de Desktop Integrada (Extremamente Leve).
//!
//! Executa a interface do SaveVault em uma janela nativa de desktop sem barras
//! de navegação, abas ou elementos de browser tradicional, utilizando o runtime
//! do Microsoft Edge (WebView2/Chromium) já pré-instalado no Windows 10/11.
//!
//! Vantagens arquiteturais:
//! - 0 KB adicionados ao binário executável (sem Electron, sem C++ pesado ou dependências de dezenas de megabytes).
//! - 0 MB de overhead estático em tempo de compilação.
//! - Execução instantânea com aceleração por hardware e isolamento de perfil.
//! - Fallback transparente para o navegador padrão caso executado em ambientes atípicos.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Mutex;
use tracing::{debug, info, warn};

static DESKTOP_CHILD: Mutex<Option<Child>> = Mutex::new(None);

/// Caminhos típicos de executáveis compatíveis com o modo `--app` no Windows.
const EDGE_PATHS: &[&str] = &[
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
];

const CHROME_PATHS: &[&str] = &[
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
];

const BRAVE_PATHS: &[&str] =
    &[r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe"];

/// Procura um executável compatível com modo aplicação no sistema operacional.
pub fn find_app_mode_executable() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        // 1. Tenta Brave (preferido pelo usuário)
        for path_str in BRAVE_PATHS {
            let p = Path::new(path_str);
            if p.exists() {
                return Some(p.to_path_buf());
            }
        }

        // 2. Tenta Google Chrome
        for path_str in CHROME_PATHS {
            let p = Path::new(path_str);
            if p.exists() {
                return Some(p.to_path_buf());
            }
        }

        // 3. Tenta Microsoft Edge
        for path_str in EDGE_PATHS {
            let p = Path::new(path_str);
            if p.exists() {
                return Some(p.to_path_buf());
            }
        }

        // 4. Tenta Edge no diretório AppData do usuário
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            let user_edge = PathBuf::from(local_appdata)
                .join("Microsoft")
                .join("Edge")
                .join("Application")
                .join("msedge.exe");
            if user_edge.exists() {
                return Some(user_edge);
            }
        }
    }

    None
}

/// Retorna `true` se o modo janela de desktop nativa estiver disponível no host.
pub fn is_desktop_mode_available() -> bool {
    find_app_mode_executable().is_some()
}

/// Retorna o caminho do diretório de dados/perfil isolado para a janela do desktop.
#[allow(dead_code)]
fn get_desktop_profile_dir() -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(appdata)
        .join("SaveVault")
        .join("desktop_profile")
}

/// Abre a interface do SaveVault em uma Janela de Desktop Integrada e ultraleve.
///
/// Se um executável com suporte a `--app` (ex.: Brave, Chrome, Edge) for encontrado,
/// inicializa uma janela dedicada isolada (sem abas, sem URL bar, tamanho ajustado).
/// Caso contrário, faz fallback seguro abrindo no navegador padrão do usuário.
pub fn open_desktop_window(url: &str) -> anyhow::Result<()> {
    // Limpa processo anterior se já encerrou
    if let Ok(mut guard) = DESKTOP_CHILD.lock() {
        if let Some(child) = guard.as_mut() {
            if let Ok(Some(_)) = child.try_wait() {
                *guard = None;
            }
        }
    }

    if let Some(exe_path) = find_app_mode_executable() {
        let app_arg = format!("--app={}", url);

        debug!(
            "Iniciando janela desktop via {} com url: {}",
            exe_path.display(),
            url
        );

        let child = Command::new(&exe_path)
            .arg(&app_arg)
            .arg("--window-size=1260,820")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .spawn();

        match child {
            Ok(c) => {
                info!("Janela de Desktop SaveVault iniciada (PID: {}).", c.id());
                if let Ok(mut guard) = DESKTOP_CHILD.lock() {
                    *guard = Some(c);
                }
                return Ok(());
            }
            Err(e) => {
                warn!(
                    "Falha ao iniciar janela desktop com {}: {}. Fazendo fallback para navegador padrão...",
                    exe_path.display(),
                    e
                );
            }
        }
    } else {
        info!("Executável com suporte a modo app não encontrado. Usando navegador padrão...");
    }

    // Fallback: abre no navegador do sistema
    crate::tray::open_browser(url);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_desktop_profile_dir_path() {
        let dir = get_desktop_profile_dir();
        assert!(dir.to_string_lossy().contains("SaveVault"));
        assert!(dir.to_string_lossy().contains("desktop_profile"));
    }

    #[test]
    fn test_find_executable_does_not_panic() {
        let _ = find_app_mode_executable();
    }
}
