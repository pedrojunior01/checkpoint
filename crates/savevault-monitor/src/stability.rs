//! Verificador de estabilidade de arquivos.
//!
//! Garante que um arquivo modificado ou criado terminou de ser gravado pela aplicação
//! do jogo antes de emitir um evento estável.
//!
//! Verifica:
//! 1. Tamanho e timestamp consistentes em múltiplas observações;
//! 2. Ausência de bloqueios exclusivos (`ERROR_SHARING_VIOLATION`) com retry e backoff controlado;
//! 3. Desaparecimento de arquivos temporários sem pânico.

use std::path::Path;
use std::time::Duration;
use crate::file_access::{FileAccess, FileAccessError, FileMetadata};

/// Configurações do verificador de estabilidade.
#[derive(Debug, Clone)]
pub struct StabilityConfig {
    /// Quantidade de verificações consecutivas idênticas necessárias.
    pub required_stable_checks: usize,
    /// Intervalo entre verificações de tamanho/timestamp.
    pub check_interval: Duration,
    /// Intervalo base de backoff ao encontrar sharing violation.
    pub initial_backoff: Duration,
    /// Fator de multiplicação exponencial para o backoff.
    pub backoff_multiplier: f64,
    /// Limite máximo de cada backoff.
    pub max_backoff: Duration,
    /// Quantidade máxima de tentativas totais.
    pub max_attempts: usize,
}

impl Default for StabilityConfig {
    fn default() -> Self {
        Self {
            required_stable_checks: 2,
            check_interval: Duration::from_millis(100),
            initial_backoff: Duration::from_millis(150),
            backoff_multiplier: 1.5,
            max_backoff: Duration::from_millis(1000),
            max_attempts: 10,
        }
    }
}

impl StabilityConfig {
    /// Configuração rápida com timeouts curtos para testes determinísticos.
    pub fn fast_for_tests() -> Self {
        Self {
            required_stable_checks: 2,
            check_interval: Duration::from_millis(5),
            initial_backoff: Duration::from_millis(10),
            backoff_multiplier: 1.2,
            max_backoff: Duration::from_millis(50),
            max_attempts: 8,
        }
    }
}

use std::sync::atomic::{AtomicBool, Ordering};

/// Resultado da verificação de estabilidade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StabilityResult {
    /// Arquivo estabilizado com sucesso, desbloqueado e acessível para leitura.
    Stable(FileMetadata),
    /// O arquivo desapareceu durante a estabilização (comum em arquivos temporários de atomic save).
    Disappeared,
    /// Acesso negado permanentemente (ACL / permissão).
    PermissionDenied,
    /// O arquivo não estabilizou (tamanho continuou mudando) dentro do limite de tentativas.
    Timeout { attempts: usize },
    /// Violação de compartilhamento persistiu após todos os retries.
    SharingViolationTimeout { attempts: usize },
    /// Operação cancelada por nova atividade no mesmo caminho ou encerramento do monitor.
    Cancelled,
}

/// Mecanismo assíncrono para validação de estabilidade de arquivos.
pub struct StabilityChecker {
    config: StabilityConfig,
}

impl StabilityChecker {
    pub fn new(config: StabilityConfig) -> Self {
        Self { config }
    }

    /// Executa o ciclo de verificação de estabilidade sobre um caminho.
    pub async fn check_stability(
        &self,
        path: &Path,
        access: &dyn FileAccess,
    ) -> StabilityResult {
        self.check_stability_with_cancel(path, access, None).await
    }

    /// Executa o ciclo de verificação de estabilidade suportando cancelamento explícito via flag atômica.
    pub async fn check_stability_with_cancel(
        &self,
        path: &Path,
        access: &dyn FileAccess,
        cancel_token: Option<&AtomicBool>,
    ) -> StabilityResult {
        let is_cancelled = || cancel_token.map(|t| t.load(Ordering::SeqCst)).unwrap_or(false);

        let mut attempts = 0;
        let mut consecutive_stable = 0;
        let mut last_metadata: Option<FileMetadata> = None;
        let mut current_backoff = self.config.initial_backoff;
        let mut last_was_sharing_violation = false;

        while attempts < self.config.max_attempts {
            if is_cancelled() {
                return StabilityResult::Cancelled;
            }

            attempts += 1;

            // 1. Tentar verificar se o arquivo está desbloqueado para leitura
            if let Err(err) = access.verify_readable(path) {
                match err {
                    FileAccessError::NotFound => {
                        return StabilityResult::Disappeared;
                    }
                    FileAccessError::PermissionDenied => {
                        tracing::warn!(
                            "Acesso negado permanentemente ao arquivo '{}' (ACL / permissão). Abortando verificação.",
                            path.display()
                        );
                        return StabilityResult::PermissionDenied;
                    }
                    FileAccessError::SharingViolation => {
                        tracing::debug!(
                            "Sharing violation em '{}' (tentativa {}/{}), aguardando backoff {:?}",
                            path.display(), attempts, self.config.max_attempts, current_backoff
                        );
                        last_was_sharing_violation = true;
                        consecutive_stable = 0;
                        if sleep_with_cancel(current_backoff, cancel_token).await {
                            return StabilityResult::Cancelled;
                        }
                        current_backoff = Duration::from_secs_f64(
                            (current_backoff.as_secs_f64() * self.config.backoff_multiplier)
                                .min(self.config.max_backoff.as_secs_f64()),
                        );
                        continue;
                    }
                    FileAccessError::Other(e) => {
                        tracing::debug!("Erro transitório ao testar leitura em '{}': {}", path.display(), e);
                        consecutive_stable = 0;
                        if sleep_with_cancel(self.config.check_interval, cancel_token).await {
                            return StabilityResult::Cancelled;
                        }
                        continue;
                    }
                }
            }

            if is_cancelled() {
                return StabilityResult::Cancelled;
            }

            last_was_sharing_violation = false;

            // 2. Obter metadados de tamanho e modificado
            let meta = match access.metadata(path) {
                Ok(m) => m,
                Err(FileAccessError::NotFound) => return StabilityResult::Disappeared,
                Err(FileAccessError::PermissionDenied) => return StabilityResult::PermissionDenied,
                Err(FileAccessError::SharingViolation) => {
                    last_was_sharing_violation = true;
                    consecutive_stable = 0;
                    if sleep_with_cancel(current_backoff, cancel_token).await {
                        return StabilityResult::Cancelled;
                    }
                    continue;
                }
                Err(_) => {
                    consecutive_stable = 0;
                    if sleep_with_cancel(self.config.check_interval, cancel_token).await {
                        return StabilityResult::Cancelled;
                    }
                    continue;
                }
            };

            if is_cancelled() {
                return StabilityResult::Cancelled;
            }

            // 3. Comparar com observação anterior
            if let Some(ref prev) = last_metadata {
                if meta.size_bytes == prev.size_bytes && meta.modified_at == prev.modified_at {
                    consecutive_stable += 1;
                    if consecutive_stable >= self.config.required_stable_checks {
                        return StabilityResult::Stable(meta);
                    }
                } else {
                    tracing::debug!(
                        "Arquivo '{}' ainda em mutação (tamanho anterior: {}, atual: {})",
                        path.display(), prev.size_bytes, meta.size_bytes
                    );
                    consecutive_stable = 0;
                }
            } else {
                // Primeira observação registrada
                consecutive_stable = 0;
            }

            last_metadata = Some(meta);
            if sleep_with_cancel(self.config.check_interval, cancel_token).await {
                return StabilityResult::Cancelled;
            }
        }

        if is_cancelled() {
            return StabilityResult::Cancelled;
        }

        if last_was_sharing_violation {
            StabilityResult::SharingViolationTimeout { attempts }
        } else {
            StabilityResult::Timeout { attempts }
        }
    }
}

/// Auxiliar assíncrono para dormir por um período especificado, retornando imediatamente se o token for cancelado.
async fn sleep_with_cancel(duration: Duration, cancel_token: Option<&AtomicBool>) -> bool {
    let start = tokio::time::Instant::now();
    let step = Duration::from_millis(10);
    while start.elapsed() < duration {
        if cancel_token.map(|t| t.load(Ordering::SeqCst)).unwrap_or(false) {
            return true;
        }
        let remaining = duration.saturating_sub(start.elapsed());
        tokio::time::sleep(remaining.min(step)).await;
    }
    cancel_token.map(|t| t.load(Ordering::SeqCst)).unwrap_or(false)
}

