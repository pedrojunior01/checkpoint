//! Ponto de entrada principal do SaveVault.
//!
//! ## Uso
//! ```
//! savevault                    # Executa o discovery e persiste no banco
//! savevault auth login         # Autentica no Google Drive via navegador
//! savevault auth logout        # Remove as credenciais salvas
//! savevault auth status        # Verifica se está autenticado
//! savevault sync               # Processa manualmente a fila de sincronização
//! ```

use savevault_core::config::Config;
use savevault_db::{
    repositories::{GamesRepo, SaveLocationsRepo},
    Database,
};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parseia os argumentos da linha de comando
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Inicializa o tracing
    let log_level = std::env::var("SAVEVAULT_LOG").unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&log_level)),
        )
        .init();

    // Carrega configuração e banco
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    let config_dir = PathBuf::from(&appdata).join("SaveVault");
    let config_path = config_dir.join("config.toml");
    let config = Config::load(&config_path).unwrap_or_default();
    config.save(&config_path)?;

    // Roteia os subcomandos antes de abrir o banco (login não precisa de DB)
    match args.as_slice() {
        // ----------------------------------------------------------------
        // savevault auth login / logout / status
        // ----------------------------------------------------------------
        [auth, sub] if auth == "auth" => {
            handle_auth_command(sub).await?;
            return Ok(());
        }

        // ----------------------------------------------------------------
        // savevault sync — processa a fila manualmente
        // ----------------------------------------------------------------
        [sync_cmd] if sync_cmd == "sync" => {
            let db = Database::open(&config.paths.database_path)?;
            let synced = savevault_sync::process_queue_google_drive(
                &db.conn,
                config.paths.backup_dir.clone(),
            )
            .await?;
            println!("Sincronização concluída: {} backup(s) enviado(s).", synced);
            return Ok(());
        }

        // ----------------------------------------------------------------
        // Comportamento padrão: discovery + persistência
        // ----------------------------------------------------------------
        _ => {}
    }

    tracing::info!("SaveVault iniciando...");

    // Abre o banco de dados
    let db = Database::open(&config.paths.database_path)?;
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
        let mut real_id = game.id;

        if let Some(ref platform_id) = game.platform_id {
            if let Ok(Some(existing)) = games_repo.find_by_platform_id(&game.platform, platform_id)
            {
                real_id = existing.id;
            } else {
                games_repo.insert(game)?;
            }
        } else {
            games_repo.insert(game)?;
        }

        real_ids.insert(game.id, real_id);
    }

    // Persiste save locations
    let mut new_locations = 0usize;
    for loc in &result.save_locations {
        if let Some(&real_id) = real_ids.get(&loc.game_id) {
            let mut loc_to_insert = loc.clone();
            loc_to_insert.game_id = real_id;
            if locations_repo.insert(&loc_to_insert).is_ok() {
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
        "SaveVault — {} jogo(s) registrado(s), {} caminho(s) de save detectado(s).",
        result.games.len(),
        result.save_locations.len()
    );

    // Tenta processar a fila de sync em background (falha silenciosa se offline)
    let synced =
        savevault_sync::process_queue_google_drive(&db.conn, config.paths.backup_dir.clone())
            .await
            .unwrap_or(0);

    if synced > 0 {
        tracing::info!("{} backup(s) sincronizado(s) com o Google Drive.", synced);
    }

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
    let watcher_config = savevault_monitor::watcher::SaveWatcherConfig::default();

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

async fn handle_auth_command(sub: &str) -> anyhow::Result<()> {
    let provider = match savevault_cloud::GoogleDriveProvider::from_env() {
        Some(p) => p,
        None => {
            eprintln!(
                "Erro: variáveis de ambiente não definidas.\n\
                Configure:\n  \
                SAVEVAULT_GOOGLE_CLIENT_ID=<seu_client_id>\n  \
                SAVEVAULT_GOOGLE_CLIENT_SECRET=<seu_client_secret>\n\n\
                Obtenha as credenciais em: https://console.cloud.google.com/"
            );
            return Err(anyhow::anyhow!("Credenciais não configuradas"));
        }
    };

    match sub {
        "login" => {
            if provider.is_authenticated() {
                println!("Já autenticado. Use 'auth logout' primeiro para reconectar.");
                return Ok(());
            }
            provider.login().await?;
            println!("✅ Login realizado com sucesso! Suas saves serão sincronizadas com o Google Drive.");
        }
        "logout" => {
            provider.logout()?;
            println!("✅ Credenciais removidas. Google Drive desvinculado.");
        }
        "status" => {
            if provider.is_authenticated() {
                println!("✅ Autenticado no Google Drive.");
            } else {
                println!("❌ Não autenticado. Execute: savevault auth login");
            }
        }
        other => {
            eprintln!(
                "Subcomando desconhecido: 'auth {}'. Use: login | logout | status",
                other
            );
        }
    }

    Ok(())
}
