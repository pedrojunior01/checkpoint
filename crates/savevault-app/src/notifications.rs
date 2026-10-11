//! Módulo de Notificações Nativas do Windows Toast (TOAST-01).
//!
//! Permite que o SaveVault exiba notificações visuais elegantes e nativas no canto
//! inferior direito do Windows 10 e Windows 11 para eventos importantes, com controle
//! estrito de frequência (cooldown anti-spam) e respeito ao modo gamer/silencioso:
//! - Criação de novo snapshot de save (silencioso por padrão para não interromper jogos);
//! - Conclusão de sincronização em lote na nuvem (Google Drive / OneDrive);
//! - Sucesso na restauração de saves;
//! - Avisos de conectividade ou erros.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use savevault_core::config::NotificationsConfig;

#[cfg(windows)]
use winrt_notification::{Duration, Toast};

// Configurações globais em tempo de execução
static NOTIFICATIONS_ENABLED: AtomicBool = AtomicBool::new(true);
static NOTIFY_ON_SAVE: AtomicBool = AtomicBool::new(false); // Padrão: false para não spammar durante a jogatina
static NOTIFY_ON_SYNC: AtomicBool = AtomicBool::new(true);
static NOTIFY_ON_RESTORE: AtomicBool = AtomicBool::new(true);
static COOLDOWN_SECS: AtomicU64 = AtomicU64::new(300); // 5 minutos padrão

// Tabela de rate-limiting em memória: mapeia chave -> instante do último disparo
static LAST_NOTIFIED: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

/// Inicializa ou atualiza as preferências de notificações a partir do arquivo de configuração.
pub fn init_from_config(config: &NotificationsConfig) {
    NOTIFICATIONS_ENABLED.store(config.enabled, Ordering::SeqCst);
    NOTIFY_ON_SAVE.store(config.notify_on_save, Ordering::SeqCst);
    NOTIFY_ON_SYNC.store(config.notify_on_sync, Ordering::SeqCst);
    NOTIFY_ON_RESTORE.store(config.notify_on_restore, Ordering::SeqCst);
    COOLDOWN_SECS.store(config.cooldown_secs, Ordering::SeqCst);

    tracing::debug!(
        "Notificações inicializadas: enabled={}, notify_on_save={}, notify_on_sync={}, cooldown={}s",
        config.enabled,
        config.notify_on_save,
        config.notify_on_sync,
        config.cooldown_secs
    );
}

/// Verifica se um evento está em período de cooldown (anti-spam).
fn check_and_update_cooldown(key: &str, cooldown_secs: u64) -> bool {
    let mut lock = match LAST_NOTIFIED.lock() {
        Ok(guard) => guard,
        Err(_) => return true, // Em caso de envenenamento de lock, permite por segurança
    };

    let map = lock.get_or_insert_with(HashMap::new);
    let now = Instant::now();

    if let Some(&last_time) = map.get(key) {
        if now.duration_since(last_time).as_secs() < cooldown_secs {
            return false; // Ainda em cooldown: não notificar
        }
    }

    map.insert(key.to_string(), now);
    true
}

/// Exibe uma notificação nativa do Windows Toast de forma assíncrona/não-bloqueante.
pub fn notify(title: impl Into<String>, message: impl Into<String>) {
    if !NOTIFICATIONS_ENABLED.load(Ordering::Relaxed) {
        return;
    }

    let title = title.into();
    let message = message.into();

    #[cfg(windows)]
    {
        std::thread::Builder::new()
            .name("savevault-toast".into())
            .spawn(move || {
                let toast = Toast::new(Toast::POWERSHELL_APP_ID)
                    .title(&title)
                    .text1(&message)
                    .duration(Duration::Short);

                if let Err(e) = toast.show() {
                    tracing::debug!(
                        "Não foi possível exibir notificação do Windows Toast: {}",
                        e
                    );
                }
            })
            .ok();
    }

    #[cfg(not(windows))]
    {
        let _ = title;
        let _ = message;
    }
}

/// Notificação específica para quando um save de jogo for detectado e protegido localmente.
///
/// Por padrão permanece SILENCIOSA (`notify_on_save = false`) para evitar interrupções
/// visuais e sonoras enquanto o usuário joga. Quando ativada, aplica cooldown por jogo.
pub fn notify_save_protected(game_name: &str, version: u32) {
    if !NOTIFY_ON_SAVE.load(Ordering::Relaxed) {
        tracing::debug!(
            "Notificação Toast para save de '{}' v{} suprimida (notify_on_save desativado).",
            game_name,
            version
        );
        return;
    }

    let cooldown = COOLDOWN_SECS.load(Ordering::Relaxed);
    let key = format!("save:{}", game_name.to_lowercase());

    if !check_and_update_cooldown(&key, cooldown) {
        tracing::debug!(
            "Notificação de save para '{}' ignorada por cooldown anti-spam ({}s).",
            game_name,
            cooldown
        );
        return;
    }

    notify(
        "checkpoint — Save Protegido",
        format!(
            "O save de '{}' foi protegido com sucesso (v{}).",
            game_name, version
        ),
    );
}

/// Notificação específica para quando o lote de snapshots for sincronizado na nuvem.
/// Aplica cooldown global para evitar notificações repetidas em pequenos intervalos.
pub fn notify_cloud_synced(game_name: &str, version: u32, provider: &str) {
    if !NOTIFY_ON_SYNC.load(Ordering::Relaxed) {
        return;
    }

    let cooldown = COOLDOWN_SECS.load(Ordering::Relaxed);
    let key = format!("sync:{}", provider.to_lowercase());

    if !check_and_update_cooldown(&key, cooldown) {
        tracing::debug!("Notificação de sync na nuvem ignorada por cooldown anti-spam.");
        return;
    }

    let lower = provider.to_lowercase();
    let provider_label = match lower.as_str() {
        "onedrive" => "Microsoft OneDrive",
        "google_drive" | "google" | "drive" => "Google Drive",
        other => other,
    };

    notify(
        "checkpoint — Sincronização Concluída",
        format!(
            "Backup de '{}' (v{}) sincronizado na nuvem via {}.",
            game_name, version, provider_label
        ),
    );
}

/// Notificação específica para restauração concluída com sucesso (sempre emitida se ativo).
pub fn notify_restore_success(game_name: &str, version: u32) {
    if !NOTIFY_ON_RESTORE.load(Ordering::Relaxed) {
        return;
    }

    notify(
        "checkpoint — Restauração Concluída",
        format!(
            "O save de '{}' foi restaurado para a versão v{}.",
            game_name, version
        ),
    );
}

/// Notificação de aviso ou erro no checkpoint.
pub fn notify_warning(title: &str, message: &str) {
    let key = format!("warn:{}", title);
    if !check_and_update_cooldown(&key, 60) {
        return;
    }
    notify(format!("checkpoint — {}", title), message.to_string());
}

/// Notificação quando um conflito multi-PC for detectado entre computadores.
pub fn notify_conflict_detected(game_name: &str, detail: &str) {
    let key = format!("conflict:{}", game_name.to_lowercase());
    if !check_and_update_cooldown(&key, 120) {
        return;
    }

    notify(
        "checkpoint — Conflito Multi-PC Detectado",
        format!(
            "O save de '{}' divergiu entre dispositivos. {}",
            game_name, detail
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cooldown_rate_limiting() {
        let key = "test_game_cooldown";
        assert!(check_and_update_cooldown(key, 10));
        // Imediatamente após, deve ser bloqueado pelo cooldown
        assert!(!check_and_update_cooldown(key, 10));
    }

    #[test]
    fn test_notify_does_not_panic() {
        let config = NotificationsConfig {
            enabled: true,
            notify_on_save: true,
            notify_on_sync: true,
            notify_on_restore: true,
            cooldown_secs: 1,
        };
        init_from_config(&config);

        notify("checkpoint Teste", "Notificação de teste automatizado");
        notify_save_protected("Teste Game Rate Limit", 1);
        notify_cloud_synced("Teste Game Rate Limit", 1, "google_drive");
        notify_restore_success("Teste Game Rate Limit", 1);
        notify_warning("Aviso", "Mensagem de teste");
    }
}
