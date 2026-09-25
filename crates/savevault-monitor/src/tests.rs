//! Testes unitários e de integração do pipeline de monitoramento de arquivos (Etapa 3).
//!
//! Cobre os 14 cenários mandatários especificados:
//! 1. Scanner de arquivo existente
//! 2. Scanner de arquivo inexistente
//! 3. Scanner de diretório
//! 4. Scanner de glob (*.sav)
//! 5. Scanner de glob não correspondente
//! 6. Debounce de múltiplos eventos consecutivos
//! 7. Coalescing (Create + Modify + Modify)
//! 8. Estabilidade com arquivo em mutação (tamanho mudando)
//! 9. Estabilidade concluída com arquivo consistente
//! 10. Sharing violation com retry e backoff limitado
//! 11. Arquivo desaparecendo durante estabilidade
//! 12. Limite de concorrência com semáforo sob alto volume
//! 13. Glob com múltiplos arquivos
//! 14. Atomic save / rename (temp -> rename -> final)

use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tempfile::tempdir;
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

use savevault_core::models::{SaveLocation, SaveLocationType};

use crate::debouncer::{DebounceConfig, FileDebouncer};
use crate::events::{RawFileEvent, RawFileEventKind};
use crate::file_access::{FileAccess, FileAccessError, FileMetadata, OsFileAccess};
use crate::scanner::{split_glob_base_and_pattern, wildcard_match, SaveScanner};
use crate::stability::{StabilityChecker, StabilityConfig, StabilityResult};
use crate::watcher::{
    match_path_to_locations, resolve_watch_directories, SaveWatcherConfig, SaveWatcherService,
};

// ============================================================================
// MOCKS PARA TESTES DETERMINÍSTICOS
// ============================================================================

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MockFailureKind {
    PermissionDenied,
    SharingViolation,
    NotFound,
}

/// Mock flexível de `FileAccess` para testar estabilidade, locks e concorrência sem race conditions.
#[derive(Default)]
struct MockFileAccess {
    /// Número de chamadas a `verify_readable` antes de retornar sucesso (simula lock temporário).
    readable_failures_before_success: Mutex<HashMap<PathBuf, usize>>,
    /// Falha configurada diretamente para verify_readable (ex: PermissionDenied).
    fail_readable_with: Mutex<HashMap<PathBuf, MockFailureKind>>,
    /// Sequência de metadados retornados em chamadas sucessivas de `metadata` (simula arquivo crescendo).
    metadata_sequence: Mutex<HashMap<PathBuf, Vec<Result<FileMetadata, FileAccessError>>>>,
    /// Delay artificial injetado durante consultas para forçar sobreposição e concorrência real.
    artificial_delay: Mutex<Option<Duration>>,
    /// Rastreia concorrência ativa em tempo real.
    active_concurrent_calls: AtomicUsize,
    /// Concorrência máxima observada durante a execução.
    peak_concurrent_calls: AtomicUsize,
}

impl MockFileAccess {
    fn new() -> Self {
        Self::default()
    }

    fn set_readable_failures(&self, path: PathBuf, failures: usize) {
        self.readable_failures_before_success
            .lock()
            .unwrap()
            .insert(path, failures);
    }

    fn set_fail_readable_with(&self, path: PathBuf, err: MockFailureKind) {
        self.fail_readable_with
            .lock()
            .unwrap()
            .insert(path, err);
    }

    fn set_metadata_sequence(
        &self,
        path: PathBuf,
        sequence: Vec<Result<FileMetadata, FileAccessError>>,
    ) {
        self.metadata_sequence
            .lock()
            .unwrap()
            .insert(path, sequence);
    }

    fn set_artificial_delay(&self, delay: Option<Duration>) {
        *self.artificial_delay.lock().unwrap() = delay;
    }
}

impl FileAccess for MockFileAccess {
    fn metadata(&self, path: &Path) -> Result<FileMetadata, FileAccessError> {
        let current = self.active_concurrent_calls.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak_concurrent_calls.fetch_max(current, Ordering::SeqCst);

        if let Some(delay) = *self.artificial_delay.lock().unwrap() {
            std::thread::sleep(delay);
        }

        let mut seq_map = self.metadata_sequence.lock().unwrap();
        let res = if let Some(seq) = seq_map.get_mut(path) {
            if seq.len() > 1 {
                seq.remove(0)
            } else if seq.len() == 1 {
                match &seq[0] {
                    Ok(m) => Ok(m.clone()),
                    Err(_) => seq.remove(0),
                }
            } else {
                Err(FileAccessError::NotFound)
            }
        } else {
            // Padrão estável se não configurado
            Ok(FileMetadata {
                size_bytes: 1024,
                modified_at: SystemTime::UNIX_EPOCH,
            })
        };

        self.active_concurrent_calls.fetch_sub(1, Ordering::SeqCst);
        res
    }

    fn verify_readable(&self, path: &Path) -> Result<(), FileAccessError> {
        if let Some(err) = self.fail_readable_with.lock().unwrap().get(path) {
            return match err {
                MockFailureKind::PermissionDenied => Err(FileAccessError::PermissionDenied),
                MockFailureKind::SharingViolation => Err(FileAccessError::SharingViolation),
                MockFailureKind::NotFound => Err(FileAccessError::NotFound),
            };
        }

        let mut fail_map = self.readable_failures_before_success.lock().unwrap();
        if let Some(remaining) = fail_map.get_mut(path) {
            if *remaining > 0 {
                *remaining -= 1;
                return Err(FileAccessError::SharingViolation);
            }
        }
        Ok(())
    }
}

// ============================================================================
// TESTES DE FILE SCANNER (Cenários 1, 2, 3, 4, 5, 13)
// ============================================================================

#[test]
fn test_01_scanner_existing_file() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("save.dat");
    {
        let mut f = File::create(&file_path).unwrap();
        f.write_all(b"dados_de_save_12345").unwrap();
    }

    let loc = SaveLocation::new(
        Uuid::new_v4(),
        file_path.clone(),
        SaveLocationType::File,
    );

    let scanner = SaveScanner::default();
    let files = scanner.scan(&loc).unwrap();

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, file_path);
    assert_eq!(files[0].size_bytes, 19);
    assert_eq!(files[0].save_location_id, loc.id);
}

#[test]
fn test_02_scanner_non_existing_file_returns_empty_without_panic() {
    let non_existent = PathBuf::from(r"C:\savevault_fake_test_dir_9999\missing_save.dat");
    let loc = SaveLocation::new(
        Uuid::new_v4(),
        non_existent,
        SaveLocationType::File,
    );

    let scanner = SaveScanner::default();
    let files = scanner.scan(&loc).unwrap();

    assert!(files.is_empty(), "Arquivo inexistente deve retornar vetor vazio sem erro fatal");
}

#[test]
fn test_03_scanner_directory() {
    let dir = tempdir().unwrap();
    let sub = dir.path().join("profile1");
    std::fs::create_dir_all(&sub).unwrap();

    let file1 = sub.join("slot1.sav");
    let file2 = sub.join("slot2.sav");
    File::create(&file1).unwrap().write_all(b"slot1").unwrap();
    File::create(&file2).unwrap().write_all(b"slot2").unwrap();

    let loc = SaveLocation::new(
        Uuid::new_v4(),
        dir.path().to_path_buf(),
        SaveLocationType::Directory,
    );

    let scanner = SaveScanner::default();
    let files = scanner.scan(&loc).unwrap();

    assert_eq!(files.len(), 2);
    let paths: Vec<PathBuf> = files.into_iter().map(|f| f.path).collect();
    assert!(paths.contains(&file1));
    assert!(paths.contains(&file2));
}

#[test]
fn test_04_scanner_glob_matches_pattern() {
    let dir = tempdir().unwrap();
    let sav1 = dir.path().join("game1.sav");
    let sav2 = dir.path().join("game2.sav");
    let other = dir.path().join("game.txt");

    File::create(&sav1).unwrap().write_all(b"sav1").unwrap();
    File::create(&sav2).unwrap().write_all(b"sav2").unwrap();
    File::create(&other).unwrap().write_all(b"txt").unwrap();

    let glob_path = dir.path().join("*.sav");
    let loc = SaveLocation::new(
        Uuid::new_v4(),
        glob_path,
        SaveLocationType::Glob,
    );

    let scanner = SaveScanner::default();
    let files = scanner.scan(&loc).unwrap();

    assert_eq!(files.len(), 2);
    let paths: Vec<PathBuf> = files.into_iter().map(|f| f.path).collect();
    assert!(paths.contains(&sav1));
    assert!(paths.contains(&sav2));
    assert!(!paths.contains(&other), "Arquivo .txt não deve corresponder ao glob *.sav");
}

#[test]
fn test_05_scanner_glob_no_match() {
    let dir = tempdir().unwrap();
    let txt = dir.path().join("readme.txt");
    File::create(&txt).unwrap().write_all(b"info").unwrap();

    let glob_path = dir.path().join("*.sav");
    let loc = SaveLocation::new(
        Uuid::new_v4(),
        glob_path,
        SaveLocationType::Glob,
    );

    let scanner = SaveScanner::default();
    let files = scanner.scan(&loc).unwrap();

    assert!(files.is_empty(), "Nenhum arquivo deve corresponder");
}

#[test]
fn test_13_scanner_glob_multiple_files() {
    let dir = tempdir().unwrap();
    for i in 1..=5 {
        let fpath = dir.path().join(format!("slot_{}.sav", i));
        File::create(&fpath).unwrap().write_all(format!("content {}", i).as_bytes()).unwrap();
    }

    let glob_path = dir.path().join("slot_*.sav");
    let loc = SaveLocation::new(
        Uuid::new_v4(),
        glob_path,
        SaveLocationType::Glob,
    );

    let scanner = SaveScanner::default();
    let files = scanner.scan(&loc).unwrap();

    assert_eq!(files.len(), 5);
}

// ============================================================================
// TESTES DE DEBOUNCE E COALESCING (Cenários 6 e 7)
// ============================================================================

#[tokio::test]
async fn test_06_debounce_multiple_writes_coalesced_into_single_event() {
    let (raw_tx, raw_rx) = tokio::sync::mpsc::channel(100);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let config = DebounceConfig::fast_for_tests();
    let (mut out_rx, handle) = FileDebouncer::new(config).start(raw_rx, shutdown_rx);

    let path = PathBuf::from(r"C:\Games\Example\save.dat");
    let loc_id = Uuid::new_v4();

    // Disparar 5 modificações rápidas consecutivas
    for _ in 0..5 {
        raw_tx
            .send(RawFileEvent {
                path: path.clone(),
                save_location_id: loc_id,
                kind: RawFileEventKind::Modified,
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // Aguardar evento coalescido
    let coalesced = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .expect("Timeout aguardando debounce")
        .expect("Canal fechado inesperadamente");

    assert_eq!(coalesced.path, path);
    assert_eq!(coalesced.kind, RawFileEventKind::Modified);
    assert_eq!(coalesced.save_location_id, loc_id);

    // Garantir que NENHUM outro evento seja emitido para as 5 escritas
    let extra = tokio::time::timeout(Duration::from_millis(50), out_rx.recv()).await;
    assert!(extra.is_err(), "Apenas 1 evento deve ser emitido após debounce");

    let _ = shutdown_tx.send(true);
    let _ = handle.await;
}

#[tokio::test]
async fn test_07_coalescing_create_plus_modify_yields_create() {
    let (raw_tx, raw_rx) = tokio::sync::mpsc::channel(100);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let config = DebounceConfig::fast_for_tests();
    let (mut out_rx, handle) = FileDebouncer::new(config).start(raw_rx, shutdown_rx);

    let path = PathBuf::from(r"C:\Games\Example\save.dat");
    let loc_id = Uuid::new_v4();

    // 1. Created
    raw_tx
        .send(RawFileEvent {
            path: path.clone(),
            save_location_id: loc_id,
            kind: RawFileEventKind::Created,
        })
        .await
        .unwrap();

    // 2. Modified logo em seguida
    raw_tx
        .send(RawFileEvent {
            path: path.clone(),
            save_location_id: loc_id,
            kind: RawFileEventKind::Modified,
        })
        .await
        .unwrap();

    // 3. Outro Modified
    raw_tx
        .send(RawFileEvent {
            path: path.clone(),
            save_location_id: loc_id,
            kind: RawFileEventKind::Modified,
        })
        .await
        .unwrap();

    let coalesced = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .expect("Timeout aguardando coalescing")
        .expect("Canal fechado");

    assert_eq!(coalesced.path, path);
    // Efeito agregado de criação + mutações subsequentes = Created
    assert_eq!(coalesced.kind, RawFileEventKind::Created);

    let _ = shutdown_tx.send(true);
    let _ = handle.await;
}

// ============================================================================
// TESTES DE ESTABILIDADE (Cenários 8, 9, 10, 11)
// ============================================================================

#[tokio::test]
async fn test_08_stability_file_actively_mutating_times_out() {
    let mock = MockFileAccess::new();
    let path = PathBuf::from(r"C:\Games\Example\growing.dat");

    // Simula arquivo crescendo a cada chamada: 100, 200, 300, 400...
    let mut seq = Vec::new();
    for i in 1..=20 {
        seq.push(Ok(FileMetadata {
            size_bytes: i * 100,
            modified_at: SystemTime::UNIX_EPOCH + Duration::from_secs(i),
        }));
    }
    mock.set_metadata_sequence(path.clone(), seq);

    let mut config = StabilityConfig::fast_for_tests();
    config.max_attempts = 5;
    config.required_stable_checks = 2;

    let checker = StabilityChecker::new(config);
    let res = checker.check_stability(&path, &mock).await;

    match res {
        StabilityResult::Timeout { attempts } => {
            assert_eq!(attempts, 5);
        }
        other => panic!("Esperado Timeout por mutação ativa, obteve: {:?}", other),
    }
}

#[tokio::test]
async fn test_09_stability_consistent_file_produces_stable_result() {
    let mock = MockFileAccess::new();
    let path = PathBuf::from(r"C:\Games\Example\stable.dat");

    // Arquivo com tamanho constante 5000 bytes
    let seq = vec![
        Ok(FileMetadata {
            size_bytes: 5000,
            modified_at: SystemTime::UNIX_EPOCH,
        }),
        Ok(FileMetadata {
            size_bytes: 5000,
            modified_at: SystemTime::UNIX_EPOCH,
        }),
        Ok(FileMetadata {
            size_bytes: 5000,
            modified_at: SystemTime::UNIX_EPOCH,
        }),
    ];
    mock.set_metadata_sequence(path.clone(), seq);

    let config = StabilityConfig::fast_for_tests();
    let checker = StabilityChecker::new(config);
    let res = checker.check_stability(&path, &mock).await;

    match res {
        StabilityResult::Stable(meta) => {
            assert_eq!(meta.size_bytes, 5000);
        }
        other => panic!("Esperado Stable, obteve: {:?}", other),
    }
}

#[tokio::test]
async fn test_10_sharing_violation_retry_with_backoff_and_recovery() {
    let mock = MockFileAccess::new();
    let path = PathBuf::from(r"C:\Games\Example\locked.dat");

    // Simula 2 falhas por SharingViolation antes do jogo liberar o lock
    mock.set_readable_failures(path.clone(), 2);

    let seq = vec![
        Ok(FileMetadata {
            size_bytes: 2048,
            modified_at: SystemTime::UNIX_EPOCH,
        }),
        Ok(FileMetadata {
            size_bytes: 2048,
            modified_at: SystemTime::UNIX_EPOCH,
        }),
    ];
    mock.set_metadata_sequence(path.clone(), seq);

    let config = StabilityConfig::fast_for_tests();
    let checker = StabilityChecker::new(config);
    let res = checker.check_stability(&path, &mock).await;

    match res {
        StabilityResult::Stable(meta) => {
            assert_eq!(meta.size_bytes, 2048);
        }
        other => panic!("Esperava estabilizar após retries de sharing violation, obteve: {:?}", other),
    }
}

#[tokio::test]
async fn test_10_sharing_violation_persistent_lock_times_out_safely() {
    let mock = MockFileAccess::new();
    let path = PathBuf::from(r"C:\Games\Example\forever_locked.dat");

    // Simula lock infinito
    mock.set_readable_failures(path.clone(), 999);

    let mut config = StabilityConfig::fast_for_tests();
    config.max_attempts = 3;

    let checker = StabilityChecker::new(config);
    let res = checker.check_stability(&path, &mock).await;

    match res {
        StabilityResult::SharingViolationTimeout { attempts } => {
            assert_eq!(attempts, 3);
        }
        other => panic!("Esperado SharingViolationTimeout, obteve: {:?}", other),
    }
}

#[tokio::test]
async fn test_11_disappearing_file_during_stability_check_returns_disappeared() {
    let mock = MockFileAccess::new();
    let path = PathBuf::from(r"C:\Games\Example\atomic_temp.tmp");

    // Primeira chamada retorna tamanho, segunda retorna NotFound
    let seq = vec![
        Ok(FileMetadata {
            size_bytes: 100,
            modified_at: SystemTime::UNIX_EPOCH,
        }),
        Err(FileAccessError::NotFound),
    ];
    mock.set_metadata_sequence(path.clone(), seq);

    let config = StabilityConfig::fast_for_tests();
    let checker = StabilityChecker::new(config);
    let res = checker.check_stability(&path, &mock).await;

    assert_eq!(
        res,
        StabilityResult::Disappeared,
        "Arquivo deletado deve retornar Disappeared sem causar panic"
    );
}

// ============================================================================
// TESTES DE PIPELINE, CONCORRÊNCIA E RENAME (Cenários 12 e 14)
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_12_bounded_concurrency_with_semaphore_under_high_event_load() {
    let mock = Arc::new(MockFileAccess::new());
    // Injeta delay controlado para forçar sobreposição temporal das tarefas de estabilidade
    mock.set_artificial_delay(Some(Duration::from_millis(25)));

    let loc_id = Uuid::new_v4();

    let mut config = SaveWatcherConfig::fast_for_tests();
    config.max_concurrent_stability_checks = 2; // Limite estrito de 2 simultâneos

    let mut location = SaveLocation::new(
        Uuid::new_v4(),
        PathBuf::from(r"C:\Games\Saves"),
        SaveLocationType::Directory,
    );
    location.id = loc_id;

    let (service, raw_tx, mut stable_rx) =
        SaveWatcherService::start_pipeline(config, vec![location], mock.clone());

    // Disparar 15 eventos para 15 arquivos distintos
    for i in 0..15 {
        let p = PathBuf::from(format!(r"C:\Games\Saves\slot_{}.dat", i));
        mock.set_metadata_sequence(
            p.clone(),
            vec![
                Ok(FileMetadata {
                    size_bytes: 1000,
                    modified_at: SystemTime::UNIX_EPOCH,
                }),
                Ok(FileMetadata {
                    size_bytes: 1000,
                    modified_at: SystemTime::UNIX_EPOCH,
                }),
            ],
        );

        raw_tx
            .send(RawFileEvent {
                path: p,
                save_location_id: loc_id,
                kind: RawFileEventKind::Created,
            })
            .await
            .unwrap();
    }

    // Coletar todos os eventos processados
    let mut received = 0;
    while received < 15 {
        match tokio::time::timeout(Duration::from_millis(1500), stable_rx.recv()).await {
            Ok(Some(_)) => received += 1,
            _ => break,
        }
    }

    assert_eq!(received, 15, "Todos os 15 eventos devem ser processados");
    let peak = mock.peak_concurrent_calls.load(Ordering::SeqCst);
    assert_eq!(
        peak, 2,
        "Pico de chamadas concorrentes ({}) deve atingir exatamente o limite do semáforo (2), provando concorrência real",
        peak
    );

    service.shutdown().await;
}

#[tokio::test]
async fn test_14_atomic_save_rename_sequence_handles_cleanly() {
    let mock = Arc::new(MockFileAccess::new());
    let loc_id = Uuid::new_v4();

    let config = SaveWatcherConfig::fast_for_tests();
    let final_path = PathBuf::from(r"C:\Games\Example\save.dat");

    let mut location = SaveLocation::new(
        Uuid::new_v4(),
        final_path.clone(),
        SaveLocationType::File,
    );
    location.id = loc_id;

    let (service, raw_tx, mut stable_rx) =
        SaveWatcherService::start_pipeline(config, vec![location], mock.clone());

    mock.set_metadata_sequence(
        final_path.clone(),
        vec![
            Ok(FileMetadata {
                size_bytes: 4096,
                modified_at: SystemTime::UNIX_EPOCH,
            }),
            Ok(FileMetadata {
                size_bytes: 4096,
                modified_at: SystemTime::UNIX_EPOCH,
            }),
        ],
    );

    // Enviar evento de Renamed para save.dat
    raw_tx
        .send(RawFileEvent {
            path: final_path.clone(),
            save_location_id: loc_id,
            kind: RawFileEventKind::Renamed,
        })
        .await
        .unwrap();

    let stable_event = tokio::time::timeout(Duration::from_millis(300), stable_rx.recv())
        .await
        .expect("Timeout aguardando evento de rename")
        .expect("Canal fechado");

    assert_eq!(stable_event.path, final_path);
    assert_eq!(stable_event.size_bytes, 4096);
    assert_eq!(stable_event.save_location_id, loc_id);

    service.shutdown().await;
}

// ============================================================================
// TESTES DE HELPERS E DEDUPLICAÇÃO
// ============================================================================

#[test]
fn test_resolve_watch_directories_deduplicates_same_parent() {
    let loc1 = SaveLocation::new(
        Uuid::new_v4(),
        PathBuf::from(r"C:\Games\Example\slot1.dat"),
        SaveLocationType::File,
    );

    let loc2 = SaveLocation::new(
        Uuid::new_v4(),
        PathBuf::from(r"C:\Games\Example\slot2.dat"),
        SaveLocationType::File,
    );

    let dirs = resolve_watch_directories(&[loc1, loc2]);
    assert_eq!(dirs.len(), 1, "Ambos os arquivos devem compartilhar o mesmo diretório pai");
    assert!(dirs.contains_key(&PathBuf::from(r"C:\Games\Example")));
}

#[test]
fn test_match_path_to_locations_filtering() {
    let loc_file = SaveLocation::new(
        Uuid::new_v4(),
        PathBuf::from(r"C:\Games\Example\save.dat"),
        SaveLocationType::File,
    );

    let loc_glob = SaveLocation::new(
        Uuid::new_v4(),
        PathBuf::from(r"C:\Games\Example\*.sav"),
        SaveLocationType::Glob,
    );

    let locations = vec![loc_file.clone(), loc_glob.clone()];

    // Correspondência de arquivo literal (case-insensitive)
    let m1 = match_path_to_locations(Path::new(r"c:\games\example\save.dat"), &locations);
    assert_eq!(m1, Some((loc_file.id, SaveLocationType::File)));

    // Correspondência de glob
    let m2 = match_path_to_locations(Path::new(r"c:\games\example\slot1.sav"), &locations);
    assert_eq!(m2, Some((loc_glob.id, SaveLocationType::Glob)));

    // Não correspondência
    let m3 = match_path_to_locations(Path::new(r"c:\games\example\other.log"), &locations);
    assert_eq!(m3, None);
}

#[test]
fn test_wildcard_matching_rules() {
    assert!(wildcard_match("*.sav", "game.sav"));
    assert!(wildcard_match("*.sav", "GAME.SAV"));
    assert!(wildcard_match("slot_?.dat", "slot_1.dat"));
    assert!(!wildcard_match("slot_?.dat", "slot_12.dat"));
    assert!(!wildcard_match("*.sav", "game.dat"));
}

#[test]
fn test_split_glob_base_and_pattern() {
    let (base, pat) = split_glob_base_and_pattern(Path::new(r"C:\Games\Example\*.sav")).unwrap();
    assert_eq!(base, PathBuf::from(r"C:\Games\Example"));
    assert_eq!(pat, "*.sav");

    let (base2, pat2) = split_glob_base_and_pattern(Path::new(r"C:\Games\slot_?\save.dat")).unwrap();
    assert_eq!(base2, PathBuf::from(r"C:\Games"));
    assert_eq!(pat2, r"slot_?\save.dat");
}

#[tokio::test]
async fn test_14_atomic_save_rename_real_filesystem() {
    let dir = tempdir().unwrap();
    let save_dir = dir.path().to_path_buf();
    let final_save_path = save_dir.join("game.sav");

    let loc = SaveLocation::new(
        Uuid::new_v4(),
        save_dir.clone(),
        SaveLocationType::Directory,
    );

    let config = SaveWatcherConfig::fast_for_tests();
    let (service, mut stable_rx) = SaveWatcherService::start(
        config,
        vec![loc.clone()],
        Arc::new(OsFileAccess),
    )
    .expect("Falha ao iniciar watcher");

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Simula salvamento atômico do jogo:
    // 1. Cria temp file
    let tmp_path = save_dir.join("game.sav.tmp");
    {
        let mut f = File::create(&tmp_path).unwrap();
        f.write_all(b"dados_do_save_atomico_v1").unwrap();
        f.sync_all().unwrap();
    }

    // 2. Renomeia atomicamente para o destino final
    std::fs::rename(&tmp_path, &final_save_path).unwrap();

    // 3. Aguardar evento para o arquivo final game.sav
    let mut detected_final = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);

    while tokio::time::Instant::now() < deadline {
        if let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(500), stable_rx.recv()).await {
            if event.path.file_name() == final_save_path.file_name() {
                assert_eq!(event.size_bytes, 24);
                assert_eq!(event.save_location_id, loc.id);
                detected_final = true;
                break;
            }
        }
    }

    assert!(detected_final, "O arquivo final após rename atômico deve ser detectado e estabilizado");
    service.shutdown().await;
}

#[test]
fn test_glob_subdirectory_and_traversal_matching() {
    let loc_sub = SaveLocation::new(
        Uuid::new_v4(),
        PathBuf::from(r"C:\Saves\*\*.sav"),
        SaveLocationType::Glob,
    );
    let loc_multi = SaveLocation::new(
        Uuid::new_v4(),
        PathBuf::from(r"C:\Saves\*\*\*.sav"),
        SaveLocationType::Glob,
    );
    let loc_wildcard_name = SaveLocation::new(
        Uuid::new_v4(),
        PathBuf::from(r"C:\Saves\Game*\*.sav"),
        SaveLocationType::Glob,
    );

    let locations = vec![loc_sub.clone(), loc_multi.clone(), loc_wildcard_name.clone()];

    // 1. C:\Saves\*\*.sav -> C:\Saves\GameA\slot1.sav
    let m1 = match_path_to_locations(Path::new(r"c:\saves\gamea\slot1.sav"), &locations);
    assert_eq!(m1, Some((loc_sub.id, SaveLocationType::Glob)));

    // 2. C:\Saves\*\*\*.sav -> C:\Saves\GameA\Profile1\slot1.sav
    let m2 = match_path_to_locations(Path::new(r"C:\Saves\GameA\Profile1\slot1.sav"), &locations);
    assert_eq!(m2, Some((loc_multi.id, SaveLocationType::Glob)));

    // 3. C:\Saves\Game*\*.sav -> C:\Saves\Game1\slot1.sav (e não-match com Other)
    let m3 = match_path_to_locations(Path::new(r"C:\Saves\Game1\slot1.sav"), std::slice::from_ref(&loc_wildcard_name));
    assert_eq!(m3, Some((loc_wildcard_name.id, SaveLocationType::Glob)));

    let m3_nomatch = match_path_to_locations(Path::new(r"C:\Saves\Other\slot1.sav"), std::slice::from_ref(&loc_wildcard_name));
    assert_eq!(m3_nomatch, None);

    // 4. Não correspondente: C:\Saves\GameA\save.txt
    let m4 = match_path_to_locations(Path::new(r"C:\Saves\GameA\save.txt"), &locations);
    assert_eq!(m4, None);

    // 5. Traversal com .. rejeitado pelo scanner
    let loc_traversal = SaveLocation::new(
        Uuid::new_v4(),
        PathBuf::from(r"C:\Saves\..\..\Windows\*.sav"),
        SaveLocationType::Glob,
    );
    let scanner = SaveScanner::default();
    let res = scanner.scan(&loc_traversal);
    assert!(matches!(res, Err(crate::scanner::ScanError::PathTraversal(_))));
}

#[tokio::test]
async fn test_dynamic_directory_created_after_startup() {
    let dir = tempdir().unwrap();
    let game_dir = dir.path().join("Games").join("MyGame");
    let save_path = game_dir.join("slot1.sav");

    // No startup, game_dir e save_path NÃO existem!
    assert!(!game_dir.exists());

    let loc = SaveLocation::new(
        Uuid::new_v4(),
        game_dir.clone(),
        SaveLocationType::Directory,
    );

    let config = SaveWatcherConfig::fast_for_tests();
    let (service, mut stable_rx) = SaveWatcherService::start(
        config,
        vec![loc.clone()],
        Arc::new(OsFileAccess),
    )
    .expect("Falha ao iniciar watcher");

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Agora o jogo inicia e cria a pasta de saves e o arquivo:
    std::fs::create_dir_all(&game_dir).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    {
        let mut f = File::create(&save_path).unwrap();
        f.write_all(b"save_data_after_startup").unwrap();
        f.sync_all().unwrap();
    }

    let event = tokio::time::timeout(Duration::from_secs(3), stable_rx.recv())
        .await
        .expect("Timeout aguardando arquivo criado em diretório dinâmico")
        .expect("Canal fechado");

    assert_eq!(
        savevault_core::path::canonicalize_windows_path(&event.path).unwrap(),
        savevault_core::path::canonicalize_windows_path(&save_path).unwrap()
    );
    assert_eq!(event.size_bytes, 23);
    assert_eq!(event.save_location_id, loc.id);

    service.shutdown().await;
}

#[tokio::test]
async fn test_directory_removal_not_emitted_as_file_removal() {
    let dir = tempdir().unwrap();
    let sub_dir = dir.path().join("sub_folder");
    std::fs::create_dir_all(&sub_dir).unwrap();

    let loc = SaveLocation::new(
        Uuid::new_v4(),
        dir.path().to_path_buf(),
        SaveLocationType::Directory,
    );

    let config = SaveWatcherConfig::fast_for_tests();
    let (service, mut stable_rx) = SaveWatcherService::start(
        config,
        vec![loc],
        Arc::new(OsFileAccess),
    )
    .expect("Falha ao iniciar watcher");

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Remove a subpasta
    std::fs::remove_dir(&sub_dir).unwrap();

    // Aguarda tempo de debounce para garantir que nenhum evento de arquivo seja emitido
    let res = tokio::time::timeout(Duration::from_millis(200), stable_rx.recv()).await;
    assert!(
        res.is_err(),
        "Remoção de diretório NÃO deve produzir evento StableFileEventKind::Removed para arquivo"
    );

    service.shutdown().await;
}

#[tokio::test]
async fn test_race_condition_debounce_and_stability_generation() {
    let mock = Arc::new(MockFileAccess::new());
    mock.set_artificial_delay(Some(Duration::from_millis(50)));

    let file_path = PathBuf::from(r"C:\Games\Saves\race_test.dat");
    let loc_id = Uuid::new_v4();
    let mut location = SaveLocation::new(
        Uuid::new_v4(),
        file_path.clone(),
        SaveLocationType::File,
    );
    location.id = loc_id;

    let config = SaveWatcherConfig::fast_for_tests();
    let (service, raw_tx, mut stable_rx) =
        SaveWatcherService::start_pipeline(config, vec![location], mock.clone());

    // Dispara primeiro evento (geração 1)
    raw_tx.send(RawFileEvent {
        path: file_path.clone(),
        save_location_id: loc_id,
        kind: RawFileEventKind::Created,
    }).await.unwrap();

    // Aguarda o debounce disparar e a verificação de estabilidade #1 iniciar
    tokio::time::sleep(Duration::from_millis(30)).await;

    // Dispara segundo evento para o mesmo caminho (geração 2)
    raw_tx.send(RawFileEvent {
        path: file_path.clone(),
        save_location_id: loc_id,
        kind: RawFileEventKind::Modified,
    }).await.unwrap();

    // Espera receber o evento estável da geração 2
    let received = tokio::time::timeout(Duration::from_millis(800), stable_rx.recv())
        .await
        .expect("Timeout aguardando evento")
        .expect("Canal fechado");

    assert_eq!(received.path, file_path);

    // Garante que NÃO haja um segundo evento duplicado da geração 1
    let duplicate = tokio::time::timeout(Duration::from_millis(150), stable_rx.recv()).await;
    assert!(duplicate.is_err(), "A geração antiga não pode produzir evento obsoleto/duplicado");

    service.shutdown().await;
}

#[tokio::test]
async fn test_debouncer_memory_limit_and_overflow_observability() {
    let mut config = DebounceConfig::fast_for_tests();
    config.max_pending_paths = 5; // Limite de 5 caminhos
    config.debounce_duration = Duration::from_millis(500); // Garante que os caminhos permaneçam em pending durante a inserção rápida

    let debouncer = FileDebouncer::new(config);
    let overflow_counter = debouncer.overflow_counter();

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (raw_tx, raw_rx) = mpsc::channel(64);
    let (_coalesced_rx, handle) = debouncer.start(raw_rx, shutdown_rx);

    // Enviar 10 caminhos distintos
    let loc_id = Uuid::new_v4();
    for i in 0..10 {
        raw_tx.send(RawFileEvent {
            path: PathBuf::from(format!(r"C:\Saves\file_{}.dat", i)),
            save_location_id: loc_id,
            kind: RawFileEventKind::Created,
        }).await.unwrap();
    }

    tokio::time::sleep(Duration::from_millis(30)).await;

    let overflow = overflow_counter.load(Ordering::SeqCst);
    assert_eq!(
        overflow, 5,
        "Exatamente 5 eventos que excederam max_pending_paths devem ser contabilizados como overflow"
    );

    let _ = shutdown_tx.send(true);
    let _ = handle.await;
}

#[tokio::test]
async fn test_shutdown_cancels_and_awaits_all_stability_tasks() {
    let mock = Arc::new(MockFileAccess::new());
    let file_path = PathBuf::from(r"C:\Games\Saves\slow_save.dat");
    // Configura lock temporário com backoff para manter a verificação ativa aguardando
    mock.set_readable_failures(file_path.clone(), 10);
    let loc = SaveLocation::new(
        Uuid::new_v4(),
        file_path.clone(),
        SaveLocationType::File,
    );

    let config = SaveWatcherConfig::fast_for_tests();
    let (service, raw_tx, mut stable_rx) =
        SaveWatcherService::start_pipeline(config, vec![loc], mock.clone());

    raw_tx.send(RawFileEvent {
        path: file_path.clone(),
        save_location_id: Uuid::new_v4(),
        kind: RawFileEventKind::Created,
    }).await.unwrap();

    // Aguarda entrar no stability check
    tokio::time::sleep(Duration::from_millis(35)).await;

    // Chama shutdown enquanto stability check ainda está dormindo/executando
    let start_shutdown = tokio::time::Instant::now();
    service.shutdown().await;
    let elapsed = start_shutdown.elapsed();

    // Shutdown deve completar prontamente através do cancelamento
    assert!(elapsed < Duration::from_millis(300), "Shutdown deve cancelar e aguardar tasks sem travar");

    // Nenhuma mensagem deve ter sido publicada antes ou após shutdown
    let res = tokio::time::timeout(Duration::from_millis(100), stable_rx.recv()).await;
    match res {
        Ok(Some(ev)) => panic!("Nenhum evento deveria ter sido publicado, mas recebeu: {:?}", ev),
        Ok(None) | Err(_) => {
            // Sucesso: canal fechou ou está vazio sem mensagens
        }
    }

    // Idempotência: segunda chamada a shutdown não deve travar
    service.shutdown().await;
}

#[tokio::test]
async fn test_distinguishes_sharing_violation_from_permission_denied() {
    let checker = StabilityChecker::new(StabilityConfig::fast_for_tests());

    // 1. Simula PermissionDenied (Win32 erro 5 / ACL)
    let mock_denied = Arc::new(MockFileAccess::new());
    mock_denied.set_fail_readable_with(
        PathBuf::from(r"C:\Saves\acl_blocked.dat"),
        MockFailureKind::PermissionDenied,
    );
    let res_denied = checker.check_stability(Path::new(r"C:\Saves\acl_blocked.dat"), &*mock_denied).await;
    assert_eq!(
        res_denied,
        StabilityResult::PermissionDenied,
        "PermissionDenied deve abortar imediatamente sem retentar como sharing violation"
    );

    // 2. Simula SharingViolation (Win32 erro 32/33) que resolve após 2 retries
    let mock_sharing = Arc::new(MockFileAccess::new());
    mock_sharing.set_readable_failures(PathBuf::from(r"C:\Saves\locked.dat"), 2);
    let res_sharing = checker.check_stability(Path::new(r"C:\Saves\locked.dat"), &*mock_sharing).await;
    assert!(
        matches!(res_sharing, StabilityResult::Stable(_)),
        "SharingViolation deve retentar com backoff e alcançar sucesso se o lock for liberado"
    );
}

#[test]
fn test_windows_path_variations_matching() {
    let loc = SaveLocation::new(
        Uuid::new_v4(),
        PathBuf::from(r"C:\Games\Saves\slot1.sav"),
        SaveLocationType::File,
    );
    let locations = vec![loc.clone()];

    // Barra invertida padrão
    assert_eq!(
        match_path_to_locations(Path::new(r"C:\Games\Saves\slot1.sav"), &locations),
        Some((loc.id, SaveLocationType::File))
    );

    // Barra normal (slash)
    assert_eq!(
        match_path_to_locations(Path::new("C:/Games/Saves/slot1.sav"), &locations),
        Some((loc.id, SaveLocationType::File))
    );

    // Letra de drive minúscula
    assert_eq!(
        match_path_to_locations(Path::new(r"c:\games\saves\slot1.sav"), &locations),
        Some((loc.id, SaveLocationType::File))
    );
}

#[tokio::test]
async fn test_15_filesystem_watcher_integration_with_notify() {
    let dir = tempdir().unwrap();
    let save_path = dir.path().join("live_save.dat");

    let loc = SaveLocation::new(
        Uuid::new_v4(),
        save_path.clone(),
        SaveLocationType::File,
    );

    let config = SaveWatcherConfig::fast_for_tests();
    let (service, mut stable_rx) = SaveWatcherService::start(
        config,
        vec![loc.clone()],
        Arc::new(OsFileAccess),
    )
    .expect("Falha ao iniciar watcher com notify real");

    // Pequeno delay para garantir que o SO registrou o watcher na pasta
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Criar e escrever no arquivo
    {
        let mut f = File::create(&save_path).unwrap();
        f.write_all(b"save_data_payload_v1").unwrap();
        f.sync_all().unwrap();
    }

    // Aguardar evento estável emitido pelo pipeline completo
    let stable = tokio::time::timeout(Duration::from_secs(3), stable_rx.recv())
        .await
        .expect("Timeout aguardando evento do notify no filesystem real")
        .expect("Canal fechado");

    assert_eq!(
        savevault_core::path::canonicalize_windows_path(&stable.path).unwrap(),
        savevault_core::path::canonicalize_windows_path(&save_path).unwrap()
    );
    assert_eq!(stable.size_bytes, 20);
    assert_eq!(stable.save_location_id, loc.id);

    service.shutdown().await;
}

#[tokio::test]
async fn test_16_clean_shutdown_terminates_all_tasks() {
    let dir = tempdir().unwrap();
    let save_path = dir.path().join("shutdown_test.dat");

    let loc = SaveLocation::new(
        Uuid::new_v4(),
        save_path.clone(),
        SaveLocationType::File,
    );

    let config = SaveWatcherConfig::fast_for_tests();
    let (service, _stable_rx) = SaveWatcherService::start(
        config,
        vec![loc],
        Arc::new(OsFileAccess),
    )
    .unwrap();

    // Chamar shutdown e verificar que completa prontamente sem travar
    let shutdown_result = tokio::time::timeout(Duration::from_millis(500), service.shutdown()).await;
    assert!(shutdown_result.is_ok(), "Shutdown deve completar prontamente sem travar");
}

