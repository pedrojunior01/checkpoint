#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Ponto de entrada principal do SaveVault.
//!
//! ## Uso
//! ```
//! savevault                    # Executa o discovery e persiste no banco
//! savevault restore <id>       # Restaura snapshot com safety backup e escrita atômica
//! savevault restore list       # Lista os snapshots disponíveis
//! savevault add <nome> <caminho> # Adiciona um jogo e local de save manualmente
//! savevault list                # Lista todos os jogos cadastrados no banco

//! ```

use savevault_core::config::Config;
use savevault_db::{
    repositories::{GamesRepo, SaveLocationsRepo},
    Database,
};
use std::path::{Path, PathBuf};

fn main() -> anyhow::Result<()> {
    // Carrega as variáveis de ambiente do arquivo .env (se existir)
    dotenvy::dotenv().ok();
    // Parseia os argumentos da linha de comando
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Se solicitado modo silencioso/tray, oculta a janela do console imediatamente
    if args
        .iter()
        .any(|a| a == "--tray" || a == "--silent" || a == "--minimized")
    {
        savevault_app::tray::hide_console_window();
    }

    // Roteia subcomandos imediatos / help sem precisar subir runtime assíncrono
    match args.as_slice() {
        [flag] if flag == "--help" || flag == "-h" => {
            print_global_help();
            return Ok(());
        }
        [flag] if flag == "--version" || flag == "-V" => {
            println!("SaveVault v{}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        [autostart_cmd, sub] if autostart_cmd == "autostart" => {
            handle_autostart_cli(sub)?;
            return Ok(());
        }
        [autostart_cmd] if autostart_cmd == "autostart" => {
            handle_autostart_cli("status")?;
            return Ok(());
        }
        [auth_cmd, help_flag]
            if auth_cmd == "auth"
                && (help_flag == "--help" || help_flag == "-h" || help_flag == "help") =>
        {
            print_auth_help();
            return Ok(());
        }
        [restore_cmd, help_flag]
            if restore_cmd == "restore"
                && (help_flag == "--help" || help_flag == "-h" || help_flag == "help") =>
        {
            print_restore_help();
            return Ok(());
        }
        [restore_cmd] if restore_cmd == "restore" => {
            print_restore_help();
            return Ok(());
        }
        [cloud_cmd, help_flag]
            if cloud_cmd == "cloud"
                && (help_flag == "--help" || help_flag == "-h" || help_flag == "help") =>
        {
            print_cloud_help();
            return Ok(());
        }
        [cloud_cmd] if cloud_cmd == "cloud" => {
            print_cloud_help();
            return Ok(());
        }
        [notif_cmd, sub] if notif_cmd == "notifications" || notif_cmd == "notify" => {
            let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
            let config_dir = PathBuf::from(&appdata).join("SaveVault");
            let config_path = config_dir.join("config.toml");
            handle_notifications_cli(&config_path, sub)?;
            return Ok(());
        }
        [notif_cmd] if notif_cmd == "notifications" || notif_cmd == "notify" => {
            let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
            let config_dir = PathBuf::from(&appdata).join("SaveVault");
            let config_path = config_dir.join("config.toml");
            handle_notifications_cli(&config_path, "status")?;
            return Ok(());
        }
        [conflict_cmd, help_flag]
            if conflict_cmd == "conflict"
                && (help_flag == "--help" || help_flag == "-h" || help_flag == "help") =>
        {
            print_conflict_help();
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
        let should_run_ui = args.is_empty()
            || args.iter().any(|a| {
                a == "--ui"
                    || a == "ui"
                    || a == "desktop"
                    || a == "--desktop"
                    || a == "--app"
                    || a == "--tray"
                    || a == "--minimized"
                    || a == "--silent"
            });
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
fn run_desktop_main(args: Vec<String>) -> anyhow::Result<()> {
    use std::sync::Arc;

    let _single_instance = match savevault_app::SingleInstanceGuard::acquire(
        "Local\\SaveVault_SingleInstance_Mutex",
    ) {
        Some(guard) => guard,
        None => {
            tracing::info!(
                "SaveVault já está em execução. Trazendo janela existente para o primeiro plano."
            );
            savevault_app::focus_existing_instance();
            println!("SaveVault já está em execução.");
            return Ok(());
        }
    };

    let start_minimized = args
        .iter()
        .any(|a| a == "--tray" || a == "--minimized" || a == "--silent");

    #[cfg(windows)]
    {
        // Otimização agressiva de consumo de memória do WebView2 (Chromium):
        // - Limita heap do motor V8 a 64MB (evita que o Chromium retenha centenas de MB de GC inativo)
        // - Desativa telemetria em segundo plano, componentes desnecessários e sincronização
        // - Limita a 1 único processo de renderização
        // - Restringe limites de cache em disco e memória
        if std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").is_err() {
            std::env::set_var(
                "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
                "--disable-features=Translate,OptimizationHints,MediaRouter,CalculateNativeWinOcclusion,InterestFeedContentSuggestions \
                 --js-flags=--max-old-space-size=64 \
                 --renderer-process-limit=1 \
                 --disable-background-networking \
                 --disable-component-update \
                 --disable-sync \
                 --disable-breakpad \
                 --disable-default-apps \
                 --disk-cache-size=1048576 \
                 --media-cache-size=1048576",
            );
        }
    }

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
    let config_dir = std::path::PathBuf::from(&appdata).join("SaveVault");
    let config_path = config_dir.join("config.toml");
    let config = Config::load(&config_path).unwrap_or_default();
    let _ = config.save(&config_path);

    let db = savevault_db::Database::open(&config.paths.database_path)?;
    let locations_repo = savevault_db::repositories::SaveLocationsRepo::new(&db.conn);
    let all_locations = locations_repo.list_all().unwrap_or_default();
    let db_conn = Arc::new(tokio::sync::Mutex::new(db.conn));

    let reconciler = Arc::new(savevault_app::Reconciler::new(
        db_conn.clone(),
        config.paths.backup_dir.clone(),
        config.general.max_versions,
    ));

    let state = Arc::new(savevault_app::DevUiState::new(
        db_conn.clone(),
        config.paths.database_path.clone(),
        config.paths.backup_dir.clone(),
        Some(reconciler.clone()),
        std::time::Duration::from_secs(config.general.reconciliation_interval_secs),
    ));

    // Sobe o tokio em uma thread separada para o backend (watcher, reconciler, servidor HTTP)
    let state_bg = state.clone();
    let config_bg = config.clone();
    let db_conn_bg = db_conn.clone();
    let reconciler_bg = reconciler.clone();
    std::thread::Builder::new()
        .name("savevault-backend".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(8 * 1024 * 1024)
                .build()
                .expect("Falha ao criar runtime tokio");
            rt.block_on(async move {
                let reconciler_config = savevault_app::ReconciliationConfig::new(
                    std::time::Duration::from_secs(config_bg.general.reconciliation_interval_secs),
                );
                let (shutdown_tx, pipeline_shutdown_rx) = tokio::sync::watch::channel(false);
                let recon_shutdown_rx = shutdown_tx.subscribe();
                let server_shutdown_rx = shutdown_tx.subscribe();

                // Reconciler
                let _recon_handle = tokio::spawn(savevault_app::run_reconciliation_loop(
                    reconciler_bg.clone(),
                    reconciler_config,
                    recon_shutdown_rx,
                ));

                // Servidor HTTP (fallback dev-ui e API)
                let _ =
                    savevault_app::start_dev_ui_server(state_bg.clone(), 4040, server_shutdown_rx)
                        .await;

                // Watcher + Pipeline (F-03: sempre inicia watcher e cloud sync worker mesmo com banco vazio)
                let file_access = Arc::new(savevault_monitor::file_access::OsFileAccess);
                let mut watcher_config = savevault_monitor::watcher::SaveWatcherConfig::default();
                watcher_config.debounce.debounce_duration =
                    std::time::Duration::from_millis(config_bg.general.debounce_ms);

                match savevault_monitor::watcher::SaveWatcherService::start(
                    watcher_config,
                    all_locations,
                    file_access,
                ) {
                    Ok((_watcher, rx)) => {
                        let _ = savevault_app::pipeline::run_pipeline(
                            db_conn_bg.clone(),
                            rx,
                            config_bg.paths.backup_dir.clone(),
                            config_bg.general.max_versions,
                            pipeline_shutdown_rx,
                            false,
                        )
                        .await;
                    }
                    Err(e) => {
                        tracing::warn!("Watcher não pôde ser iniciado: {}", e);
                        let _ = tokio::signal::ctrl_c().await;
                    }
                }

                let _ = shutdown_tx.send(true);
            });
        })?;

    // Thread principal fica bloqueada rodando o Tauri (event loop nativo do Windows)
    println!("================================================================================");
    println!("               checkpoint — Iniciando Interface Desktop                         ");
    println!("================================================================================");

    savevault_app::ui::tauri_app::run_tauri_app(state, start_minimized)
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
    let config_dir = PathBuf::from(&appdata).join("SaveVault");
    let config_path = config_dir.join("config.toml");
    let mut config = Config::load(&config_path).unwrap_or_default();
    let _ = config.save(&config_path);
    savevault_app::notifications::init_from_config(&config.notifications);

    // Se compilado com desktop, modo GUI é o padrão quando não há argumentos.
    // Senão, exige `--ui`, `ui`, `desktop`, `--desktop` ou `--app`.
    #[cfg(feature = "desktop")]
    let should_run_ui = args.is_empty()
        || args.iter().any(|a| {
            a == "--ui" || a == "ui" || a == "--app" || a == "desktop" || a == "--desktop"
        });
    #[cfg(not(feature = "desktop"))]
    let should_run_ui = args
        .iter()
        .any(|a| a == "--ui" || a == "ui" || a == "--app" || a == "desktop" || a == "--desktop");

    if should_run_ui {
        return handle_ui_command(&config, &config_path).await;
    }

    // Roteia os subcomandos antes de abrir o banco (login não precisa de DB)
    match args.as_slice() {
        // ----------------------------------------------------------------
        // savevault auth — gerencia sincronização com a pasta da nuvem
        // ----------------------------------------------------------------
        [cmd, sub, path_or_target] if cmd == "auth" => {
            handle_auth_command(sub, Some(path_or_target), &mut config, &config_path).await?;
            return Ok(());
        }

        [cmd, sub] if cmd == "auth" => {
            handle_auth_command(sub, None, &mut config, &config_path).await?;
            return Ok(());
        }

        [cmd] if cmd == "auth" => {
            eprintln!("Uso: savevault auth <status | login [pasta] | logout>");
            return Ok(());
        }

        [login_cmd] if login_cmd == "login" => {
            handle_auth_command("login", None, &mut config, &config_path).await?;
            return Ok(());
        }

        // ----------------------------------------------------------------
        // savevault sync — processamento manual da fila
        // ----------------------------------------------------------------
        [cmd, provider] if cmd == "sync" => {
            handle_sync_command(&config, Some(provider)).await?;
            return Ok(());
        }

        [cmd] if cmd == "sync" => {
            handle_sync_command(&config, None).await?;
            return Ok(());
        }

        // ----------------------------------------------------------------
        // savevault cloud — listagem e recuperação em massa da nuvem (Disaster Recovery)
        // ----------------------------------------------------------------
        [cmd, sub, game_arg] if cmd == "cloud" && sub == "pull" => {
            handle_cloud_pull(&config, Some(game_arg)).await?;
            return Ok(());
        }

        [cmd, sub] if cmd == "cloud" && sub == "pull" => {
            handle_cloud_pull(&config, None).await?;
            return Ok(());
        }

        [cmd, sub] if cmd == "cloud" && sub == "list" => {
            handle_cloud_list(&config).await?;
            return Ok(());
        }

        [cmd] if cmd == "cloud" => {
            print_cloud_help();
            return Ok(());
        }

        // ----------------------------------------------------------------
        // savevault conflict — gerenciamento de conflitos multi-PC
        // ----------------------------------------------------------------
        [cmd, sub, target, flag, val]
            if cmd == "conflict" && sub == "resolve" && (flag == "--policy" || flag == "-p") =>
        {
            handle_conflict_resolve(&config, target, Some(val)).await?;
            return Ok(());
        }

        [cmd, sub, target] if cmd == "conflict" && sub == "resolve" => {
            handle_conflict_resolve(&config, target, None).await?;
            return Ok(());
        }

        [cmd, sub] if cmd == "conflict" && (sub == "list" || sub == "status") => {
            handle_conflict_list(&config).await?;
            return Ok(());
        }

        [cmd] if cmd == "conflict" => {
            handle_conflict_list(&config).await?;
            return Ok(());
        }

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

        [restore_cmd, id_str, flag]
            if restore_cmd == "restore"
                && (flag == "--cloud" || flag == "--from-cloud" || flag == "-c") =>
        {
            handle_restore_command(&config, id_str, None, true).await?;
            return Ok(());
        }

        [restore_cmd, flag, id_str]
            if restore_cmd == "restore"
                && (flag == "--cloud" || flag == "--from-cloud" || flag == "-c") =>
        {
            handle_restore_command(&config, id_str, None, true).await?;
            return Ok(());
        }

        [restore_cmd, id_str, flag, hash_str]
            if restore_cmd == "restore" && (flag == "--expected-hash" || flag == "-e") =>
        {
            handle_restore_command(&config, id_str, Some(hash_str.to_string()), false).await?;
            return Ok(());
        }

        [restore_cmd, flag, id_str]
            if restore_cmd == "restore" && (flag == "--snapshot" || flag == "-s") =>
        {
            handle_restore_command(&config, id_str, None, false).await?;
            return Ok(());
        }

        [restore_cmd, id_str] if restore_cmd == "restore" => {
            handle_restore_command(&config, id_str, None, false).await?;
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
    tracing::info!("SaveVault iniciando...");

    // Abre o banco de dados
    let db = Database::open(&config.paths.database_path)?;
    // Limpeza de Banco de Dados (Pruning)
    let backups_repo = savevault_db::repositories::BackupsRepo::new(&db.conn);
    let sync_queue_repo = savevault_db::repositories::SyncQueueRepo::new(&db.conn);
    if let Ok(count) = backups_repo.prune_old_records(30) {
        if count > 0 {
            tracing::info!("Pruning: {} backups antigos removidos do DB.", count);
        }
    }
    if let Ok(count) = sync_queue_repo.prune_completed_or_old(30) {
        if count > 0 {
            tracing::info!("Pruning: {} itens da fila de sync removidos.", count);
        }
    }
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

    let report = savevault_app::orchestrator::persist_discovery_result(&db.conn, &result)?;

    if !report.persisted_locations.is_empty() {
        tracing::info!(
            "{} caminho(s) de save registrado(s)/atualizado(s) no banco.",
            report.persisted_locations.len()
        );
    }

    tracing::info!(
        "SaveVault — {} jogo(s) registrado(s), {} caminho(s) de save detectado(s).",
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
    let _single_instance = match savevault_app::SingleInstanceGuard::acquire(
        "Local\\SaveVault_SingleInstance_Mutex",
    ) {
        Some(guard) => guard,
        None => {
            tracing::info!("SaveVault já está em execução em segundo plano. Trazendo janela existente para o primeiro plano.");
            savevault_app::focus_existing_instance();
            println!("SaveVault já está em execução em segundo plano.");
            return Ok(());
        }
    };

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
    watcher_config.debounce.debounce_duration =
        std::time::Duration::from_millis(config.general.debounce_ms);

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
        reconciler.clone(),
        reconciler_config,
        recon_shutdown_rx,
    ));

    // Inicializa a interface web DEV-UI em background para acesso via Tray ou navegador
    let config_path = config_dir.join("config.toml");
    let ui_state = std::sync::Arc::new(
        savevault_app::DevUiState::new(
            db_conn.clone(),
            config.paths.database_path.clone(),
            config.paths.backup_dir.clone(),
            Some(reconciler.clone()),
            std::time::Duration::from_secs(config.general.reconciliation_interval_secs),
        )
        .with_config_path(config_path.clone()),
    );

    let server_shutdown_rx = shutdown_tx.subscribe();
    let (ui_addr, server_handle) =
        match savevault_app::start_dev_ui_server(ui_state, 4040, server_shutdown_rx).await {
            Ok((addr, handle)) => {
                tracing::info!("Interface Web DEV-UI disponível em http://{}", addr);
                (Some(addr), Some(handle))
            }
            Err(e) => {
                tracing::warn!("Não foi possível iniciar DEV-UI em background: {}", e);
                (None, None)
            }
        };

    let ui_port = ui_addr.map(|a| a.port()).unwrap_or(4040);
    let _tray = savevault_app::tray::setup_tray(shutdown_tx.clone(), ui_port);

    // Se for primeira execução (setup pendente) e não estiver em modo silencioso/tray, abre a janela desktop
    if !config.general.setup_completed
        && !args
            .iter()
            .any(|a| a == "--tray" || a == "--silent" || a == "--minimized")
    {
        if let Some(addr) = ui_addr {
            let app_url = format!("http://{}", addr);
            tracing::info!(
                "Primeira execução detectada (setup pendente). Abrindo Assistente de Configuração..."
            );
            let _ = savevault_app::desktop::open_desktop_window(&app_url);
        }
    }

    tracing::info!("SaveVault monitorando em modo residente (Watcher + Reconciler + Sync + Tray). Pressione Ctrl+C para encerrar.");

    let mut tray_shutdown_rx = shutdown_tx.subscribe();

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Sinal de encerramento recebido (Ctrl+C). Iniciando shutdown gracioso...");
            let _ = shutdown_tx.send(true);
            watcher.shutdown().await;
            let _ = reconciler_handle.await;
            if let Some(h) = server_handle {
                let _ = h.await;
            }
            tracing::info!("Serviços de monitoramento e reconciliação encerrados com sucesso.");
        }
        _ = async {
            while tray_shutdown_rx.changed().await.is_ok() {
                if *tray_shutdown_rx.borrow() {
                    break;
                }
            }
        } => {
            tracing::info!("Sinal de encerramento recebido via Tray. Iniciando shutdown gracioso...");
            watcher.shutdown().await;
            let _ = reconciler_handle.await;
            if let Some(h) = server_handle {
                let _ = h.await;
            }
            tracing::info!("Serviços encerrados com sucesso.");
        }
        res = savevault_app::pipeline::run_pipeline_with_provider(
            db_conn.clone(),
            stable_rx,
            config.paths.backup_dir.clone(),
            config.general.max_versions,
            pipeline_shutdown_rx,
            true,
            &config.cloud.provider,
        ) => {
            if let Err(e) = res {
                tracing::error!("Erro no pipeline de eventos: {}", e);
            }
            let _ = shutdown_tx.send(true);
            watcher.shutdown().await;
            let _ = reconciler_handle.await;
            if let Some(h) = server_handle {
                let _ = h.await;
            }
        }
    }

    println!("SaveVault encerrado com sucesso.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Handlers de subcomandos
// ---------------------------------------------------------------------------

async fn handle_auth_command(
    sub: &str,
    target_path_arg: Option<&str>,
    config: &mut Config,
    config_path: &Path,
) -> anyhow::Result<()> {
    match sub {
        "status" => {
            println!("==================================================");
            println!("           SaveVault — Status da Nuvem            ");
            println!("==================================================");
            let detected = savevault_cloud::detect_cloud_folders();
            let local_status = if let Some(ref p) = config.cloud.local_sync_path {
                format!("✅ Ativo ({})", p.display())
            } else if let Some(first) = detected.first() {
                format!("💡 Detectado ({})", first.savevault_folder.display())
            } else {
                "⚪ Nenhuma pasta detectada".to_string()
            };
            println!("  Sincronização:      Pasta Local da Nuvem");
            println!("  Pasta Alvo:         {}", local_status);
            if !detected.is_empty() {
                println!("  Pastas Detectadas no Sistema:");
                for d in &detected {
                    println!("    - {}: {}", d.name, d.savevault_folder.display());
                }
            }
            println!("==================================================");
        }
        "login" | "configure" | "set" => {
            let path = target_path_arg
                .map(PathBuf::from)
                .or_else(|| config.cloud.local_sync_path.clone())
                .or_else(|| {
                    savevault_cloud::detect_cloud_folders()
                        .first()
                        .map(|d| d.savevault_folder.clone())
                });
            if let Some(p) = path {
                let _ = std::fs::create_dir_all(&p);
                config.cloud.provider = "local_folder".to_string();
                config.cloud.local_sync_path = Some(p.clone());
                let _ = config.save(config_path);
                println!(
                    "✅ Sincronização via Pasta Local da Nuvem ativa em: {}",
                    p.display()
                );
            } else {
                eprintln!("⚠️ Nenhuma pasta de nuvem informada ou detectada.");
                eprintln!("Uso: savevault auth login [caminho_da_pasta]");
            }
        }
        "logout" | "clear" => {
            config.cloud.local_sync_path = None;
            let _ = config.save(config_path);
            println!("✅ Pasta de sincronização em nuvem desvinculada.");
        }
        other => {
            eprintln!(
                "Subcomando desconhecido: '{}'. Use: status | login [pasta] | logout",
                other
            );
        }
    }
    Ok(())
}

async fn handle_sync_command(config: &Config, provider_arg: Option<&str>) -> anyhow::Result<()> {
    let db = Database::open(&config.paths.database_path)?;
    let provider = provider_arg.unwrap_or(&config.cloud.provider);
    println!("Sincronizando backups na nuvem (provedor: {})...", provider);
    match savevault_sync::process_queue_active_provider(
        &db.conn,
        config.paths.backup_dir.clone(),
        provider,
    )
    .await
    {
        Ok(count) => {
            println!(
                "✅ Sincronização concluída: {} backup(s) processado(s).",
                count
            );
        }
        Err(e) => {
            eprintln!("❌ Falha na sincronização: {}", e);
        }
    }
    Ok(())
}

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
    force_cloud: bool,
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
    let mut request = match expected_hash {
        Some(hash) => savevault_app::RestoreRequest::with_expected_hash(snapshot_id, hash),
        None => savevault_app::RestoreRequest::new(snapshot_id),
    };
    request = request.with_cloud_options(true, force_cloud, Some(config.cloud.provider.clone()));

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
                "[FALHA] Arquivo físico do snapshot não encontrado em '{}'. O snapshot pode ter sido apagado externamente e não possui backup na nuvem.",
                path.display()
            );
            Err(anyhow::anyhow!("Arquivo físico ausente"))
        }
        Err(savevault_app::RestoreError::CloudRecoveryFailed(reason)) => {
            eprintln!(
                "[FALHA NA NUVEM] Falha ao recuperar snapshot da nuvem: {}",
                reason
            );
            Err(anyhow::anyhow!("Falha na recuperação da nuvem"))
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

async fn handle_cloud_list(config: &Config) -> anyhow::Result<()> {
    let db = Database::open(&config.paths.database_path)?;
    let provider = savevault_sync::resolve_active_cloud_provider(
        &config.cloud.provider,
        &config.paths.backup_dir,
    );
    let _ =
        savevault_app::cloud_recovery::reconstruct_catalog_from_cloud(&db.conn, provider.as_ref())
            .await;
    let items =
        savevault_app::cloud_recovery::list_cloud_snapshots(&db.conn, &config.paths.backup_dir)?;

    println!("================================================================================");
    println!("               SaveVault — Snapshots Sincronizados na Nuvem                     ");
    println!("================================================================================");
    println!(
        "{:<38} {:<15} {:<6} {:<10} REMOTE ID / NUVEM",
        "SNAPSHOT ID", "JOGO", "VERSÃO", "STATUS"
    );
    println!("--------------------------------------------------------------------------------");

    let mut local_count = 0;
    let mut cloud_only_count = 0;

    for item in &items {
        let status_str = if item.local_exists {
            local_count += 1;
            "🟢 Local"
        } else {
            cloud_only_count += 1;
            "☁️ Nuvem"
        };
        let remote_display = item.drive_file_id.as_deref().unwrap_or("-");
        println!(
            "{:<38} {:<15} v{:<5} {:<10} {}",
            item.snapshot_id, item.game_name, item.version, status_str, remote_display
        );
    }

    println!("================================================================================");
    println!(
        "Total: {} snapshot(s) na nuvem ({} presentes no disco local, {} disponíveis apenas na nuvem).",
        items.len(),
        local_count,
        cloud_only_count
    );
    if cloud_only_count > 0 {
        println!("Para baixar todos os saves ausentes para este PC, use: savevault cloud pull");
    }

    Ok(())
}

async fn handle_cloud_pull(config: &Config, filter_game: Option<&str>) -> anyhow::Result<()> {
    println!("================================================================================");
    println!("           SaveVault — Recuperação de Saves da Nuvem (Disaster Recovery)        ");
    println!("================================================================================");
    println!("  Provedor Ativo:   {}", config.cloud.provider);
    println!("  Destino Backups:  {}", config.paths.backup_dir.display());
    if let Some(game) = filter_game {
        println!("  Filtro por jogo:  '{}'", game);
    }
    println!("--------------------------------------------------------------------------------");
    println!("Varrendo banco de dados e verificando arquivos ausentes...");

    let db = Database::open(&config.paths.database_path)?;
    let report = savevault_app::cloud_recovery::pull_snapshots_from_cloud(
        &db.conn,
        &config.paths.backup_dir,
        Some(&config.cloud.provider),
        filter_game,
    )
    .await?;

    for msg in &report.details {
        println!("  • {}", msg);
    }

    println!("--------------------------------------------------------------------------------");
    println!("Resumo da Recuperação:");
    println!("  Total escaneado:           {}", report.total_scanned);
    println!("  Já existentes no disco:    {}", report.already_local);
    println!("  Baixados com sucesso:      {}", report.downloaded);
    println!("  Falhas:                    {}", report.failed);
    println!("================================================================================");

    Ok(())
}

fn print_cloud_help() {
    println!("Uso: savevault cloud <subcomando> [opções]");
    println!();
    println!("Subcomandos:");
    println!("  list               Lista todos os snapshots registrados com cópia na nuvem");
    println!("  pull [jogo]        Baixa da nuvem todos os snapshots que faltam no disco local");
    println!("  help               Exibe esta mensagem de ajuda");
}

fn print_conflict_help() {
    println!("Uso: savevault conflict [list | resolve <jogo> [--policy <politica>]]");
    println!();
    println!("Gerencia e resolve divergências entre saves em múltiplos computadores.");
    println!();
    println!("Subcomandos:");
    println!("  list                         Lista o status de sincronização e divergências");
    println!("  resolve <jogo> [--policy ..] Aplica a política de resolução de conflitos");
    println!();
    println!("Políticas disponíveis (--policy):");
    println!(
        "  keep-both   (Padrão) Preserva o save local e arquiva snapshot remoto em conflicts/"
    );
    println!("  local       Mantém o save deste computador como prioritário");
    println!("  remote      Aplica a versão da nuvem (gerando safety backup local prévio)");
    println!("  newest      Compara datas de modificação e aplica o save mais recente");
    println!("  manual      Apenas registra o conflito sem alterar arquivos");
    println!();
    println!("Exemplos:");
    println!("  savevault conflict list");
    println!("  savevault conflict resolve Witcher --policy keep-both");
}

fn truncate_display(s: &str, max_len: usize) -> String {
    if s.chars().count() > max_len {
        let mut truncated: String = s.chars().take(max_len.saturating_sub(1)).collect();
        truncated.push('…');
        truncated
    } else {
        s.to_string()
    }
}

async fn handle_conflict_list(config: &Config) -> anyhow::Result<()> {
    let db = Database::open(&config.paths.database_path)?;
    let items = savevault_app::conflict::scan_conflicts(&db.conn, &config.paths.backup_dir).await?;

    println!("================================================================================");
    println!("             SaveVault — Sincronização & Conflitos Multi-PC                    ");
    println!("================================================================================");
    println!("  Dispositivo Local:     {}", config.general.device_name);
    println!(
        "  Política Padrão:       {:?}",
        config.general.conflict_policy
    );
    println!("--------------------------------------------------------------------------------");

    if items.is_empty() {
        println!("  Nenhum save com rastreamento no banco para verificação.");
        println!(
            "================================================================================"
        );
        return Ok(());
    }

    let mut conflicts_found = 0;
    println!(
        "{:<24} {:<20} {:<12} DETALHES",
        "JOGO", "STATUS", "VERSÃO NUVEM"
    );
    println!("{:-<24} {:-<20} {:-<12} {:-<22}", "", "", "", "");

    for item in &items {
        let (status_str, detail_str) = match item.classification {
            savevault_app::ConflictClassification::InSync => {
                ("🟢 Sincronizado", "Local e nuvem idênticos".to_string())
            }
            savevault_app::ConflictClassification::LocalAhead => (
                "🟡 Alteração Local",
                "Pronto para enviar à nuvem".to_string(),
            ),
            savevault_app::ConflictClassification::RemoteAhead => (
                "🔵 Nuvem mais Recente",
                format!("v{} disponível para pull", item.remote_version),
            ),
            savevault_app::ConflictClassification::Diverged => {
                conflicts_found += 1;
                (
                    "🔴 CONFLITO MULTI-PC",
                    "Ambos os computadores alteraram o save".to_string(),
                )
            }
        };

        println!(
            "{:<24} {:<20} {:<12} {}",
            truncate_display(&item.game_name, 23),
            status_str,
            if item.remote_version > 0 {
                format!("v{}", item.remote_version)
            } else {
                "-".into()
            },
            detail_str
        );
    }

    println!("================================================================================");
    if conflicts_found > 0 {
        println!(
            "⚠️  Foram detectados {} conflito(s) entre computadores!",
            conflicts_found
        );
        println!("    Para resolver de forma segura sem perda de dados, execute:");
        println!("      savevault conflict resolve <nome_do_jogo> [--policy keep-both|local|remote|newest]");
    } else {
        println!("✅ Nenhum conflito ativo detectado. Todos os saves estão íntegros.");
    }
    println!("================================================================================");

    Ok(())
}

async fn handle_conflict_resolve(
    config: &Config,
    game_filter: &str,
    policy_override: Option<&str>,
) -> anyhow::Result<()> {
    let db = Database::open(&config.paths.database_path)?;
    let items = savevault_app::conflict::scan_conflicts(&db.conn, &config.paths.backup_dir).await?;

    let policy = match policy_override {
        Some("keep-both" | "both" | "keep") => savevault_core::config::ConflictPolicy::KeepBoth,
        Some("local" | "local-wins") => savevault_core::config::ConflictPolicy::PreferLocal,
        Some("remote" | "cloud" | "cloud-wins") => {
            savevault_core::config::ConflictPolicy::PreferRemote
        }
        Some("newest" | "newer" | "recent") => savevault_core::config::ConflictPolicy::PreferNewest,
        Some("manual") => savevault_core::config::ConflictPolicy::Manual,
        Some(other) => {
            eprintln!(
                "Política desconhecida '{}'. Opções: keep-both, local, remote, newest, manual",
                other
            );
            return Ok(());
        }
        None => config.general.conflict_policy,
    };

    let target_lower = game_filter.trim().to_lowercase();
    let matching: Vec<_> = items
        .into_iter()
        .filter(|it| it.game_name.to_lowercase().contains(&target_lower))
        .collect();

    if matching.is_empty() {
        println!(
            "Nenhum save com conflito encontrado para '{}'.",
            game_filter
        );
        return Ok(());
    }

    println!("================================================================================");
    println!("              SaveVault — Resolução de Conflito Multi-PC                        ");
    println!("================================================================================");

    for item in matching {
        println!(
            "▶ Processando '{}' com política {:?}",
            item.game_name, policy
        );
        let res = savevault_app::conflict::resolve_conflict(
            policy,
            &item,
            &config.paths.backup_dir,
            &db.conn,
            Some(&config.cloud.provider),
        )
        .await?;

        match res {
            savevault_app::ConflictResolutionResult::KeptBoth {
                conflict_copy_path,
                message,
            } => {
                println!("  ✅ {}", message);
                println!("     Destino do conflito: {}", conflict_copy_path.display());
            }
            savevault_app::ConflictResolutionResult::LocalKept { message } => {
                println!("  ✅ {}", message);
            }
            savevault_app::ConflictResolutionResult::RemoteApplied {
                safety_backup_path,
                message,
            } => {
                println!("  ✅ {}", message);
                println!("     Safety Backup: {}", safety_backup_path.display());
            }
            savevault_app::ConflictResolutionResult::NewerApplied { winner, message } => {
                println!("  ✅ {} (Vencedor: {})", message, winner);
            }
            savevault_app::ConflictResolutionResult::ManualRequired { message } => {
                println!("  ⚠️ {}", message);
            }
        }
    }
    println!("================================================================================");

    Ok(())
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
        "Total: {} snapshot(s) listado(s). Para restaurar, use: savevault restore <SNAPSHOT_ID>",
        count
    );

    Ok(())
}

fn print_global_help() {
    println!("SaveVault — Sistema de Proteção e Sincronização de Saves");
    println!();
    println!("USO:");
    println!(
        "  savevault                                   Inicia o daemon de monitoramento contínuo"
    );
    println!("  savevault --tray                            Inicia minimizado na bandeja do sistema (silencioso)");
    println!("  savevault --once                            Executa descoberta e persiste sem manter ativo");
    println!("  savevault add <nome> <caminho>              Cadastra manualmente um jogo e pasta de save");
    println!("  savevault list                              Lista todos os jogos e diretórios monitorados");
    println!("  savevault restore <backup_id> [--cloud]     Restaura snapshot (com recuperação automática da nuvem)");
    println!(
        "  savevault restore list                      Lista snapshots históricos disponíveis"
    );
    println!("  savevault cloud list                        Lista snapshots salvos na nuvem e disponibilidade local");
    println!(
        "  savevault cloud pull [jogo]                 Baixa da nuvem snapshots ausentes no disco"
    );
    println!("  savevault auth [status|login|logout]        Gerencia pasta local de sincronização de nuvem");
    println!("  savevault autostart [enable|disable|status] Configura inicialização automática com o Windows");
    println!("  savevault notifications [on|off|all|status] Configura frequência das notificações (modo gamer)");
    println!("  savevault conflict [list|resolve]           Gerencia e resolve divergências entre múltiplos PCs");
    println!("  savevault desktop                           Inicia a Janela de Desktop integrada ultraleve");
    println!("  savevault --ui                              Inicia a interface de testes DEV-UI (porta 4040)");
    println!("  savevault --help                            Exibe esta ajuda");
    println!("  savevault --version                         Exibe a versão do SaveVault");
}

fn handle_autostart_cli(sub: &str) -> anyhow::Result<()> {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    let config_dir = PathBuf::from(&appdata).join("SaveVault");
    let config_path = config_dir.join("config.toml");
    let mut config = Config::load(&config_path).unwrap_or_default();

    match sub.to_ascii_lowercase().as_str() {
        "enable" | "on" | "true" | "1" => {
            let status = savevault_core::autostart::enable(None)?;
            config.general.autostart = true;
            let _ = config.save(&config_path);

            println!("✅ Inicialização automática com o Windows HABILITADA com sucesso!");
            if let Some(cmd) = status.command {
                println!("   Comando:  {}", cmd);
            }
            if let Some(exe) = status.executable_path {
                println!("   Binário:  {}", exe.display());
            }
            println!(
                "   Registro: HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\\SaveVault"
            );
            println!("   O SaveVault será iniciado automaticamente em segundo plano ao ligar o computador.");
        }
        "disable" | "off" | "false" | "0" => {
            let _ = savevault_core::autostart::disable()?;
            config.general.autostart = false;
            let _ = config.save(&config_path);

            println!("✅ Inicialização automática com o Windows DESABILITADA.");
            println!(
                "   O SaveVault não será mais iniciado automaticamente ao ligar o computador."
            );
        }
        "status" | "check" | "info" => {
            let status = savevault_core::autostart::status()?;
            println!(
                "================================================================================"
            );
            println!("INICIALIZAÇÃO AUTOMÁTICA COM O WINDOWS (AUTOSTART)");
            println!(
                "--------------------------------------------------------------------------------"
            );
            if status.enabled {
                println!("Status:     🟢 HABILITADA");
                if let Some(cmd) = status.command {
                    println!("Comando:    {}", cmd);
                }
                if let Some(path) = status.executable_path {
                    println!("Executável: {}", path.display());
                }
                println!("Registro:   HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\\SaveVault");
                println!();
                println!("Para desabilitar, execute: savevault autostart disable");
            } else {
                println!("Status:     ⚪ DESABILITADA");
                println!("Registro:   Nenhum registro ativo em HKCU\\...\\Run\\SaveVault");
                println!();
                println!("Para habilitar, execute: savevault autostart enable");
            }
            println!(
                "================================================================================"
            );
        }
        other => {
            eprintln!("Subcomando desconhecido: 'autostart {}'.", other);
            eprintln!("Uso:");
            eprintln!("  savevault autostart enable   Habilita a inicialização com o Windows");
            eprintln!("  savevault autostart disable  Desabilita a inicialização com o Windows");
            eprintln!("  savevault autostart status   Verifica se está ativo");
        }
    }

    Ok(())
}

fn handle_notifications_cli(config_path: &Path, sub: &str) -> anyhow::Result<()> {
    let mut config = Config::load(config_path).unwrap_or_default();
    match sub.to_ascii_lowercase().as_str() {
        "off" | "disable" | "false" => {
            config.notifications.enabled = false;
            config.save(config_path)?;
            savevault_app::notifications::init_from_config(&config.notifications);
            println!("🔕 Notificações do Windows Toast DESATIVADAS.");
            println!("   O SaveVault continuará monitorando e salvando em segundo plano de forma 100% silenciosa.");
        }
        "on" | "enable" | "gamer" | "default" | "quiet" => {
            config.notifications.enabled = true;
            config.notifications.notify_on_save = false; // Silencioso ao jogar
            config.notifications.notify_on_sync = true;
            config.notifications.notify_on_restore = true;
            config.save(config_path)?;
            savevault_app::notifications::init_from_config(&config.notifications);
            println!("🔔 Modo Gamer / Equilibrado ATIVADO.");
            println!("   • Auto-saves durante o jogo: ⚪ Silencioso (sem popups na tela)");
            println!("   • Sincronização na nuvem:   🟢 Ativo (com cooldown anti-spam)");
            println!("   • Restauração de saves:     🟢 Ativo");
        }
        "all" | "verbose" => {
            config.notifications.enabled = true;
            config.notifications.notify_on_save = true;
            config.notifications.notify_on_sync = true;
            config.notifications.notify_on_restore = true;
            config.save(config_path)?;
            savevault_app::notifications::init_from_config(&config.notifications);
            println!("🔔 Notificações Detalhadas ATIVADAS.");
            println!("   • Auto-saves:             🟢 Ativo (com cooldown anti-spam)");
            println!("   • Sincronização na nuvem: 🟢 Ativo");
            println!("   • Restauração de saves:   🟢 Ativo");
        }
        "status" => {
            println!(
                "================================================================================"
            );
            println!(
                "              SaveVault — Status de Notificações do Windows                     "
            );
            println!(
                "================================================================================"
            );
            let state_str = if config.notifications.enabled {
                "🟢 ATIVAS"
            } else {
                "⚪ DESATIVADAS"
            };
            println!("  Status Geral:              {}", state_str);
            println!(
                "  Notificar a cada save:     {}",
                if config.notifications.notify_on_save {
                    "🟢 Sim"
                } else {
                    "⚪ Não (Silencioso para não atrapalhar no jogo)"
                }
            );
            println!(
                "  Notificar sincronização:   {}",
                if config.notifications.notify_on_sync {
                    "🟢 Sim"
                } else {
                    "⚪ Não"
                }
            );
            println!(
                "  Notificar restauração:     {}",
                if config.notifications.notify_on_restore {
                    "🟢 Sim"
                } else {
                    "⚪ Não"
                }
            );
            println!(
                "  Cooldown anti-spam:        {} segundos",
                config.notifications.cooldown_secs
            );
            println!(
                "--------------------------------------------------------------------------------"
            );
            println!("Comandos disponíveis:");
            println!(
                "  savevault notifications on       Ativa o modo equilibrado (silencioso no jogo)"
            );
            println!("  savevault notifications off      Desativa todas as notificações");
            println!("  savevault notifications all      Ativa notificações detalhadas");
            println!(
                "================================================================================"
            );
        }
        other => {
            eprintln!("Subcomando desconhecido: 'notifications {}'.", other);
            eprintln!("Uso:");
            eprintln!(
                "  savevault notifications on       Ativa o modo equilibrado (silencioso no jogo)"
            );
            eprintln!("  savevault notifications off      Desativa todas as notificações");
            eprintln!("  savevault notifications all      Ativa notificações detalhadas");
            eprintln!("  savevault notifications status   Exibe configuração atual");
        }
    }
    Ok(())
}

fn print_restore_help() {
    println!("Uso: savevault restore <snapshot-id> [opções]");
    println!("     savevault restore list");
    println!();
    println!("Restaura um snapshot versionado para o local de save original.");
    println!("Se o arquivo físico estiver ausente do disco local, o SaveVault o baixa");
    println!("automaticamente da nuvem (Google Drive ou OneDrive).");
    println!("Cria automaticamente um backup de segurança (safety backup) do estado atual");
    println!("antes de aplicar qualquer alteração no disco.");
    println!();
    println!("Argumentos:");
    println!("  <snapshot-id>            UUID do snapshot a ser restaurado");
    println!("  list                     Lista os snapshots disponíveis no banco de dados");
    println!();
    println!("Opções:");
    println!("  --snapshot, -s <id>      Especifica o UUID do snapshot (sintaxe alternativa)");
    println!("  --cloud, -c              Força o download da versão remota da nuvem");
    println!("  --expected-hash <hash>   Valida o hash BLAKE3 do snapshot antes de restaurar");
    println!("  --help, -h               Exibe esta mensagem de ajuda");
    println!();
    println!("Exemplos:");
    println!("  savevault restore 550e8400-e29b-41d4-a716-446655440000");
    println!("  savevault restore 550e8400-e29b-41d4-a716-446655440000 --cloud");
    println!("  savevault restore list");
}

async fn handle_ui_command(config: &Config, config_path: &Path) -> anyhow::Result<()> {
    println!("================================================================================");
    println!("               SaveVault — Interface Desktop Integrada (DEV-UI)                 ");
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

    let state = std::sync::Arc::new(
        savevault_app::DevUiState::new(
            db_conn.clone(),
            config.paths.database_path.clone(),
            config.paths.backup_dir.clone(),
            Some(reconciler.clone()),
            std::time::Duration::from_secs(config.general.reconciliation_interval_secs),
        )
        .with_config_path(config_path.to_path_buf()),
    );

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
    watcher_config.debounce.debounce_duration =
        std::time::Duration::from_millis(config.general.debounce_ms);
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

    #[cfg(feature = "desktop")]
    {
        println!(
            "================================================================================"
        );
        println!(
            "               checkpoint — Modo Desktop Ativo                                  "
        );
        println!(
            "================================================================================"
        );

        let state_clone = state.clone();
        std::thread::spawn(move || {
            if let Err(e) = savevault_app::ui::tauri_app::run_tauri_app(state_clone, false) {
                tracing::error!("Falha ao iniciar Tauri: {}", e);
            }
            std::process::exit(0);
        });
    }

    #[cfg(not(feature = "desktop"))]
    let _tray = savevault_app::tray::setup_tray(shutdown_tx.clone(), _addr.port());

    #[cfg(not(feature = "desktop"))]
    let ui_url = format!("http://{}", _addr);
    #[cfg(not(feature = "desktop"))]
    {
        println!("  Interface SaveVault ativa em: {}", ui_url);
        println!("  Iniciando Janela Desktop ultraleve... (Pressione Ctrl+C ou feche a janela/bandeja para encerrar)");
        println!(
            "================================================================================"
        );

        let _ = savevault_app::desktop::open_desktop_window(&ui_url);
    }

    let mut tray_shutdown_rx = shutdown_tx.subscribe();

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\nSinal de encerramento (Ctrl+C). Encerrando DEV-UI...");
        }
        _ = async {
            while tray_shutdown_rx.changed().await.is_ok() {
                if *tray_shutdown_rx.borrow() {
                    break;
                }
            }
        } => {
            println!("\nSinal de encerramento recebido via Tray. Encerrando DEV-UI...");
        }
        res = async {
            if let Some((_w, rx)) = watcher_opt.take() {
                savevault_app::pipeline::run_pipeline_with_provider(
                    db_conn.clone(),
                    rx,
                    config.paths.backup_dir.clone(),
                    config.general.max_versions,
                    pipeline_shutdown_rx,
                    false,
                    &config.cloud.provider,
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

fn print_auth_help() {
    println!("Uso: savevault auth <status | login [pasta] | logout>");
    println!();
    println!("Subcomandos:");
    println!("  status          Verifica a pasta de nuvem configurada e detectada");
    println!("  login [pasta]   Configura a pasta local da nuvem para sincronização");
    println!("  logout          Desvincula a pasta de nuvem configurada");
    println!("  help            Exibe esta mensagem de ajuda");
}
