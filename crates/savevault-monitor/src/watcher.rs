//! Serviço de monitoramento de arquivos (File Watcher).
//!
//! Integra:
//! 1. Detecção não-bloqueante de eventos do sistema operacional (`notify`);
//! 2. Suporte dinâmico a diretórios criados/recriados após o startup;
//! 3. Filtragem contra `SaveLocation`s configurados (incluindo subdiretórios em globs e proteção contra traversal);
//! 4. Coalescing e debounce assíncrono limitado em memória com backpressure (`FileDebouncer`);
//! 5. Verificação de estabilidade física, rastreamento de tarefas com JoinSet, eliminação de race conditions
//!    por geração e tratamento correto de `ERROR_SHARING_VIOLATION` vs `ERROR_ACCESS_DENIED`;
//! 6. Emissão controlada de `StableFileEvent` com controle estrito de concorrência (`Semaphore`) e shutdown limpo.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use thiserror::Error;
use tokio::sync::{mpsc, watch, Semaphore};
use uuid::Uuid;

use savevault_core::models::{SaveLocation, SaveLocationType};
use savevault_core::path::{
    canonicalize_windows_path, is_ignored_file_extension, is_ignored_relative_save_path,
    is_internal_savevault_file,
};

use crate::debouncer::{DebounceConfig, FileDebouncer, InFlightTracker};
use crate::events::{CoalescedFileEvent, RawFileEvent, RawFileEventKind, StableFileEvent, StableFileEventKind};
use crate::file_access::FileAccess;
use crate::scanner::{match_glob_relative, split_glob_base_and_pattern, SaveScanner};
use crate::stability::{StabilityChecker, StabilityConfig, StabilityResult};

#[derive(Debug, Error)]
pub enum WatcherError {
    #[error("Erro no watcher do sistema operacional (notify): {0}")]
    Notify(#[from] notify::Error),

    #[error("Erro de I/O no monitor: {0}")]
    Io(#[from] std::io::Error),
}

/// Configurações do serviço de monitoramento.
#[derive(Debug, Clone)]
pub struct SaveWatcherConfig {
    pub debounce: DebounceConfig,
    pub stability: StabilityConfig,
    pub max_concurrent_stability_checks: usize,
    pub raw_channel_capacity: usize,
    pub stable_channel_capacity: usize,
}

impl Default for SaveWatcherConfig {
    fn default() -> Self {
        Self {
            debounce: DebounceConfig::default(),
            stability: StabilityConfig::default(),
            max_concurrent_stability_checks: 8,
            raw_channel_capacity: 1024,
            stable_channel_capacity: 256,
        }
    }
}

impl SaveWatcherConfig {
    /// Configuração para testes rápidos e determinísticos.
    pub fn fast_for_tests() -> Self {
        Self {
            debounce: DebounceConfig::fast_for_tests(),
            stability: StabilityConfig::fast_for_tests(),
            max_concurrent_stability_checks: 4,
            raw_channel_capacity: 64,
            stable_channel_capacity: 64,
        }
    }
}

/// Alvo desejado de monitoramento para um `SaveLocation`.
#[derive(Debug, Clone)]
pub struct WatchTarget {
    pub desired_path: PathBuf,
    pub mode: RecursiveMode,
}

/// Serviço de monitoramento e estabilização de arquivos de save.
pub struct SaveWatcherService {
    shutdown_tx: watch::Sender<bool>,
    task_handles: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    _watcher: Arc<Mutex<Option<RecommendedWatcher>>>,
    locations: Vec<SaveLocation>,
    raw_overflow_counter: Arc<AtomicUsize>,
    debouncer_overflow_counter: Arc<AtomicUsize>,
    in_flight_tracker: InFlightTracker,
}

impl SaveWatcherService {
    /// Inicia o monitoramento com o watcher real do sistema operacional (`notify`).
    pub fn start(
        config: SaveWatcherConfig,
        locations: Vec<SaveLocation>,
        file_access: Arc<dyn FileAccess>,
    ) -> Result<(Self, mpsc::Receiver<StableFileEvent>), WatcherError> {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (raw_tx, raw_rx) = mpsc::channel(config.raw_channel_capacity);
        let (stable_tx, stable_rx) = mpsc::channel(config.stable_channel_capacity);

        let raw_overflow_counter = Arc::new(AtomicUsize::new(0));
        let debouncer = FileDebouncer::new(config.debounce.clone());
        let debouncer_overflow_counter = debouncer.overflow_counter();
        let in_flight_tracker = debouncer.tracker();

        // 1. Iniciar pipeline de debounce e estabilização
        let (debouncer_rx, debouncer_handle) = debouncer.start(raw_rx, shutdown_rx.clone());

        let dispatcher_handle = spawn_stability_dispatcher(
            config.clone(),
            debouncer_rx,
            stable_tx,
            file_access,
            in_flight_tracker.clone(),
            shutdown_rx.clone(),
        );

        // 2. Configurar o watcher do sistema operacional (notify) sem bloqueio
        let (notify_tx, mut notify_rx) = mpsc::channel::<notify::Result<notify::Event>>(2048);
        let raw_overflow_cb = raw_overflow_counter.clone();

        let watcher = RecommendedWatcher::new(
            move |res| {
                if let Err(e) = notify_tx.try_send(res) {
                    match e {
                        mpsc::error::TrySendError::Full(_) => {
                            let count = raw_overflow_cb.fetch_add(1, Ordering::Relaxed) + 1;
                            tracing::warn!(
                                overflow_count = count,
                                "Canal de eventos do sistema operacional saturado! Evento descartado sem bloquear."
                            );
                        }
                        mpsc::error::TrySendError::Closed(_) => {}
                    }
                }
            },
            Config::default(),
        )?;

        let watcher_shared = Arc::new(Mutex::new(Some(watcher)));
        let watched_dirs = Arc::new(Mutex::new(HashMap::new()));
        let pending_targets = Arc::new(Mutex::new(Vec::new()));

        // Rastreamento de arquivos e diretórios conhecidos para diferenciar remoção de arquivos de diretórios
        let known_files = Arc::new(Mutex::new(HashSet::new()));
        let known_dirs = Arc::new(Mutex::new(HashSet::new()));

        // Popula arquivos e diretórios existentes no startup
        let scanner = SaveScanner::default();
        for loc in &locations {
            if let Ok(discovered) = scanner.scan(loc) {
                let mut kf = known_files.lock().unwrap();
                for f in discovered {
                    kf.insert(f.path);
                }
            }
        }

        // 3. Registrar watches iniciais e encontrar ancestrais para diretórios pendentes
        let watch_targets = resolve_watch_targets(&locations);
        for target in watch_targets {
            if target.desired_path.exists() {
                register_watch(&watcher_shared, &watched_dirs, &target.desired_path, target.mode);
                if target.desired_path.is_dir() {
                    known_dirs.lock().unwrap().insert(target.desired_path.clone());
                }
            } else {
                // Diretório ainda não existe: encontrar o ancestral mais próximo existente
                if let Some(ancestor) = find_nearest_existing_ancestor(&target.desired_path) {
                    tracing::info!(
                        "Diretório '{}' não existe ainda. Registrando watch recursivo no ancestral mais próximo: '{}'",
                        target.desired_path.display(),
                        ancestor.display()
                    );
                    register_watch(&watcher_shared, &watched_dirs, &ancestor, RecursiveMode::Recursive);
                }
                pending_targets.lock().unwrap().push(target);
            }
        }

        // 4. Task para receber eventos brutos do notify, atualizar diretórios dinâmicos e filtrar contra SaveLocations
        let locations_clone = locations.clone();
        let mut shutdown_rx_filter = shutdown_rx.clone();
        let watcher_clone = watcher_shared.clone();
        let watched_dirs_clone = watched_dirs.clone();
        let pending_targets_clone = pending_targets.clone();
        let known_files_clone = known_files.clone();
        let known_dirs_clone = known_dirs.clone();

        let filter_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;

                    changed = shutdown_rx_filter.changed() => {
                        if changed.is_err() || *shutdown_rx_filter.borrow() {
                            break;
                        }
                    }

                    event_opt = notify_rx.recv() => {
                        let res = match event_opt {
                            Some(r) => r,
                            None => break,
                        };

                        let event = match res {
                            Ok(e) => e,
                            Err(err) => {
                                tracing::warn!("Erro retornado pelo notify: {}", err);
                                continue;
                            }
                        };

                        // Verifica se algum diretório pendente agora foi criado ou recriado
                        check_and_register_dynamic_directories(
                            &watcher_clone,
                            &watched_dirs_clone,
                            &pending_targets_clone,
                            &known_dirs_clone,
                            &event,
                        );

                        // Processamento dos caminhos afetados
                        for path in event.paths {
                            handle_path_event(
                                path,
                                &event.kind,
                                &locations_clone,
                                &known_files_clone,
                                &known_dirs_clone,
                                &raw_tx,
                            ).await;
                        }
                    }
                }
            }
        });

        let task_handles = Arc::new(Mutex::new(vec![debouncer_handle, dispatcher_handle, filter_handle]));

        let service = Self {
            shutdown_tx,
            task_handles,
            _watcher: watcher_shared,
            locations,
            raw_overflow_counter,
            debouncer_overflow_counter,
            in_flight_tracker,
        };

        Ok((service, stable_rx))
    }

    /// Inicia o pipeline lógico sem registrar o watcher real do sistema operacional.
    /// Útil para testes unitários determinísticos injetando eventos diretamente via canal.
    pub fn start_pipeline(
        config: SaveWatcherConfig,
        locations: Vec<SaveLocation>,
        file_access: Arc<dyn FileAccess>,
    ) -> (Self, mpsc::Sender<RawFileEvent>, mpsc::Receiver<StableFileEvent>) {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (raw_tx, raw_rx) = mpsc::channel(config.raw_channel_capacity);
        let (stable_tx, stable_rx) = mpsc::channel(config.stable_channel_capacity);

        let raw_overflow_counter = Arc::new(AtomicUsize::new(0));
        let debouncer = FileDebouncer::new(config.debounce.clone());
        let debouncer_overflow_counter = debouncer.overflow_counter();
        let in_flight_tracker = debouncer.tracker();

        let (debouncer_rx, debouncer_handle) = debouncer.start(raw_rx, shutdown_rx.clone());

        let dispatcher_handle = spawn_stability_dispatcher(
            config,
            debouncer_rx,
            stable_tx,
            file_access,
            in_flight_tracker.clone(),
            shutdown_rx,
        );

        let task_handles = Arc::new(Mutex::new(vec![debouncer_handle, dispatcher_handle]));

        let service = Self {
            shutdown_tx,
            task_handles,
            _watcher: Arc::new(Mutex::new(None)),
            locations,
            raw_overflow_counter,
            debouncer_overflow_counter,
            in_flight_tracker,
        };

        (service, raw_tx, stable_rx)
    }

    /// Retorna a contagem de eventos descartados por overflow no callback do notify.
    pub fn raw_overflow_count(&self) -> usize {
        self.raw_overflow_counter.load(Ordering::SeqCst)
    }

    /// Retorna a contagem de eventos descartados por limite de caminhos pendentes no debouncer.
    pub fn debouncer_overflow_count(&self) -> usize {
        self.debouncer_overflow_counter.load(Ordering::SeqCst)
    }

    /// Retorna a contagem total de overflows registrados.
    pub fn total_overflow_count(&self) -> usize {
        self.raw_overflow_count() + self.debouncer_overflow_count()
    }

    /// Retorna as localizações de save configuradas no serviço.
    pub fn locations(&self) -> &[SaveLocation] {
        &self.locations
    }

    /// Encerra todas as tasks e cancela o monitoramento de forma limpa e idempotente.
    pub async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
        self.in_flight_tracker.cancel_all();

        // Desaloca o watcher do notify para liberar handles do sistema operacional
        if let Ok(mut watcher_opt) = self._watcher.lock() {
            watcher_opt.take();
        }

        let handles: Vec<_> = if let Ok(mut lock) = self.task_handles.lock() {
            lock.drain(..).collect()
        } else {
            Vec::new()
        };

        for handle in handles {
            let _ = handle.await;
        }
    }
}

impl Drop for SaveWatcherService {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        self.in_flight_tracker.cancel_all();
        if let Ok(mut watcher_opt) = self._watcher.lock() {
            watcher_opt.take();
        }
        if let Ok(handles) = self.task_handles.lock() {
            for handle in handles.iter() {
                handle.abort();
            }
        }
    }
}

/// Trata um evento individual de path do notify, mapeando para SaveLocations e diferenciando arquivos de pastas.
async fn handle_path_event(
    path: PathBuf,
    event_kind: &notify::EventKind,
    locations: &[SaveLocation],
    known_files: &Arc<Mutex<HashSet<PathBuf>>>,
    known_dirs: &Arc<Mutex<HashSet<PathBuf>>>,
    raw_tx: &mpsc::Sender<RawFileEvent>,
) {
    // Se o caminho ainda existir no disco e for diretório:
    if path.is_dir() {
        known_dirs.lock().unwrap().insert(path);
        return;
    }

    let is_removal = matches!(event_kind, notify::EventKind::Remove(_));

    if is_removal {
        // Se estava registrado como diretório conhecido, remover e NÃO emitir como arquivo
        if known_dirs.lock().unwrap().remove(&path) {
            tracing::debug!("Diretório removido no disco: '{}'. Ignorando para eventos de arquivo.", path.display());
            return;
        }

        // Só emite se era um arquivo conhecido ou se é o arquivo configurado de SaveLocationType::File
        let was_known_file = known_files.lock().unwrap().remove(&path);
        if let Some((loc_id, loc_type)) = match_path_to_locations(&path, locations) {
            if was_known_file || loc_type == SaveLocationType::File {
                let raw_event = RawFileEvent {
                    path,
                    save_location_id: loc_id,
                    kind: RawFileEventKind::Removed,
                };
                let _ = raw_tx.send(raw_event).await;
            }
        }
        return;
    }

    // Eventos não de remoção (Create, Modify, Rename)
    let kind = match event_kind {
        notify::EventKind::Create(_) => RawFileEventKind::Created,
        notify::EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
            // Em rename/atomic save: se o arquivo agora existe, Created ou Modified
            if path.exists() {
                let is_known = known_files.lock().unwrap().contains(&path);
                if is_known {
                    RawFileEventKind::Modified
                } else {
                    RawFileEventKind::Created
                }
            } else {
                // Arquivo origem desapareceu no rename
                if known_files.lock().unwrap().remove(&path) {
                    RawFileEventKind::Removed
                } else {
                    return;
                }
            }
        }
        notify::EventKind::Modify(_) => RawFileEventKind::Modified,
        _ => return,
    };

    if let Some((loc_id, _)) = match_path_to_locations(&path, locations) {
        if kind != RawFileEventKind::Removed {
            known_files.lock().unwrap().insert(path.clone());
        }
        let raw_event = RawFileEvent {
            path,
            save_location_id: loc_id,
            kind,
        };
        let _ = raw_tx.send(raw_event).await;
    }
}

/// Verifica diretórios pendentes que podem ter acabado de ser criados no disco.
fn check_and_register_dynamic_directories(
    watcher: &Arc<Mutex<Option<RecommendedWatcher>>>,
    watched_dirs: &Arc<Mutex<HashMap<PathBuf, RecursiveMode>>>,
    pending_targets: &Arc<Mutex<Vec<WatchTarget>>>,
    known_dirs: &Arc<Mutex<HashSet<PathBuf>>>,
    _event: &notify::Event,
) {
    let mut pending = pending_targets.lock().unwrap();
    if pending.is_empty() {
        return;
    }

    pending.retain(|target| {
        if target.desired_path.exists() {
            tracing::info!(
                "Diretório pendente agora detectado como criado: '{}'. Registrando watch.",
                target.desired_path.display()
            );
            register_watch(watcher, watched_dirs, &target.desired_path, target.mode);
            if target.desired_path.is_dir() {
                known_dirs.lock().unwrap().insert(target.desired_path.clone());
            }
            false // Remove de pending
        } else {
            true // Mantém em pending
        }
    });
}

/// Registra um diretório para monitoramento no notify de forma segura contra duplicatas.
fn register_watch(
    watcher: &Arc<Mutex<Option<RecommendedWatcher>>>,
    watched_dirs: &Arc<Mutex<HashMap<PathBuf, RecursiveMode>>>,
    path: &Path,
    mode: RecursiveMode,
) {
    let mut map = watched_dirs.lock().unwrap();
    if let Some(&existing_mode) = map.get(path) {
        if existing_mode == RecursiveMode::Recursive || existing_mode == mode {
            return;
        }
    }

    if let Ok(mut watcher_opt) = watcher.lock() {
        if let Some(ref mut w) = *watcher_opt {
            if let Err(e) = w.watch(path, mode) {
                tracing::warn!("Falha ao registrar watch no diretório '{}': {}", path.display(), e);
            } else {
                tracing::info!("Watch registrado com sucesso em: '{}' ({:?})", path.display(), mode);
                map.insert(path.to_path_buf(), mode);
            }
        }
    }
}

/// Encontra o ancestral existente mais próximo para caminhos que ainda não existem no disco.
pub fn find_nearest_existing_ancestor(path: &Path) -> Option<PathBuf> {
    for ancestor in path.ancestors().skip(1) {
        if ancestor.exists() && ancestor.is_dir() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

/// Spawna a task de dispatch e validação de estabilidade com concorrência estritamente limitada por semáforo
/// e rastreamento completo de ciclo de vida via JoinSet e InFlightTracker.
fn spawn_stability_dispatcher(
    config: SaveWatcherConfig,
    mut coalesced_rx: mpsc::Receiver<CoalescedFileEvent>,
    stable_tx: mpsc::Sender<StableFileEvent>,
    file_access: Arc<dyn FileAccess>,
    in_flight_tracker: InFlightTracker,
    mut shutdown_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let semaphore = Arc::new(Semaphore::new(config.max_concurrent_stability_checks));
    let stability_checker = Arc::new(StabilityChecker::new(config.stability));

    tokio::spawn(async move {
        let mut join_set = tokio::task::JoinSet::new();

        loop {
            tokio::select! {
                biased;

                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        break;
                    }
                }

                // Drena tarefas concluídas do JoinSet
                Some(res) = join_set.join_next(), if !join_set.is_empty() => {
                    if let Err(e) = res {
                        if !e.is_cancelled() {
                            tracing::error!("Erro em stability task: {}", e);
                        }
                    }
                }

                coalesced_opt = coalesced_rx.recv() => {
                    let event = match coalesced_opt {
                        Some(ev) => ev,
                        None => break,
                    };

                    // Se for evento de remoção consolidada
                    if event.kind == RawFileEventKind::Removed {
                        let stable_ev = StableFileEvent {
                            path: event.path,
                            save_location_id: event.save_location_id,
                            kind: StableFileEventKind::Removed,
                            size_bytes: 0,
                            observed_at: SystemTime::now(),
                        };
                        let _ = stable_tx.send(stable_ev).await;
                        continue;
                    }

                    // Controle de concorrência: adquirir permissão do Semaphore antes de disparar
                    let permit = match semaphore.clone().acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => break,
                    };

                    let checker = stability_checker.clone();
                    let access = file_access.clone();
                    let out_tx = stable_tx.clone();
                    let tracker = in_flight_tracker.clone();
                    let cancel_token = tracker.get_cancel_token(&event.path);

                    join_set.spawn(async move {
                        let _permit = permit;

                        // Se cancelado ou se uma nova geração foi registrada, descarta imediatamente
                        if !tracker.is_current(&event.path, event.generation) {
                            return;
                        }

                        match checker.check_stability_with_cancel(&event.path, &*access, cancel_token.as_deref()).await {
                            StabilityResult::Stable(meta) => {
                                // Validação antes de publicar: verifica se a geração ainda é válida
                                if !tracker.is_current(&event.path, event.generation) {
                                    tracing::debug!(
                                        "Evento estável obsoleto descartado para '{}'",
                                        event.path.display()
                                    );
                                    return;
                                }

                                let kind = match event.kind {
                                    RawFileEventKind::Created => StableFileEventKind::Created,
                                    _ => StableFileEventKind::Modified,
                                };

                                let stable_ev = StableFileEvent {
                                    path: event.path,
                                    save_location_id: event.save_location_id,
                                    kind,
                                    size_bytes: meta.size_bytes,
                                    observed_at: meta.modified_at,
                                };

                                let _ = out_tx.send(stable_ev).await;
                            }
                            StabilityResult::Disappeared => {
                                tracing::debug!(
                                    "Arquivo '{}' desapareceu durante verificação de estabilidade (arquivo temporário de gravação atômica).",
                                    event.path.display()
                                );
                            }
                            StabilityResult::Timeout { attempts } => {
                                tracing::warn!(
                                    "Arquivo '{}' não estabilizou após {} tentativas.",
                                    event.path.display(),
                                    attempts
                                );
                            }
                            StabilityResult::SharingViolationTimeout { attempts } => {
                                tracing::warn!(
                                    "Arquivo '{}' continua retido exclusivamente pelo jogo após {} tentativas com backoff.",
                                    event.path.display(),
                                    attempts
                                );
                            }
                            StabilityResult::PermissionDenied => {
                                tracing::error!(
                                    "Acesso negado permanentemente (PermissionDenied) a '{}'.",
                                    event.path.display()
                                );
                            }
                            StabilityResult::Cancelled => {
                                tracing::debug!(
                                    "Verificação de estabilidade cancelada para '{}'.",
                                    event.path.display()
                                );
                            }
                        }
                    });
                }
            }
        }

        // No shutdown da task dispatcher:
        // 1. Sinalizar cancelamento no tracker para todas as tarefas em voo
        in_flight_tracker.cancel_all();

        // 2. Aguarda e aborta todas as tasks do JoinSet
        join_set.shutdown().await;
    })
}

/// Identifica os alvos de monitoramento para cada SaveLocation.
pub fn resolve_watch_targets(locations: &[SaveLocation]) -> Vec<WatchTarget> {
    let mut targets = Vec::new();

    for loc in locations {
        match loc.location_type {
            SaveLocationType::File => {
                if let Some(parent) = loc.path.parent() {
                    targets.push(WatchTarget {
                        desired_path: parent.to_path_buf(),
                        mode: RecursiveMode::NonRecursive,
                    });
                }
            }
            SaveLocationType::Directory => {
                if loc.path.is_file() {
                    if let Some(parent) = loc.path.parent() {
                        targets.push(WatchTarget {
                            desired_path: parent.to_path_buf(),
                            mode: RecursiveMode::NonRecursive,
                        });
                    }
                } else {
                    targets.push(WatchTarget {
                        desired_path: loc.path.clone(),
                        mode: RecursiveMode::Recursive,
                    });
                }
            }
            SaveLocationType::Glob => {
                if let Ok((base_dir, _)) = split_glob_base_and_pattern(&loc.path) {
                    targets.push(WatchTarget {
                        desired_path: base_dir,
                        mode: RecursiveMode::Recursive,
                    });
                }
            }
            SaveLocationType::Registry => {}
        }
    }

    targets
}

/// Identifica diretórios únicos que precisam ser registrados no `notify`, evitando registros duplicados.
pub fn resolve_watch_directories(locations: &[SaveLocation]) -> HashMap<PathBuf, RecursiveMode> {
    let mut map = HashMap::new();

    for target in resolve_watch_targets(locations) {
        map.entry(target.desired_path)
            .and_modify(|m| {
                if target.mode == RecursiveMode::Recursive {
                    *m = RecursiveMode::Recursive;
                }
            })
            .or_insert(target.mode);
    }

    map
}

/// Verifica se um caminho de evento físico pertence a algum dos `SaveLocation`s registrados.
pub fn match_path_to_locations(
    path: &Path,
    locations: &[SaveLocation],
) -> Option<(Uuid, SaveLocationType)> {
    if is_internal_savevault_file(path) {
        return None;
    }

    if path
        .extension()
        .and_then(|e| e.to_str())
        .map(is_ignored_file_extension)
        .unwrap_or(false)
    {
        return None;
    }

    let canon_path = match canonicalize_windows_path(path) {
        Ok(p) => p,
        Err(_) => return None,
    };

    for loc in locations {
        match loc.location_type {
            SaveLocationType::File => {
                if let Ok(canon_loc) = canonicalize_windows_path(&loc.path) {
                    if canon_path == canon_loc {
                        return Some((loc.id, SaveLocationType::File));
                    }
                }
            }
            SaveLocationType::Directory => {
                if let Ok(canon_loc) = canonicalize_windows_path(&loc.path) {
                    if canon_path == canon_loc {
                        return Some((loc.id, SaveLocationType::File));
                    }
                    if canon_path != canon_loc && canon_path.starts_with(&canon_loc) {
                        if let Ok(rel) = canon_path.strip_prefix(&canon_loc) {
                            if is_ignored_relative_save_path(rel) {
                                return None;
                            }
                        }
                        return Some((loc.id, SaveLocationType::Directory));
                    }
                }
            }
            SaveLocationType::Glob => {
                if let Ok((base_dir, pattern)) = split_glob_base_and_pattern(&loc.path) {
                    if let Ok(canon_base) = canonicalize_windows_path(&base_dir) {
                        if canon_path != canon_base && canon_path.starts_with(&canon_base) {
                            if let Ok(rel_path) = canon_path.strip_prefix(&canon_base) {
                                if is_ignored_relative_save_path(rel_path) {
                                    return None;
                                }
                                if match_glob_relative(&pattern, rel_path) {
                                    return Some((loc.id, SaveLocationType::Glob));
                                }
                            }
                        }
                    }
                }
            }
            SaveLocationType::Registry => {}
        }
    }

    None
}
