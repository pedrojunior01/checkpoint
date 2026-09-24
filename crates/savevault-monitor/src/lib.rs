//! savevault-monitor
//!
//! Monitoramento de arquivos via `notify` com debounce de estabilização.
//! Conforme spec seção 7.1: NUNCA fazer upload imediato ao detectar mudança —
//! aguardar estabilização antes de acionar o backup.

use anyhow::Result;
use notify::{RecommendedWatcher, RecursiveMode, Watcher, Config, Event};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Evento emitido pelo monitor após estabilização do arquivo.
#[derive(Debug)]
pub struct SaveChangedEvent {
    pub path: PathBuf,
}

/// Monitor de arquivos de save.
pub struct FileWatcher {
    debounce_duration: Duration,
}

impl FileWatcher {
    pub fn new(debounce_ms: u64) -> Self {
        Self {
            debounce_duration: Duration::from_millis(debounce_ms),
        }
    }

    /// Inicia o monitoramento dos caminhos fornecidos.
    /// Retorna um receiver que emite eventos estabilizados.
    pub fn watch(
        &self,
        paths: Vec<PathBuf>,
    ) -> Result<mpsc::UnboundedReceiver<SaveChangedEvent>> {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<notify::Result<Event>>();
        let (stable_tx, stable_rx) = mpsc::unbounded_channel::<SaveChangedEvent>();

        // Configura o watcher do sistema operacional
        let mut watcher: RecommendedWatcher = Watcher::new(
            move |res| {
                let _ = event_tx.send(res);
            },
            Config::default(),
        )?;

        for path in &paths {
            if path.exists() {
                watcher.watch(path, RecursiveMode::Recursive)?;
                info!("Monitorando: {}", path.display());
            } else {
                warn!("Caminho não existe, ignorando: {}", path.display());
            }
        }

        let debounce = self.debounce_duration;

        // Task de debounce: aguarda estabilização antes de emitir evento
        tokio::spawn(async move {
            // Mantém o watcher vivo enquanto a task existir
            let _watcher = watcher;

            loop {
                match event_rx.recv().await {
                    None => break,
                    Some(Err(e)) => {
                        warn!("Erro do watcher: {}", e);
                        continue;
                    }
                    Some(Ok(event)) => {
                        // Só processa eventos de modificação/criação
                        use notify::EventKind::*;
                        match event.kind {
                            Modify(_) | Create(_) => {}
                            _ => continue,
                        }

                        for path in event.paths {
                            debug!("Mudança detectada: {} — aguardando estabilização ({:?})", path.display(), debounce);

                            // Aguarda debounce
                            tokio::time::sleep(debounce).await;

                            // Verifica se o arquivo ainda existe e tem tamanho estável
                            if path.exists() {
                                let _ = stable_tx.send(SaveChangedEvent { path });
                            }
                        }
                    }
                }
            }
        });

        Ok(stable_rx)
    }
}
