with open('crates/savevault-app/src/main.rs', 'r', encoding='utf-8') as f:
    content = f.read()

cleanup_code = '''    // Abre o banco de dados
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
'''

content = content.replace('    // Abre o banco de dados\n    let db = Database::open(&config.paths.database_path)?;\n', cleanup_code)

with open('crates/savevault-app/src/main.rs', 'w', encoding='utf-8') as f:
    f.write(content)
