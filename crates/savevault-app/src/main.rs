//! Ponto de entrada principal do checkpoint.
//!
//! ## Uso
//! ```
//! checkpoint                    # Executa o discovery e persiste no banco
//! checkpoint restore <id>       # Restaura snapshot com safety backup e escrita atômica
//! checkpoint restore list       # Lista os snapshots disponíveis
//! checkpoint add <nome> <caminho> # Adiciona um jogo e local de save manualmente
//! checkpoint list                # Lista todos os jogos cadastrados no banco
//! ```

use savevault_core::config::Config;
use savevault_db::{
    repositories::{GamesRepo, SaveLocationsRepo},
    Database,
};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    // Otimizações estritas de baixo consumo de memória RAM para o Chromium WebView2
    #[cfg(windows)]
    std::env::set_var(
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "--disable-gpu --disable-gpu-compositing --disable-gpu-rasterization --in-process-gpu --disable-software-rasterizer --renderer-process-limit=1 --js-flags=\"--max-old-space-size=32 --optimize-for-size --lite-mode\" --disable-features=Translate,OptimizationHints,MediaRouter,CalculateNativeWinOcclusion,InterestFeedContentSuggestions,AudioServiceOutOfProcess --enable-features=NetworkServiceInProcess --disable-extensions --disable-component-update --disable-background-networking --disable-speech-api --disable-sync --disable-breakpad --disable-default-apps --disable-domain-reliability --no-pings --mute-audio --disable-dev-shm-usage"
    );

    // Carrega as variáveis de ambiente do arquivo .env (se existir)
    dotenvy::dotenv().ok();

    // Parseia os argumentos da linha de comando
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Roteia subcomandos imediatos / help sem precisar subir runtime assíncrono
    match args.as_slice() {
        [restore_cmd, help_flag]
            if restore_cmd == "restore" && (help_flag == "--help" || help_flag == "-h") =>
        {
            print_restore_help();
            return Ok(());
        }
        [restore_cmd] if restore_cmd == "restore" => {
            print_restore_help();
            return Ok(());
        }
        _ => {}
    }

    // ── Modo Desktop (Tauri) ──────────────────────────────────────────────────
    // No Windows, a janela gráfica DEVE ser inicializada na thread principal.
    // Por isso, quando compilado com --features desktop (e sem sub-comandos CLI),
    // fazemos o seguinte:
    //   1. Subimos o tokio runtime em uma thread separada de background.
    //   2. Bloqueamos ESSA thread (principal) para iniciar o Tauri.
    #[cfg(feature = "desktop")]
    {
        let should_run_ui = args.is_empty() || args.iter().any(|a| a == "--ui" || a == "ui");
        if should_run_ui {
            return run_desktop_main(args);
        }
    }

    // ── Modo CLI / Headless ──────────────────────────────────────────────────
    // Executa o runtime assíncrono em thread com stack de 16MB para evitar
    // estouro do stack padrão de 1MB da thread primária do Windows sob GCC/MinGW
    let handle = std::thread::Builder::new()
        .name("savevault-runner".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(8 * 1024 * 1024)
                .build()?;
            rt.block_on(run_async(args))
        })?;

    match handle.join() {
        Ok(res) => res,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// Ponto de entrada do modo desktop: o tokio sobe em background e o
/// Tauri (event loop de GUI) toma conta da thread principal.
#[cfg(feature = "desktop")]
fn run_desktop_main(_args: Vec<String>) -> anyhow::Result<()> {
    use std::sync::Arc;

    let _single_instance = match savevault_app::SingleInstanceGuard::acquire("Local\\checkpoint_Mutex") {
        Some(guard) => guard,
        None => {
            eprintln!("checkpoint já está em execução no sistema.");
            return Ok(());
        }
    };

    dotenvy::dotenv().ok();

    // Inicializa tracing
    let log_level = std::env::var("SAVEVAULT_LOG").unwrap_or_else(|_| "info".into());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&log_level)),
        )
        .try_init();

    // Carrega config e banco
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    let checkpoint_dir = std::path::PathBuf::from(&appdata).join("checkpoint");
    let legacy_dir = std::path::PathBuf::from(&appdata).join("SaveVault");
    let config_dir = if checkpoint_dir.exists() {
        checkpoint_dir
    } else if legacy_dir.exists() {
        legacy_dir
    } else {
        checkpoint_dir
    };
    let config_path = config_dir.join("config.toml");
    let config = Config::load(&config_path).unwrap_or_default();
    let _ = std::fs::create_dir_all(&config_dir);
    let _ = config.save(&config_path);

    // Migração automática transparente de dados legados do SaveVault para checkpoint se aplicável
    savevault_cloud::auto_migrate_legacy_cloud_folder(&config.paths.backup_dir);
    if let Some(ref lsp) = config.cloud.local_sync_path {
        savevault_cloud::auto_migrate_legacy_cloud_folder(lsp);
    }

    // Inicializa subsistema de notificações do Windows Toast
    savevault_app::notifications::init_from_config(&config.notifications);

    let db = savevault_db::Database::open(&config.paths.database_path)?;
    let db_conn = Arc::new(tokio::sync::Mutex::new(db.conn));

    let reconciler = Arc::new(savevault_app::Reconciler::new(
        db_conn.clone(),
        config.paths.backup_dir.clone(),
        config.general.max_versions,
    ));

    let sync_worker = Arc::new(savevault_app::SyncWorker::new(
        db_conn.clone(),
        config.paths.backup_dir.clone(),
        savevault_app::SyncWorkerConfig {
            provider: config.cloud.provider.clone(),
            local_sync_path: config.cloud.local_sync_path.clone(),
            reconciliation_interval: std::time::Duration::from_secs(60),
            max_attempts: 5,
            base_backoff_ms: 2000,
        },
    ));

    let state = Arc::new(savevault_app::DevUiState::new(
        db_conn.clone(),
        config.paths.database_path.clone(),
        config.paths.backup_dir.clone(),
        Some(reconciler.clone()),
        std::time::Duration::from_secs(config.general.reconciliation_interval_secs),
    ));

    // Sobe o tokio em uma thread separada para o backend (watcher, reconciler, sync worker, servidor HTTP)
    let state_bg = state.clone();
    let config_bg = config.clone();
    let config_dir_bg = config_dir.clone();
    let db_conn_bg = db_conn.clone();
    let reconciler_bg = reconciler.clone();
    let sync_worker_bg = sync_worker.clone();

    std::thread::Builder::new()
        .name("savevault-backend".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .max_blocking_threads(4)
                .enable_all()
                .build()
                .expect("Falha ao criar runtime tokio");
            rt.block_on(async move {
                // Registra o worker no DevUiState
                state_bg.set_sync_worker(sync_worker_bg.clone()).await;

                // Executa a descoberta automática de jogos instalados na máquina
                match savevault_app::run_full_discovery(&db_conn_bg, &config_dir_bg).await {
                    Ok((games_count, locs_count)) => {
                        tracing::info!(
                            "Descoberta inicial concluída: {} jogo(s), {} local(is) de save.",
                            games_count, locs_count
                        );
                    }
                    Err(e) => {
                        tracing::warn!("Erro durante descoberta inicial de jogos: {}", e);
                    }
                }

                // Lê as localizações cadastradas para o watcher e reconciliação
                let all_locations = {
                    let conn = db_conn_bg.lock().await;
                    SaveLocationsRepo::new(&conn)
                        .list_all()
                        .unwrap_or_default()
                };

                // Reconciliação inicial se houver localizações
                if !all_locations.is_empty() {
                    let _ = reconciler_bg.reconcile().await;
                }

                let reconciler_config = savevault_app::ReconciliationConfig::new(
                    std::time::Duration::from_secs(config_bg.general.reconciliation_interval_secs),
                );
                let (shutdown_tx, pipeline_shutdown_rx) = tokio::sync::watch::channel(false);
                let recon_shutdown_rx = shutdown_tx.subscribe();
                let server_shutdown_rx = shutdown_tx.subscribe();
                let worker_shutdown_rx = shutdown_tx.subscribe();

                // Reconciler periódica local
                let _recon_handle = tokio::spawn(savevault_app::run_reconciliation_loop(
                    reconciler_bg.clone(),
                    reconciler_config,
                    recon_shutdown_rx,
                ));

                // SyncWorker em segundo plano (OneDrive / Cloud Folder)
                let worker_task = sync_worker_bg.clone();
                let _sync_handle = tokio::spawn(async move {
                    let _ = worker_task.start(worker_shutdown_rx).await;
                });

                // Servidor HTTP (fallback dev-ui e API)
                let _ = savevault_app::start_dev_ui_server(
                    state_bg.clone(),
                    4040,
                    server_shutdown_rx,
                )
                .await;

                // Watcher + Pipeline com notificação instantânea para o SyncWorker
                let file_access = Arc::new(savevault_monitor::file_access::OsFileAccess);
                let mut watcher_config = savevault_monitor::watcher::SaveWatcherConfig::default();
                watcher_config.debounce.debounce_duration =
                    std::time::Duration::from_millis(config_bg.general.debounce_ms);

                if !all_locations.is_empty() {
                    match savevault_monitor::watcher::SaveWatcherService::start(
                        watcher_config,
                        all_locations,
                        file_access,
                    ) {
                        Ok((_watcher, rx)) => {
                            let _ = savevault_app::pipeline::run_pipeline_with_worker(
                                db_conn_bg.clone(),
                                rx,
                                config_bg.paths.backup_dir.clone(),
                                config_bg.general.max_versions,
                                pipeline_shutdown_rx,
                                Some(sync_worker_bg.clone()),
                            )
                            .await;
                        }
                        Err(e) => {
                            tracing::warn!("Watcher não pôde ser iniciado: {}", e);
                            let _ = tokio::signal::ctrl_c().await;
                        }
                    }
                } else {
                    let _ = tokio::signal::ctrl_c().await;
                }

                let _ = shutdown_tx.send(true);
            });
        })?;

    // Thread principal fica bloqueada rodando o Tauri (event loop nativo do Windows)
    println!("================================================================================");
    println!("               checkpoint — Iniciando Interface Desktop                         ");
    println!("================================================================================");

    savevault_app::ui::tauri_app::run_tauri_app(state)
}


async fn run_async(args: Vec<String>) -> anyhow::Result<()> {
    // Inicializa o tracing
    let log_level = std::env::var("SAVEVAULT_LOG").unwrap_or_else(|_| "info".into());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&log_level)),
        )
        .try_init();

    // Carrega configuração e banco
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    let checkpoint_dir = PathBuf::from(&appdata).join("checkpoint");
    let legacy_dir = PathBuf::from(&appdata).join("SaveVault");
    let config_dir = if checkpoint_dir.exists() {
        checkpoint_dir
    } else if legacy_dir.exists() {
        legacy_dir
    } else {
        checkpoint_dir
    };
    let config_path = config_dir.join("config.toml");
    let config = Config::load(&config_path).unwrap_or_default();
    let _ = std::fs::create_dir_all(&config_dir);
    let _ = config.save(&config_path);

    // Se compilado com desktop, modo GUI é o padrão quando não há argumentos.
    // Senão, exige `--ui` ou `ui`.
    #[cfg(feature = "desktop")]
    let should_run_ui = args.is_empty() || args.iter().any(|a| a == "--ui" || a == "ui");
    #[cfg(not(feature = "desktop"))]
    let should_run_ui = args.iter().any(|a| a == "--ui" || a == "ui");

    if should_run_ui {
        return handle_ui_command(&config).await;
    }

    // Roteia os subcomandos antes de abrir o banco (login não precisa de DB)
    match args.as_slice() {


        // ----------------------------------------------------------------
        // savevault restore — restaura um snapshot para o save original
        // ----------------------------------------------------------------
        [restore_cmd, list_cmd]
            if restore_cmd == "restore"
                && (list_cmd == "list" || list_cmd == "--list" || list_cmd == "-l") =>
        {
            handle_restore_list(&config)?;
            return Ok(());
        }

        [restore_cmd, id_str, flag, hash_str]
            if restore_cmd == "restore" && (flag == "--expected-hash" || flag == "-e") =>
        {
            handle_restore_command(&config, id_str, Some(hash_str.to_string())).await?;
            return Ok(());
        }

        [restore_cmd, flag, id_str]
            if restore_cmd == "restore" && (flag == "--snapshot" || flag == "-s") =>
        {
            handle_restore_command(&config, id_str, None).await?;
            return Ok(());
        }

        [restore_cmd, id_str] if restore_cmd == "restore" => {
            handle_restore_command(&config, id_str, None).await?;
            return Ok(());
        }

        // ----------------------------------------------------------------
        // savevault add / add-game — cadastro manual de jogo e local de save
        // ----------------------------------------------------------------
        [cmd, name, path_str] if cmd == "add" || cmd == "add-game" => {
            handle_add_game(&config, name, path_str)?;
            return Ok(());
        }

        // ----------------------------------------------------------------
        // savevault list / games — lista jogos e locais monitorados
        // ----------------------------------------------------------------
        [cmd] if cmd == "list" || cmd == "list-games" || cmd == "games" => {
            handle_list_games(&config)?;
            return Ok(());
        }

        _ => {}
    }

    run_resident_daemon(&config, config_dir, &args).await
}

async fn run_resident_daemon(
    config: &Config,
    config_dir: PathBuf,
    args: &[String],
) -> anyhow::Result<()> {

    tracing::info!("checkpoint iniciando...");

    // Abre o banco de dados
    let db = Database::open(&config.paths.database_path)?;
    
    // Limpeza de Banco de Dados (Pruning)
    let backups_repo = savevault_db::repositories::BackupsRepo::new(&db.conn);
    let sync_queue_repo = savevault_db::repositories::SyncQueueRepo::new(&db.conn);
    if let Ok(count) = backups_repo.prune_old_records(30) {
        if count > 0 { tracing::info!("Pruning: {} backups antigos removidos do DB.", count); }
    }
    if let Ok(count) = sync_queue_repo.prune_completed_or_old(30) {
        if count > 0 { tracing::info!("Pruning: {} itens da fila de sync removidos.", count); }
    }
    let games_repo = GamesRepo::new(&db.conn);
    let locations_repo = SaveLocationsRepo::new(&db.conn);

    tracing::info!(
        "Config carregada: max_versions={}, log_level={}",
        config.general.max_versions,
        config.general.log_level
    );
    tracing::info!("Banco de dados: {}", config.paths.database_path.display());

    // Discovery
    tracing::info!("Iniciando discovery...");
    let result = savevault_discovery::run_all(&config_dir).await?;

    // Persiste jogos
    let mut real_ids = std::collections::HashMap::new();
    for game in &result.games {
        let mut existing_opt = None;

        if let Some(ref platform_id) = game.platform_id {
            if let Ok(Some(existing)) = games_repo.find_by_platform_id(&game.platform, platform_id) {
                existing_opt = Some(existing);
            }
        }

        if existing_opt.is_none() {
            if let Ok(Some(existing)) = games_repo.find_by_name_and_platform(&game.name, &game.platform) {
                existing_opt = Some(existing);
            }
        }

        let real_id = match existing_opt {
            Some(existing) => existing.id,
            None => {
                games_repo.insert(game)?;
                game.id
            }
        };

        real_ids.insert(game.id, real_id);
    }

    // Persiste save locations
    let mut new_locations = 0usize;
    for loc in &result.save_locations {
        if let Some(&real_id) = real_ids.get(&loc.game_id) {
            let mut loc_to_insert = loc.clone();
            loc_to_insert.game_id = real_id;
            if locations_repo.upsert(&loc_to_insert).is_ok() {
                new_locations += 1;
            }
        }
    }

    if new_locations > 0 {
        tracing::info!(
            "{} caminho(s) de save novo(s) registrado(s) no banco.",
            new_locations
        );
    }

    tracing::info!(
        "checkpoint — {} jogo(s) registrado(s), {} caminho(s) de save detectado(s).",
        result.games.len(),
        result.save_locations.len()
    );



    // Se for solicitado modo pontual (--once ou scan), finaliza aqui
    if args.iter().any(|a| a == "--once" || a == "scan") {
        tracing::info!("Execução pontual concluída com sucesso.");
        return Ok(());
    }

    // ------------------------------------------------------------------------
    // MODO RESIDENTE: Monitoramento Contínuo e Sincronização em Background
    // ------------------------------------------------------------------------
    let all_locations = locations_repo.list_all()?;
    if all_locations.is_empty() {
        tracing::warn!("Nenhum local de save encontrado no banco para monitorar.");
    } else {
        tracing::info!(
            "Iniciando serviço de monitoramento para {} local(is) de save...",
            all_locations.len()
        );
    }

    let file_access = std::sync::Arc::new(savevault_monitor::file_access::OsFileAccess);
    let mut watcher_config = savevault_monitor::watcher::SaveWatcherConfig::default();
    watcher_config.debounce.debounce_duration = std::time::Duration::from_millis(config.general.debounce_ms);

    let (watcher, stable_rx) = match savevault_monitor::watcher::SaveWatcherService::start(
        watcher_config,
        all_locations,
        file_access,
    ) {
        Ok(res) => res,
        Err(e) => {
            tracing::error!("Falha ao inicializar o serviço de monitoramento: {}", e);
            return Err(anyhow::anyhow!("Erro ao iniciar watcher: {}", e));
        }
    };

    let db_conn = std::sync::Arc::new(tokio::sync::Mutex::new(db.conn));

    let reconciler_config = savevault_app::ReconciliationConfig::new(
        std::time::Duration::from_secs(config.general.reconciliation_interval_secs),
    );
    let reconciler = std::sync::Arc::new(savevault_app::Reconciler::new(
        db_conn.clone(),
        config.paths.backup_dir.clone(),
        config.general.max_versions,
    ));

    let (shutdown_tx, pipeline_shutdown_rx) = tokio::sync::watch::channel(false);
    let recon_shutdown_rx = shutdown_tx.subscribe();

    let reconciler_handle = tokio::spawn(savevault_app::run_reconciliation_loop(
        reconciler,
        reconciler_config,
        recon_shutdown_rx,
    ));

    tracing::info!("SaveVault monitorando em modo residente (Watcher + Reconciler). Pressione Ctrl+C para encerrar.");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Sinal de encerramento recebido (Ctrl+C). Iniciando shutdown gracioso...");
            let _ = shutdown_tx.send(true);
            watcher.shutdown().await;
            let _ = reconciler_handle.await;
            tracing::info!("Serviços de monitoramento e reconciliação encerrados com sucesso.");
        }
        res = savevault_app::pipeline::run_pipeline(
            db_conn.clone(),
            stable_rx,
            config.paths.backup_dir.clone(),
            config.general.max_versions,
            pipeline_shutdown_rx,
            true,
        ) => {
            if let Err(e) = res {
                tracing::error!("Erro no pipeline de eventos: {}", e);
            }
            let _ = shutdown_tx.send(true);
            watcher.shutdown().await;
            let _ = reconciler_handle.await;
        }
    }

    println!("SaveVault encerrado com sucesso.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Handlers de subcomandos
// ---------------------------------------------------------------------------



fn handle_add_game(config: &Config, name: &str, path_str: &str) -> anyhow::Result<()> {
    let raw_path = std::path::PathBuf::from(path_str);
    let location_type = if raw_path.is_file() {
        savevault_core::models::SaveLocationType::File
    } else {
        savevault_core::models::SaveLocationType::Directory
    };

    let db = Database::open(&config.paths.database_path)?;
    let games_repo = GamesRepo::new(&db.conn);
    let locations_repo = SaveLocationsRepo::new(&db.conn);

    let game_id = if let Ok(Some(existing)) =
        games_repo.find_by_name_and_platform(name, &savevault_core::models::Platform::Generic)
    {
        existing.id
    } else {
        let game = savevault_core::models::Game::new(
            name,
            savevault_core::models::Platform::Generic,
            None,
            savevault_core::models::Confidence::High,
        );
        games_repo.insert(&game)?;
        game.id
    };

    let loc = savevault_core::models::SaveLocation::new(game_id, raw_path.clone(), location_type);
    locations_repo.upsert(&loc)?;

    println!("✅ Jogo '{}' adicionado com sucesso!", name);
    println!("   ID: {}", game_id);
    println!("   Local de save: {}", raw_path.display());
    println!("   Monitoramento ativo na próxima execução do SaveVault.");
    Ok(())
}

fn handle_list_games(config: &Config) -> anyhow::Result<()> {
    let db = Database::open(&config.paths.database_path)?;
    let games_repo = GamesRepo::new(&db.conn);
    let locations_repo = SaveLocationsRepo::new(&db.conn);

    let games = games_repo.list_all()?;
    if games.is_empty() {
        println!("Nenhum jogo cadastrado no banco de dados.");
        println!("Dica: execute 'savevault --once' para rodar o discovery automático ou 'savevault add <nome> <caminho>' para adicionar manualmente.");
        return Ok(());
    }

    println!("\nJogos registrados (total: {}):", games.len());
    println!("{:-<70}", "");
    for g in games {
        let locs = locations_repo.list_by_game(g.id).unwrap_or_default();
        println!("• {} [{:?}] (id: {})", g.name, g.platform, g.id);
        if locs.is_empty() {
            println!("    (sem locais de save mapeados)");
        } else {
            for l in locs {
                println!("    📁 {}", l.path.display());
            }
        }
    }
    println!("{:-<70}\n", "");
    Ok(())
}


async fn handle_restore_command(
    config: &Config,
    id_str: &str,
    expected_hash: Option<String>,
) -> anyhow::Result<()> {
    let snapshot_id = match uuid::Uuid::parse_str(id_str) {
        Ok(id) => id,
        Err(_) => {
            eprintln!(
                "[FALHA] ID de snapshot inválido: '{}'. Esperado um UUID v4 (ex.: 550e8400-e29b-41d4-a716-446655440000).",
                id_str
            );
            return Err(anyhow::anyhow!("ID de snapshot inválido: {}", id_str));
        }
    };

    let db = Database::open(&config.paths.database_path)?;
    let request = match expected_hash {
        Some(hash) => savevault_app::RestoreRequest::with_expected_hash(snapshot_id, hash),
        None => savevault_app::RestoreRequest::new(snapshot_id),
    };

    match savevault_app::restore_snapshot(&db.conn, &config.paths.backup_dir, &request).await {
        Ok(report) => {
            println!("✅ [OK] Restauração concluída com sucesso!");
            println!("  Snapshot ID:       {}", report.snapshot_id);
            println!("  Versão:            v{}", report.version);
            println!("  Arquivo destino:   {}", report.dest_path.display());
            println!("  Bytes restaurados: {}", report.bytes_restored);
            println!("  Hash BLAKE3:       {}", report.restored_hash);
            match report.safety_backup_path {
                Some(p) => println!("  Safety Backup:     {}", p.display()),
                None => println!("  Safety Backup:     (nenhum arquivo pré-existente no destino)"),
            }
            Ok(())
        }
        Err(savevault_app::RestoreError::SnapshotNotFound(id)) => {
            eprintln!(
                "[FALHA] Snapshot '{}' não encontrado no banco de dados. Use 'savevault restore list' para ver os snapshots disponíveis.",
                id
            );
            Err(anyhow::anyhow!("Snapshot não encontrado"))
        }
        Err(savevault_app::RestoreError::FileNotFound(id)) => {
            eprintln!(
                "[FALHA] Registro de arquivo (ID '{}') associado ao snapshot não foi encontrado no banco de dados.",
                id
            );
            Err(anyhow::anyhow!("Arquivo associado não encontrado"))
        }
        Err(savevault_app::RestoreError::SaveLocationNotFound(id)) => {
            eprintln!(
                "[FALHA] Local de save autorizado (ID '{}') associado ao arquivo não foi encontrado no banco de dados.",
                id
            );
            Err(anyhow::anyhow!("SaveLocation não encontrado"))
        }
        Err(savevault_app::RestoreError::PhysicalSnapshotMissing(path)) => {
            eprintln!(
                "[FALHA] Arquivo físico do snapshot não encontrado em '{}'. O snapshot pode ter sido apagado externamente.",
                path.display()
            );
            Err(anyhow::anyhow!("Arquivo físico ausente"))
        }
        Err(savevault_app::RestoreError::InvalidSnapshotSource(path)) => {
            eprintln!(
                "[FALHA] O arquivo de snapshot em '{}' é inválido ou um diretório.",
                path.display()
            );
            Err(anyhow::anyhow!("Snapshot inválido"))
        }
        Err(savevault_app::RestoreError::PathSecurityViolation(reason)) => {
            eprintln!(
                "[FALHA DE SEGURANÇA] Bloqueada tentativa de restauração fora do diretório autorizado: {}",
                reason
            );
            Err(anyhow::anyhow!("Violação de segurança de caminho"))
        }
        Err(savevault_app::RestoreError::IntegrityCheckFailed(id, exp, act)) => {
            eprintln!(
                "[FALHA DE INTEGRIDADE] O snapshot '{}' está corrompido! Hash BLAKE3 calculado ('{}') difere do esperado ('{}'). O save atual não foi modificado.",
                id, act, exp
            );
            Err(anyhow::anyhow!("Integridade violada"))
        }
        Err(savevault_app::RestoreError::SafetyBackupFailed(path, err)) => {
            eprintln!(
                "[FALHA] Falha crítica ao criar safety backup em '{}': {}. O save original permanece intocado.",
                path.display(),
                err
            );
            Err(anyhow::anyhow!("Falha ao criar safety backup"))
        }
        Err(savevault_app::RestoreError::AtomicWriteFailed(path, err)) => {
            eprintln!(
                "[FALHA] Falha na escrita atômica do save em '{}': {}. O save original permanece protegido.",
                path.display(),
                err
            );
            Err(anyhow::anyhow!("Falha de escrita atômica"))
        }
        Err(savevault_app::RestoreError::PostWriteValidationFailed(path)) => {
            eprintln!(
                "[FALHA DE INTEGRIDADE] A validação pós-escrita falhou em '{}'. O conteúdo gravado diverge do snapshot. O safety backup foi preservado.",
                path.display()
            );
            Err(anyhow::anyhow!("Validação pós-escrita falhou"))
        }
        Err(e) => {
            eprintln!("[FALHA] Erro durante a restauração: {}", e);
            Err(anyhow::anyhow!(e))
        }
    }
}

fn handle_restore_list(config: &Config) -> anyhow::Result<()> {
    let db = Database::open(&config.paths.database_path)?;
    let mut stmt = db.conn.prepare(
        "SELECT b.id, b.version, b.created_at, f.path, g.name
         FROM backups b
         JOIN files f ON b.file_id = f.id
         JOIN save_locations sl ON f.save_location_id = sl.id
         JOIN games g ON sl.game_id = g.id
         ORDER BY b.created_at DESC
         LIMIT 50",
    )?;

    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let version: i64 = row.get(1)?;
        let created_at: String = row.get(2)?;
        let file_path: String = row.get(3)?;
        let game_name: String = row.get(4)?;
        Ok((id, version, created_at, file_path, game_name))
    })?;

    println!("================================================================================");
    println!(
        "{:<38} {:<15} {:<6} {:<20} ARQUIVO",
        "SNAPSHOT ID", "JOGO", "VERSÃO", "CRIADO EM"
    );
    println!("--------------------------------------------------------------------------------");
    let mut count = 0;
    for r in rows {
        let (id, version, created_at, file_path, game_name) = r?;
        let short_file = std::path::Path::new(&file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&file_path);
        let short_created = if created_at.len() > 19 {
            &created_at[..19]
        } else {
            &created_at
        };
        println!(
            "{:<38} {:<15} v{:<5} {:<20} {}",
            id, game_name, version, short_created, short_file
        );
        count += 1;
    }
    println!("================================================================================");
    println!(
        "Total: {} snapshot(s) listado(s). Para restaurar, use: checkpoint restore <SNAPSHOT_ID>",
        count
    );

    Ok(())
}

fn print_restore_help() {
    println!("Uso: checkpoint restore <snapshot-id> [opções]");
    println!("     checkpoint restore list");
    println!();
    println!("Restaura um snapshot versionado para o local de save original.");
    println!("Cria automaticamente um backup de segurança (safety backup) do estado atual");
    println!("antes de aplicar qualquer alteração no disco.");
    println!();
    println!("Argumentos:");
    println!("  <snapshot-id>            UUID do snapshot a ser restaurado");
    println!("  list                     Lista os snapshots disponíveis no banco de dados");
    println!();
    println!("Opções:");
    println!("  --snapshot, -s <id>      Especifica o UUID do snapshot (sintaxe alternativa)");
    println!("  --expected-hash <hash>   Valida o hash BLAKE3 do snapshot antes de restaurar");
    println!("  --help, -h               Exibe esta mensagem de ajuda");
    println!();
    println!("Exemplos:");
    println!("  checkpoint restore 550e8400-e29b-41d4-a716-446655440000");
    println!("  checkpoint restore --snapshot 550e8400-e29b-41d4-a716-446655440000");
    println!("  checkpoint restore list");
}

async fn handle_ui_command(config: &Config) -> anyhow::Result<()> {
    println!("================================================================================");
    println!("               checkpoint DEV-UI-01 — Modo de Teste Provisório                  ");
    println!("================================================================================");
    println!("  Banco de dados: {}", config.paths.database_path.display());
    println!("  Cofre backups:  {}", config.paths.backup_dir.display());
    println!("--------------------------------------------------------------------------------");

    let db = Database::open(&config.paths.database_path)?;
    let locations_repo = SaveLocationsRepo::new(&db.conn);
    let all_locations = locations_repo.list_all().unwrap_or_default();
    let db_conn = std::sync::Arc::new(tokio::sync::Mutex::new(db.conn));

    let reconciler_config = savevault_app::ReconciliationConfig::new(
        std::time::Duration::from_secs(config.general.reconciliation_interval_secs),
    );
    let reconciler = std::sync::Arc::new(savevault_app::Reconciler::new(
        db_conn.clone(),
        config.paths.backup_dir.clone(),
        config.general.max_versions,
    ));

    let state = std::sync::Arc::new(savevault_app::DevUiState::new(
        db_conn.clone(),
        config.paths.database_path.clone(),
        config.paths.backup_dir.clone(),
        Some(reconciler.clone()),
        std::time::Duration::from_secs(config.general.reconciliation_interval_secs),
    ));

    let (shutdown_tx, pipeline_shutdown_rx) = tokio::sync::watch::channel(false);
    let recon_shutdown_rx = shutdown_tx.subscribe();

    // Inicia reconciliador em background
    let reconciler_handle = tokio::spawn(savevault_app::run_reconciliation_loop(
        reconciler.clone(),
        reconciler_config,
        recon_shutdown_rx,
    ));

    // Prepara watcher se houver locais
    let file_access = std::sync::Arc::new(savevault_monitor::file_access::OsFileAccess);
    let mut watcher_config = savevault_monitor::watcher::SaveWatcherConfig::default();
    watcher_config.debounce.debounce_duration = std::time::Duration::from_millis(config.general.debounce_ms);
    let mut watcher_opt = if !all_locations.is_empty() {
        match savevault_monitor::watcher::SaveWatcherService::start(
            watcher_config,
            all_locations,
            file_access,
        ) {
            Ok((watcher, stable_rx)) => Some((watcher, stable_rx)),
            Err(e) => {
                tracing::warn!("Não foi possível iniciar watcher em modo DEV-UI: {}", e);
                None
            }
        }
    } else {
        None
    };

    let server_shutdown_rx = shutdown_tx.subscribe();
    let (_addr, server_handle) =
        savevault_app::start_dev_ui_server(state.clone(), 4040, server_shutdown_rx).await?;

    #[cfg(not(feature = "desktop"))]
    let ui_url = format!("http://{}", _addr);

    #[cfg(feature = "desktop")]
    {
        println!("================================================================================");
        println!("               checkpoint — Modo Desktop Ativo                                  ");
        println!("================================================================================");
        
        let state_clone = state.clone();
        std::thread::spawn(move || {
            if let Err(e) = savevault_app::ui::tauri_app::run_tauri_app(state_clone) {
                tracing::error!("Falha ao iniciar Tauri: {}", e);
            }
            std::process::exit(0);
        });
    }

    #[cfg(not(feature = "desktop"))]
    {
        println!("  DEV-UI ativa em: {}", ui_url);
        println!("  Abrindo navegador... (Pressione Ctrl+C para encerrar)");
        println!("================================================================================");

        #[cfg(windows)]
        {
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", &ui_url])
                .spawn();
        }
    }

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\nSinal de encerramento (Ctrl+C). Encerrando DEV-UI...");
        }
        res = async {
            if let Some((_w, rx)) = watcher_opt.take() {
                savevault_app::pipeline::run_pipeline(
                    db_conn.clone(),
                    rx,
                    config.paths.backup_dir.clone(),
                    config.general.max_versions,
                    pipeline_shutdown_rx,
                    false,
                )
                .await
            } else {
                std::future::pending::<anyhow::Result<()>>().await
            }
        } => {
            if let Err(e) = res {
                tracing::warn!("Pipeline finalizado: {}", e);
            }
        }
    }

    let _ = shutdown_tx.send(true);
    let _ = reconciler_handle.await;
    let _ = server_handle.await;

    println!("DEV-UI encerrada com sucesso.");
    Ok(())
}
