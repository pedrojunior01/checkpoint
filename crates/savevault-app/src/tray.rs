//! Módulo de integração com a Bandeja do Sistema (System Tray) e gerenciamento de console no Windows.
//!
//! Permite que o SaveVault execute em segundo plano (background) com a janela do console
//! oculta, exibindo um ícone na área de notificação do Windows com menu de contexto para:
//! - Abrir a interface web (DEV-UI)
//! - Alternar inicialização automática com o Windows
//! - Exibir / Ocultar a janela do console
//! - Encerrar a aplicação de forma graciosa

#[cfg(windows)]
use tray_item::{IconSource, TrayItem};

/// Manipulador do ciclo de vida do System Tray.
pub struct TrayHandle {
    #[cfg(windows)]
    _tray: TrayItem,
}

/// Oculta a janela de console do processo atual (Windows).
pub fn hide_console_window() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::GetConsoleWindow;
        use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};

        let hwnd = GetConsoleWindow();
        if hwnd != 0 {
            ShowWindow(hwnd, SW_HIDE);
        }
    }
}

/// Exibe e restaura a janela de console do processo atual (Windows).
pub fn show_console_window() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::GetConsoleWindow;
        use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOW};

        let hwnd = GetConsoleWindow();
        if hwnd != 0 {
            ShowWindow(hwnd, SW_SHOW);
        }
    }
}

/// Retorna `true` se a janela do console estiver atualmente visível.
pub fn is_console_visible() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::GetConsoleWindow;
        use windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible;

        let hwnd = GetConsoleWindow();
        if hwnd != 0 {
            IsWindowVisible(hwnd) != 0
        } else {
            false
        }
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// Alterna a visibilidade da janela do console (exibe se estiver oculta, oculta se estiver visível).
pub fn toggle_console_window() {
    #[cfg(windows)]
    {
        if is_console_visible() {
            hide_console_window();
        } else {
            show_console_window();
        }
    }
}

/// Abre o navegador padrão na URL especificada.
pub fn open_browser(url: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

/// Inicializa o ícone da bandeja do sistema com menu de contexto.
///
/// Retorna `Some(TrayHandle)` se criado com sucesso, ou `None` em caso de erro ou em plataformas não-Windows.
pub fn setup_tray(
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    ui_port: u16,
) -> Option<TrayHandle> {
    #[cfg(windows)]
    {
        let title = format!("SaveVault v{}", env!("CARGO_PKG_VERSION"));
        let mut tray = match TrayItem::new(&title, IconSource::Resource("tray-default")) {
            Ok(t) => t,
            Err(_) => match TrayItem::new(&title, IconSource::Resource("1")) {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!("Falha ao inicializar System Tray: {}", e);
                    return None;
                }
            },
        };

        // Rótulo de título / versão
        let _ = tray.add_label(&title);
        let _ = tray.add_label("✓ Status: Protegido");

        // Ação: Abrir Janela do SaveVault (Desktop Integrada)
        let web_url = format!("http://127.0.0.1:{}", ui_port);
        let url_clone = web_url.clone();
        let _ = tray.add_menu_item("🖥️ Abrir SaveVault", move || {
            let _ = crate::desktop::open_desktop_window(&url_clone);
        });

        // Ação: Ver Jogos
        let url_games = format!("http://127.0.0.1:{}#jogos", ui_port);
        let _ = tray.add_menu_item("🎮 Ver Jogos", move || {
            let _ = crate::desktop::open_desktop_window(&url_games);
        });

        // Ação: Sincronizar / Verificar
        let url_sync = format!("http://127.0.0.1:{}#sync", ui_port);
        let _ = tray.add_menu_item("🔄 Sincronizar / Verificar", move || {
            let _ = crate::desktop::open_desktop_window(&url_sync);
        });

        // Ação: Configurações
        let url_config = format!("http://127.0.0.1:{}#config", ui_port);
        let _ = tray.add_menu_item("⚙️ Configurações", move || {
            let _ = crate::desktop::open_desktop_window(&url_config);
        });

        // Ação: Alternar Console (Exibir/Ocultar)
        let _ = tray.add_menu_item("💻 Exibir / Ocultar Console", move || {
            toggle_console_window();
        });

        // Ação: Alternar Início Automático com o Windows
        let _ = tray.add_menu_item("🔄 Iniciar com o Windows (Alternar)", move || {
            match savevault_core::autostart::status() {
                Ok(status) => {
                    if status.enabled {
                        let _ = savevault_core::autostart::disable();
                        tracing::info!("Autostart desabilitado via System Tray.");
                    } else {
                        let _ = savevault_core::autostart::enable(None);
                        tracing::info!("Autostart habilitado via System Tray.");
                    }
                }
                Err(e) => {
                    tracing::error!("Erro ao alternar autostart via Tray: {}", e);
                }
            }
        });

        // Ação: Sair da Aplicação
        let shutdown_tx_clone = shutdown_tx.clone();
        let _ = tray.add_menu_item("❌ Sair do SaveVault", move || {
            tracing::info!("Comando de encerramento acionado pelo System Tray.");
            let _ = shutdown_tx_clone.send(true);
        });

        tracing::info!("System Tray inicializado com sucesso.");
        Some(TrayHandle { _tray: tray })
    }

    #[cfg(not(windows))]
    {
        let _ = shutdown_tx;
        let _ = ui_port;
        None
    }
}
