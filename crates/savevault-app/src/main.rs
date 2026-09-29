//! Ponto de entrada principal do SaveVault.
//!
//! ## Uso
//! ```
//! savevault                    # Executa o discovery e persiste no banco
//! savevault restore <id>       # Restaura snapshot com safety backup e escrita atômica
//! savevault restore list       # Lista os snapshots disponíveis
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

fn main() -> anyhow::Result<()> {
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(async_main())
        })?
        .join()
        .map_err(|_| anyhow::anyhow!("Thread panicked"))?
}

async fn async_main() -> anyhow::Result<()> {
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

    // Roteia modo DEV-UI se solicitado
    if args.iter().any(|a| a == "--ui" || a == "ui") {
        handle_ui_command(&config).await?;
        return Ok(());
    }

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
        // savevault restore — restaura um snapshot para o save original
        // ----------------------------------------------------------------
        [restore_cmd, list_cmd]
            if restore_cmd == "restore"
                && (list_cmd == "list" || list_cmd == "--list" || list_cmd == "-l") =>
        {
            handle_restore_list(&config)?;
            return Ok(());
        }

        [restore_cmd, help_flag]
            if restore_cmd == "restore" && (help_flag == "--help" || help_flag == "-h") =>
        {
            print_restore_help();
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

        [restore_cmd] if restore_cmd == "restore" => {
            print_restore_help();
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
        "Total: {} snapshot(s) listado(s). Para restaurar, use: savevault restore <SNAPSHOT_ID>",
        count
    );

    Ok(())
}

fn print_restore_help() {
    println!("Uso: savevault restore <snapshot-id> [opções]");
    println!("     savevault restore list");
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
    println!("  savevault restore 550e8400-e29b-41d4-a716-446655440000");
    println!("  savevault restore --snapshot 550e8400-e29b-41d4-a716-446655440000");
    println!("  savevault restore list");
}

async fn handle_ui_command(config: &Config) -> anyhow::Result<()> {
    println!("================================================================================");
    println!("               SaveVault DEV-UI-01 — Modo de Teste Provisório                   ");
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
    let watcher_config = savevault_monitor::watcher::SaveWatcherConfig::default();
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
    let (addr, server_handle) =
        savevault_app::start_dev_ui_server(state.clone(), 4040, server_shutdown_rx).await?;

    let ui_url = format!("http://{}", addr);
    println!("  DEV-UI ativa em: {}", ui_url);
    println!("  Abrindo navegador... (Pressione Ctrl+C para encerrar)");
    println!("================================================================================");

    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", &ui_url])
            .spawn();
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
