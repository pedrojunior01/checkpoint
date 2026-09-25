//! Coalescing e debounce assíncrono de eventos de arquivos.
//!
//! Agrupa múltiplos eventos rápidos disparados pelo sistema operacional sobre o mesmo arquivo
//! em uma única unidade de processamento (`CoalescedFileEvent`), sem bloquear o event loop com `sleep()`,
//! com limites estritos de memória e suporte a cancelamento de stability checks em voo.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use uuid::Uuid;

use crate::events::{CoalescedFileEvent, RawFileEvent, RawFileEventKind};

#[derive(Clone)]
struct PathGenerationState {
    generation: u64,
    cancel_token: Arc<AtomicBool>,
}

type InFlightMap = Arc<Mutex<HashMap<PathBuf, PathGenerationState>>>;

/// Rastreador de cancelamento e gerações de tarefas de estabilidade ativas.
#[derive(Clone, Default)]
pub struct InFlightTracker {
    inner: InFlightMap,
}

impl InFlightTracker {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Registra um novo evento para o caminho.
    /// Incrementa a geração e cancela qualquer token ativo da geração anterior.
    /// Retorna a nova geração.
    pub fn register_event(&self, path: &Path) -> u64 {
        let mut map = self.inner.lock().unwrap();
        if let Some(old) = map.get(path) {
            old.cancel_token.store(true, Ordering::SeqCst);
        }
        let current_gen = map.get(path).map(|s| s.generation).unwrap_or(0);
        let next_gen = current_gen + 1;
        let new_cancel = Arc::new(AtomicBool::new(false));
        map.insert(
            path.to_path_buf(),
            PathGenerationState {
                generation: next_gen,
                cancel_token: new_cancel,
            },
        );
        next_gen
    }

    /// Retorna o token de cancelamento para o caminho, se existir.
    pub fn get_cancel_token(&self, path: &Path) -> Option<Arc<AtomicBool>> {
        let map = self.inner.lock().unwrap();
        map.get(path).map(|s| s.cancel_token.clone())
    }

    /// Verifica se a geração fornecida ainda é a mais recente para o caminho e não foi cancelada.
    pub fn is_current(&self, path: &Path, generation: u64) -> bool {
        let map = self.inner.lock().unwrap();
        if let Some(s) = map.get(path) {
            s.generation == generation && !s.cancel_token.load(Ordering::SeqCst)
        } else {
            false
        }
    }

    /// Cancela uma tarefa em andamento para o caminho.
    pub fn cancel_path(&self, path: &Path) {
        let mut map = self.inner.lock().unwrap();
        if let Some(s) = map.remove(path) {
            s.cancel_token.store(true, Ordering::SeqCst);
        }
    }

    /// Cancela todas as tarefas de estabilidade em voo (utilizado no shutdown).
    pub fn cancel_all(&self) {
        let mut map = self.inner.lock().unwrap();
        for (_, s) in map.drain() {
            s.cancel_token.store(true, Ordering::SeqCst);
        }
    }
}

/// Configurações para a etapa de debounce e coalescing.
#[derive(Debug, Clone)]
pub struct DebounceConfig {
    /// Janela de silêncio necessária após a última alteração para liberar o evento.
    pub debounce_duration: Duration,
    /// Tempo máximo de espera total desde o primeiro evento para evitar inanição em streams contínuos.
    pub max_wait_duration: Duration,
    /// Capacidade do canal limitado (bounded) de saída para eventos coalescidos.
    pub channel_capacity: usize,
    /// Quantidade máxima de caminhos mantidos simultaneamente na memória do debouncer.
    pub max_pending_paths: usize,
}

impl Default for DebounceConfig {
    fn default() -> Self {
        Self {
            debounce_duration: Duration::from_millis(300),
            max_wait_duration: Duration::from_millis(2000),
            channel_capacity: 256,
            max_pending_paths: 5000,
        }
    }
}

impl DebounceConfig {
    /// Configuração acelerada para testes determinísticos sem longas pausas.
    pub fn fast_for_tests() -> Self {
        Self {
            debounce_duration: Duration::from_millis(20),
            max_wait_duration: Duration::from_millis(100),
            channel_capacity: 256,
            max_pending_paths: 50,
        }
    }
}

/// Estado pendente mantido por caminho monitorado.
#[derive(Debug, Clone)]
pub struct PendingPathState {
    pub save_location_id: Uuid,
    pub first_seen: Instant,
    pub last_event: Instant,
    pub latest_kind: RawFileEventKind,
    pub deadline: Instant,
    pub generation: u64,
}

/// Mecanismo assíncrono de debounce sem bloqueio do event loop.
pub struct FileDebouncer {
    config: DebounceConfig,
    in_flight_tracker: InFlightTracker,
    overflow_counter: Arc<AtomicUsize>,
}

impl FileDebouncer {
    pub fn new(config: DebounceConfig) -> Self {
        Self {
            config,
            in_flight_tracker: InFlightTracker::new(),
            overflow_counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn with_tracker(config: DebounceConfig, tracker: InFlightTracker) -> Self {
        Self {
            config,
            in_flight_tracker: tracker,
            overflow_counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Retorna o contador de eventos descartados por overflow de memória.
    pub fn overflow_count(&self) -> usize {
        self.overflow_counter.load(Ordering::SeqCst)
    }

    /// Retorna referência ao contador atômico de overflow.
    pub fn overflow_counter(&self) -> Arc<AtomicUsize> {
        self.overflow_counter.clone()
    }

    /// Retorna o rastreador de tarefas em voo (InFlightTracker).
    pub fn tracker(&self) -> InFlightTracker {
        self.in_flight_tracker.clone()
    }

    /// Inicia a task de debounce e retorna o canal receptor de eventos coalescidos e o JoinHandle da task.
    pub fn start(
        self,
        mut raw_rx: mpsc::Receiver<RawFileEvent>,
        mut shutdown_rx: watch::Receiver<bool>,
    ) -> (mpsc::Receiver<CoalescedFileEvent>, tokio::task::JoinHandle<()>) {
        let (output_tx, output_rx) = mpsc::channel(self.config.channel_capacity);
        let config = self.config;
        let tracker = self.in_flight_tracker;
        let overflow_counter = self.overflow_counter;

        let handle = tokio::spawn(async move {
            let mut pending: HashMap<PathBuf, PendingPathState> = HashMap::new();
            let mut ready_queue: VecDeque<CoalescedFileEvent> = VecDeque::new();
            let mut next_deadline: Option<Instant> = None;

            loop {
                // 1. Drenar caminhos que atingiram a deadline para a fila de prontos
                let now = Instant::now();
                let mut ready_keys = Vec::new();

                for (path, state) in &pending {
                    if state.deadline <= now {
                        ready_keys.push(path.clone());
                    }
                }

                if !ready_keys.is_empty() {
                    for path in ready_keys {
                        if let Some(state) = pending.remove(&path) {
                            ready_queue.push_back(CoalescedFileEvent {
                                path,
                                save_location_id: state.save_location_id,
                                kind: state.latest_kind,
                                generation: state.generation,
                            });
                        }
                    }
                    // Recalcula próxima deadline após remoção
                    next_deadline = pending.values().map(|s| s.deadline).min();
                }

                // 2. Select totalmente não-bloqueante
                tokio::select! {
                    biased;

                    // Sinal de shutdown
                    changed = shutdown_rx.changed() => {
                        if changed.is_err() || *shutdown_rx.borrow() {
                            break;
                        }
                    }

                    // Envio de itens prontos para o canal de saída sob backpressure não-bloqueante
                    send_res = async {
                        if let Some(front) = ready_queue.front() {
                            output_tx.send(front.clone()).await
                        } else {
                            std::future::pending::<Result<(), mpsc::error::SendError<CoalescedFileEvent>>>().await
                        }
                    }, if !ready_queue.is_empty() => {
                        match send_res {
                            Ok(()) => {
                                ready_queue.pop_front();
                            }
                            Err(_) => {
                                // Receptor fechado: finalizar task
                                return;
                            }
                        }
                    }

                    // Recepção imediata de novos eventos brutos
                    event_opt = raw_rx.recv() => {
                        match event_opt {
                            Some(raw_event) => {
                                update_pending_state(
                                    &mut pending,
                                    &mut ready_queue,
                                    raw_event,
                                    &config,
                                    &tracker,
                                    &overflow_counter,
                                    &mut next_deadline,
                                );
                            }
                            None => {
                                // Canal de entrada fechado.
                                // Se não há mais nada pendente nem na fila, finalizar.
                                if pending.is_empty() && ready_queue.is_empty() {
                                    break;
                                }
                            }
                        }
                    }

                    // Timer de expiração da deadline mais próxima
                    _ = async {
                        if let Some(deadline) = next_deadline {
                            tokio::time::sleep_until(deadline).await;
                        } else {
                            std::future::pending::<()>().await;
                        }
                    } => {
                        // Timer disparou; próximo ciclo drenará os caminhos prontos
                    }
                }
            }

            // Drenagem final no shutdown
            let now = Instant::now();
            for (path, state) in pending {
                if state.deadline <= now {
                    let _ = output_tx.send(CoalescedFileEvent {
                        path,
                        save_location_id: state.save_location_id,
                        kind: state.latest_kind,
                        generation: state.generation,
                    }).await;
                }
            }
        });

        (output_rx, handle)
    }
}

/// Atualiza ou insere o estado pendente de um caminho com regras de coalescing,
/// respeitando estritamente o limite de memória `max_pending_paths` e cancelando verificações obsoletas.
fn update_pending_state(
    pending: &mut HashMap<PathBuf, PendingPathState>,
    ready_queue: &mut VecDeque<CoalescedFileEvent>,
    event: RawFileEvent,
    config: &DebounceConfig,
    tracker: &InFlightTracker,
    overflow_counter: &Arc<AtomicUsize>,
    next_deadline: &mut Option<Instant>,
) {
    let now = Instant::now();

    // Registra nova geração e cancela tarefas de estabilidade em andamento para o mesmo caminho
    let generation = tracker.register_event(&event.path);

    // Se o caminho já estava na ready_queue aguardando envio sob backpressure, retira e consolida de volta em pending
    if let Some(pos) = ready_queue.iter().position(|ev| ev.path == event.path) {
        let queued = ready_queue.remove(pos).unwrap();
        let deadline = now + config.debounce_duration;
        pending.insert(
            event.path,
            PendingPathState {
                save_location_id: queued.save_location_id,
                first_seen: now,
                last_event: now,
                latest_kind: coalesce_kinds(queued.kind, event.kind),
                deadline,
                generation,
            },
        );
        *next_deadline = Some(next_deadline.map_or(deadline, |d| d.min(deadline)));
        return;
    }

    if let Some(entry) = pending.get_mut(&event.path) {
        entry.last_event = now;
        entry.latest_kind = coalesce_kinds(entry.latest_kind, event.kind);
        entry.generation = generation;

        let proposed = now + config.debounce_duration;
        let ceiling = entry.first_seen + config.max_wait_duration;
        entry.deadline = proposed.min(ceiling);

        *next_deadline = Some(next_deadline.map_or(entry.deadline, |d| d.min(entry.deadline)));
    } else {
        // Verifica limite de capacidade de memória
        if pending.len() >= config.max_pending_paths {
            overflow_counter.fetch_add(1, Ordering::SeqCst);
            tracing::warn!(
                "Debouncer atingiu a capacidade máxima de caminhos pendentes ({}). Evento descartado sob overflow para: {}",
                config.max_pending_paths,
                event.path.display()
            );
            return;
        }

        let deadline = now + config.debounce_duration;

        pending.insert(
            event.path,
            PendingPathState {
                save_location_id: event.save_location_id,
                first_seen: now,
                last_event: now,
                latest_kind: event.kind,
                deadline,
                generation,
            },
        );

        *next_deadline = Some(next_deadline.map_or(deadline, |d| d.min(deadline)));
    }
}

/// Regras de combinação lógica de eventos sucessivos sobre um mesmo arquivo.
fn coalesce_kinds(prev: RawFileEventKind, next: RawFileEventKind) -> RawFileEventKind {
    match (prev, next) {
        (RawFileEventKind::Created, RawFileEventKind::Modified) => RawFileEventKind::Created,
        (_, RawFileEventKind::Removed) => RawFileEventKind::Removed,
        (RawFileEventKind::Removed, RawFileEventKind::Created) => RawFileEventKind::Modified,
        (_, new_kind) => new_kind,
    }
}
