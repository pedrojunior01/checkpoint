# AUDIT REMEDIATION TRACKER — SAVEVAULT

Este documento acompanha permanentemente o status de remediação de todos os findings identificados na auditoria completa e adversarial do SaveVault.

Estados:
- `NOT_STARTED`: Problema mapeado, aguardando início da remediação.
- `INVESTIGATING`: Análise de causa raiz e impactos colaterais em andamento.
- `IN_PROGRESS`: Alterações de código e arquitetura sendo implementadas.
- `FIXED`: Código corrigido, aguardando testes de validação.
- `TESTED`: Testes unitários/integração implementados e aprovados.
- `VERIFIED`: Validado de ponta a ponta sem regressões na suíte do workspace.
- `BLOCKED`: Requer validação manual ou dependência externa.
- `WONT_FIX`: Decisão justificada de não alteração.

---

## Tabela Geral de Acompanhamento

| ID | Título | Severidade | Categoria | Status | Arquivos Principais | Testes |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **BUG-01** | Sync Queue IPC quebrado (colunas SQL e tipos) | CRITICAL | IPC / UI / DB | VERIFIED | `savevault-app/src/ui/state.rs`, `frontend/app.js` | `dev_ui_integration.rs` |
| **BUG-02** | Duplicação de jogos sem platform_id | HIGH | DB / Persistência | VERIFIED | `savevault-db/src/repositories/games.rs`, `main.rs`, `state.rs` | `db/src/tests.rs`, `dev_ui_integration.rs` |
| **BUG-03** | Mutex SQLite retido durante operações de rede | HIGH | Concorrência / Async | VERIFIED | `savevault-sync/src/lib.rs`, `savevault-app/src/pipeline.rs`, `main.rs` | `sync/src/tests.rs`, `pipeline_integration.rs` |
| **BUG-04** | SaveLocationType hardcoded como Directory | HIGH | Descoberta / Filesystem | VERIFIED | `savevault-discovery/src/lib.rs` | `discovery/src/tests.rs` |
| **BUG-05** | Discovery incompleto para jogos não-Steam | HIGH | Descoberta | VERIFIED | `savevault-discovery/src/lib.rs` | `discovery/src/tests.rs` |
| **BUG-06** | Integridade da tabela `files` e case-folding | HIGH | DB / Filesystem | VERIFIED | `savevault-db/src/migrations.rs`, `files.rs` | `db/src/tests.rs` |
| **BUG-07** | Consumo de RAM e multipart >5MB no Drive | MEDIUM | Cloud / Performance | VERIFIED | `savevault-cloud/src/google_drive.rs` | `cloud/tests/google_drive_mock.rs` |
| **BUG-08** | Canonicalização Unicode com to_lowercase | MEDIUM | Filesystem / Unicode | VERIFIED | `savevault-core/src/path.rs` | `core/src/path.rs` (tests) |
| **BUG-09** | Restore duplicado e morto em SnapshotManager | LOW | Arquitetura / Código Morto | VERIFIED | `savevault-backup/src/snapshot.rs`, `savevault-app/src/restore.rs` | `hasher.rs`, `dev_ui_integration.rs` |
| **BUG-10** | Servidor HTTP legado após migração para Tauri | LOW | Código Morto | VERIFIED | `savevault-app/src/ui/server.rs` (removido), `assets.rs` (removido), `ui/mod.rs` | `dev_ui_integration.rs` |
| **BUG-11** | Shutdown abrupto no System Tray do Tauri | MEDIUM | Lifecycle / Shutdown | VERIFIED | `savevault-app/src/ui/tauri_app.rs`, `state.rs`, `main.rs` | `dev_ui_integration.rs` |
| **BUG-12** | Migrations não atômicas sem transação | LOW | DB / Migrations | VERIFIED | `savevault-db/src/migrations.rs` | `db/src/tests.rs` |
| **BUG-13** | GOG detector é stub vazio | LOW | Descoberta / Registry | VERIFIED | `savevault-discovery/src/gog.rs` | `discovery/src/tests.rs` |
| **BUG-14** | Localização do Steam sem Registry | LOW | Descoberta / Registry | VERIFIED | `savevault-discovery/src/steam.rs` | `discovery/src/tests.rs` |
| **BUG-15** | Salvamento de config.toml não atômico | LOW | Config / Filesystem | VERIFIED | `savevault-core/src/config.rs` | `core/src/tests.rs` |

---

## Log Detalhado de Remediação

### [BUG-01]
- **Status**: VERIFIED
- **Root cause**: Consulta SQL em `DevUiState::get_sync_queue` buscava colunas inexistentes `sq.retry_count` e `sq.last_error`, e lia `id` como `i64` quando no SQLite schema a coluna se chama `attempts` e `id` é `TEXT PRIMARY KEY` (UUID).
- **Changed**:
  - `crates/savevault-app/src/ui/state.rs`: `SyncQueueItemView` agora possui `pub id: Uuid` e `pub retry_count: u32` (mapeando a coluna `attempts`). A consulta SQL foi corrigida para selecionar `sq.id, sq.backup_id, sq.operation, sq.status, sq.attempts, sq.created_at, f.path, g.name`.
  - `crates/savevault-app/ui/frontend/app.js`: Ajustada a renderização para suportar UUID no `item.id`.
- **Tests**: `test_get_sync_queue_returns_expected_items` em `crates/savevault-app/tests/dev_ui_integration.rs`.
- **Validation**: Teste de integração executado com sucesso: `test test_get_sync_queue_returns_expected_items ... ok`.
- **Remaining**: Nenhum.

### [BUG-02]
- **Status**: VERIFIED
- **Root cause**: Jogos sem `platform_id` (ex: emuladores e genéricos) eram inseridos incondicionalmente a cada varredura de discovery gerando novos UUIDs a cada inicialização/scan. Além disso, `locations_repo.insert()` falhava na inserção de locais repetidos em vez de atualizar via `upsert()`.
- **Changed**:
  - `crates/savevault-db/src/repositories/games.rs`: Implementado `find_by_name_and_platform(&self, name: &str, platform: &Platform)` com `name = ?1 COLLATE NOCASE` e `trim()`.
  - `crates/savevault-app/src/main.rs`: Na persistência do discovery, antes de inserir um jogo sem `platform_id`, busca por `find_by_name_and_platform`, reutilizando o UUID existente se já cadastrado. Os save locations agora utilizam `locations_repo.upsert(&loc)`.
  - `crates/savevault-app/src/ui/state.rs`: Atualizado `trigger_discovery` para a mesma lógica idempotente com `find_by_name_and_platform` e `locations_repo.upsert`.
- **Tests**:
  - `test_find_by_name_and_platform_case_insensitive_and_trimmed` em `crates/savevault-db/src/tests.rs`.
  - `test_deduplication_games_and_locations_multiple_scans` em `crates/savevault-db/src/tests.rs`.
- **Validation**: Executado `cargo test -p savevault-db` (37 testes aprovados) e `cargo test -p savevault-app` (20 testes aprovados).
- **Remaining**: Nenhum.

### [BUG-03]
- **Status**: VERIFIED
- **Root cause**: O `MutexGuard<Connection>` do SQLite era mantido por longos períodos durante requisições de rede, uploads pesados para o Google Drive e backoff com `tokio::time::sleep`, bloqueando completamente o pipeline de eventos, reconciliador e requisições da UI.
- **Changed**:
  - `crates/savevault-sync/src/lib.rs`: Desacoplado o ciclo de vida do lock do banco das operações de rede através de `prepare_upload_paths` e do método `process_queue_with_mutex`. O padrão implementado agora é: (1) Leitura curta no DB -> libera lock; (2) Backoff sleep sem lock; (3) Upload HTTP no Google Drive sem lock; (4) Transação atômica curta de escrita no DB -> libera lock.
  - `crates/savevault-app/src/pipeline.rs`: Removida a aquisição prévia e duradoura do lock em `run_pipeline`, passando a referência compartilhada `&db` diretamente para `savevault_sync::process_queue_google_drive`.
  - `crates/savevault-app/src/main.rs`: Atualizadas as chamadas pontuais da CLI para `process_queue_google_drive_conn`.
- **Tests**:
  - `test_concurrent_database_operations_during_sync_upload` em `crates/savevault-sync/src/tests.rs` (comprova que inserções e leituras concorrentes no banco executam em <100ms enquanto um upload de 150ms está em andamento).
  - `test_end_to_end_pipeline_from_event_to_cloud` em `crates/savevault-app/tests/pipeline_integration.rs`.
- **Validation**:
  - `cargo test -p savevault-sync`: 13 testes aprovados (incluindo teste de concorrência).
  - `cargo test -p savevault-app --test pipeline_integration`: 2 testes aprovados.
- **Remaining**: Nenhum.

### [BUG-08]
- **Status**: VERIFIED
- **Root cause**: `canonicalize_windows_path_str` em `savevault-core/src/path.rs` utilizava `.to_ascii_lowercase()`, que não faz case-folding de caracteres Unicode não-ASCII (ex: `Ação` vs `AÇÃO`, `München` vs `MÜNCHEN`, Cirílico, CJK), violando a semântica case-insensitive do NTFS/FAT do Windows.
- **Changed**:
  - `crates/savevault-core/src/path.rs`: Substituído `.to_ascii_lowercase()` por `.to_lowercase()` para case-folding Unicode completo de acordo com o padrão do filesystem.
- **Tests**:
  - `test_canonicalize_unicode_case_folding` em `crates/savevault-core/src/path.rs` cobrindo Português (`Ação`/`AÇÃO`), Alemão (`München`/`MÜNCHEN`), Cirílico (`Игры`/`ИГРЫ`) e CJK.
- **Validation**: Executado `cargo test -p savevault-core` (todos os testes aprovados).
- **Remaining**: Nenhum.

### [BUG-04]
- **Status**: VERIFIED
- **Root cause**: `run_all` em `savevault-discovery/src/lib.rs` forçava `SaveLocationType::Directory` para todas as localizações de saves, ignorando extensões de arquivo existentes, arquivos únicos e padrões de glob com curingas (`*`, `?`).
- **Changed**:
  - `crates/savevault-discovery/src/lib.rs`: Conectada a função `determine_location_type(path)` que inspeciona o sistema de arquivos e a sintaxe do caminho:
    - Se contém glob (`*`, `?`, `[`), classifica como `Glob`;
    - Se o caminho existe fisicamente no disco como arquivo, ou se termina em extensão comum de save/config (`.sav`, `.dat`, `.json`, `.xml`, etc.), classifica como `File`;
    - Se existe como diretório ou não tem extensão de arquivo, classifica como `Directory`.
- **Tests**: `test_determine_location_type_detection` em `crates/savevault-discovery/src/tests.rs` cobrindo globs, arquivos reais, arquivos com extensão e diretórios.
- **Validation**: Executado `cargo test -p savevault-discovery` (28 testes aprovados).
- **Remaining**: Nenhum.

### [BUG-05]
- **Status**: VERIFIED
- **Root cause**: `run_all` em `savevault-discovery/src/lib.rs` executava a detecção de save locations via `LudusaviSaveDetector` apenas para jogos cujo `platform_id` era numérico (AppID da Steam). Jogos de GOG, emuladores e genéricos eram descobertos mas tinham suas save locations completamente ignoradas. Além disso, havia um pipeline paralelo duplicado que não aproveitava a resolução completa de manifest.
- **Changed**:
  - `crates/savevault-discovery/src/lib.rs`: Refatorado `run_all` para incluir jogos Steam em `games` e rodar a detecção de `SaveLocation` através do `LudusaviSaveDetector::detect_save_locations` para TODOS os jogos mapeados (Steam, GOG, Emuladores e Genéricos), associando os locais descobertos aos jogos respectivos com tipos inferidos corretamente.
- **Tests**: `test_discover_save_locations_for_non_steam_games` em `crates/savevault-discovery/src/tests.rs`.
- **Validation**: Executado `cargo test -p savevault-discovery` (28 testes aprovados).
- **Remaining**: Nenhum.

### [BUG-06]
- **Status**: VERIFIED
- **Root cause**: Caminhos na tabela `files` não eram normalizados com case-folding semântico do Windows nem protegidos por um índice único case-insensitive. Isso permitia que caminhos como `C:\Saves\Game1` e `c:\saves\game1` fossem inseridos como duas linhas diferentes na tabela `files`, causando descontinuidade de histórico de backups e duplicatas.
- **Changed**:
  - `crates/savevault-db/src/migrations.rs`: Adicionada `MIGRATION_005` que:
    1. Consolida backups que apontam para arquivos duplicados por casing diferente para um único `file_id`;
    2. Deleta linhas duplicadas de `files`;
    3. Cria o índice único case-insensitive: `CREATE UNIQUE INDEX IF NOT EXISTS idx_files_path_nocase ON files(path COLLATE NOCASE)`.
  - `crates/savevault-db/src/repositories/files.rs`:
    - `find_by_path` agora usa `WHERE path = ?1 COLLATE NOCASE`.
    - `upsert` agora usa `ON CONFLICT(path COLLATE NOCASE) DO UPDATE SET ... RETURNING id`.
- **Tests**: `test_files_case_insensitive_identity_and_upsert_deduplication` em `crates/savevault-db/src/tests.rs`.
- **Validation**: Executado `cargo test -p savevault-db` (39 testes aprovados).
- **Remaining**: Nenhum.

### [BUG-12]
- **Status**: VERIFIED
- **Root cause**: As migrações em `crates/savevault-db/src/migrations.rs` executavam instruções DDL soltas fora de uma transação explícita. Se uma instrução falhasse no meio, o schema ficava em estado parcial corrompido sem registro na tabela `__migrations`, tornando impossível retomar de forma limpa.
- **Changed**:
  - `crates/savevault-db/src/migrations.rs`: Cada etapa de migração agora é executada dentro de uma transação exclusiva (`conn.unchecked_transaction()?`). A transação engloba a execução do script DDL e o registro da migração aplicada em `__migrations`. Em caso de erro, a transação sofre rollback total pelo SQLite, preservando o estado íntegro anterior.
- **Tests**: `test_atomic_migration_rollback_on_failure` em `crates/savevault-db/src/tests.rs`.
- **Validation**: Executado `cargo test -p savevault-db` (39 testes aprovados).
- **Remaining**: Nenhum.

### [BUG-15]
- **Status**: VERIFIED
- **Root cause**: `Config::save` gravava o arquivo `config.toml` diretamente com `std::fs::write`. Quedas de energia, panes do sistema ou finalização abrupta durante a escrita podiam deixar o arquivo de configuração zerado (0 bytes) ou truncado, impedindo a inicialização do aplicativo em execuções subsequentes.
- **Changed**:
  - `crates/savevault-core/src/config.rs`:
    - `Config::save`: agora grava primeiramente em arquivo temporário único no mesmo diretório (`.{filename}.tmp.{uuid}`), executa `file.sync_all()` para forçar a descarga dos buffers físicos do sistema operacional, e em seguida realiza a substituição atômica via `std::fs::rename`. Caso ocorra qualquer falha, o arquivo temporário residual é removido.
    - `Config::load`: detecta arquivos vazios ou contendo apenas espaços (resíduo de corrupções parciais prévias) e retorna graciosamente a configuração default com log apropriado, sem quebrar a inicialização.
- **Tests**:
  - `config_save_atomic_overwrite_leaves_no_tmp_files` em `crates/savevault-core/src/tests.rs` (valida atomicidade e ausência de resíduos temporários).
  - `config_load_empty_or_whitespace_file_returns_default` em `crates/savevault-core/src/tests.rs` (valida resiliência a arquivos vazios/zerados).
  - `config_load_corrupted_syntax_returns_error` em `crates/savevault-core/src/tests.rs`.
- **Validation**: Executado `cargo test -p savevault-core` (21 testes aprovados).
- **Remaining**: Nenhum.

### [BUG-13]
- **Status**: VERIFIED
- **Root cause**: O detector GOG (`crates/savevault-discovery/src/gog.rs`) era um stub que retornava incondicionalmente `Ok(vec![])`.
- **Changed**:
  - `crates/savevault-discovery/src/gog.rs`: Implementado detector completo que:
    1. Consulta chaves do Registro do Windows (`HKLM\SOFTWARE\WOW6432Node\GOG.com\Games` e `HKLM\SOFTWARE\GOG.com\Games`), extraindo ID do jogo, nome (`gameName`) e caminho (`path`);
    2. Varre pastas padrão de instalação de jogos GOG (`C:\GOG Games`, `D:\GOG Games`, `E:\GOG Games`, etc.);
    3. Analisa arquivos de manifesto nativos `goggame-*.info` (formato JSON oficial do instalador GOG) para recuperar nome do jogo e ID de forma confiável e desacoplada;
    4. Suporta `GogDetector::with_root` para fixtures e injeção de bibliotecas em testes.
- **Tests**: `gog_detect_with_custom_root_finds_all_games` em `crates/savevault-discovery/src/tests.rs`.
- **Validation**: Executado `cargo test -p savevault-discovery` (30 testes aprovados).
- **Remaining**: Nenhum.

### [BUG-14]
- **Status**: VERIFIED
- **Root cause**: `find_steam_root` em `crates/savevault-discovery/src/steam.rs` verificava apenas o caminho fixo `C:\Program Files (x86)\Steam` e uma variável de ambiente, falhando em instalações em outros drives ou pastas customizadas.
- **Changed**:
  - `crates/savevault-discovery/src/steam.rs`:
    1. Implementada consulta ativa ao Registro do Windows via `query_registry_value`:
       - `HKCU\Software\Valve\Steam\SteamPath`
       - `HKLM\SOFTWARE\WOW6432Node\Valve\Steam\InstallPath`
       - `HKLM\SOFTWARE\Valve\Steam\InstallPath`
    2. Verificação de caminhos secundários comuns em múltiplos drives (`D:\Steam`, `E:\Steam`, `C:\Program Files\Steam`);
    3. Adicionado `SteamDetector::with_root` para permitir testes unitários isolados com mocks/fixtures.
- **Tests**:
  - `steam_detect_with_custom_root_finds_all_games` em `crates/savevault-discovery/src/tests.rs` (valida descoberta de múltiplos jogos através de `libraryfolders.vdf` e arquivos `.acf`).
  - `detect_with_no_steam_returns_empty` em `crates/savevault-discovery/src/tests.rs`.
- **Validation**: Executado `cargo test -p savevault-discovery` (30 testes aprovados).
- **Remaining**: Nenhum.

### [BUG-09]
- **Status**: VERIFIED
- **Root cause**: Existiam duas implementações divergentes de restauração: uma rotina legada e crua em `SnapshotManager::restore_snapshot` (que operava apenas no filesystem sem SQLite nem validação BLAKE3) e o motor oficial transacional em `savevault-app::restore`.
- **Changed**:
  - Consolidada a arquitetura com fonte única de verdade no motor `savevault_app::restore` (`restore_snapshot_with_db` e `restore_by_id`), que provê validação de hash BLAKE3 pré e pós-escrita, verificação contra path traversal, rollback e registro de safety backups no banco de dados.
  - A função de baixo nível `SnapshotManager::restore_snapshot` foi marcada com `#[deprecated]` instruindo o uso do motor oficial.
- **Tests**:
  - `test_dev_ui_restore_success_and_safety_backup` e `test_dev_ui_restore_error_handling` em `crates/savevault-app/tests/dev_ui_integration.rs`.
  - Testes de baixo nível em `crates/savevault-backup/src/hasher.rs`.
- **Validation**: Executado `cargo test -p savevault-backup` e `cargo test -p savevault-app --test dev_ui_integration`.
- **Remaining**: Nenhum.

### [BUG-10]
- **Status**: VERIFIED
- **Root cause**: O SaveVault mantinha um servidor HTTP embutido (`server.rs` e `assets.rs`) com portas TCP locais (`localhost`), que havia sido criado na fase inicial antes da migração para Tauri 2 nativo. Isso violava a diretriz de arquitetura 100% nativa sem portas de rede abertas.
- **Changed**:
  - Removidos completamente os arquivos legados `crates/savevault-app/src/ui/server.rs` e `crates/savevault-app/src/ui/assets.rs` (>1.270 linhas de código morto e HTML hardcoded).
  - Removida a exportação `start_dev_ui_server` de `savevault_app::ui` e `savevault_app::lib`.
  - Atualizados os testes de integração para validar diretamente os estados e comandos do Tauri sem necessidade de binding de sockets TCP.
- **Tests**: `test_dev_ui_graceful_shutdown_and_ipc_state` em `crates/savevault-app/tests/dev_ui_integration.rs`.
- **Validation**: Executado `cargo test -p savevault-app --test dev_ui_integration` (7 testes aprovados).
- **Remaining**: Nenhum.

### [BUG-11]
- **Status**: VERIFIED
- **Root cause**: Ao clicar em "Sair do SaveVault" no System Tray, o handler invocava diretamente `app.exit(0)`, encerrando o processo do SO sem notificar as tasks assíncronas em execução (pipeline, reconciliador e monitor de arquivos), podendo corromper gravações ativas.
- **Changed**:
  - `crates/savevault-app/src/ui/state.rs`: Adicionado canal de controle de encerramento (`shutdown_tx`) e método `trigger_graceful_shutdown()`, que desativa atomicamente os sinalizadores de execução e envia sinal a todos os ouvintes.
  - `crates/savevault-app/src/ui/tauri_app.rs`: O evento `quit` da bandeja agora aciona `s.trigger_graceful_shutdown().await`, aguarda a drenagem e liberação de recursos pelos workers (watcher e reconciliador) e só então chama `app_handle.exit(0)`.
  - `crates/savevault-app/src/main.rs`: Conectado `shutdown_tx` ao `DevUiState`, além de drenar `run_pipeline`, `reconciler` e invocar `watcher.shutdown().await` tanto no modo UI quanto no modo residente CLI.
- **Tests**: `test_dev_ui_graceful_shutdown_and_ipc_state` em `crates/savevault-app/tests/dev_ui_integration.rs`.
- **Validation**: Executado `cargo test -p savevault-app --test dev_ui_integration` (todos aprovados).
- **Remaining**: Nenhum.

### [BUG-07]
- **Status**: VERIFIED
- **Root cause**: `upload` em `GoogleDriveProvider` lia o arquivo local integralmente para a memória (`Vec<u8>`) via `tokio::fs::read` e utilizava upload multipart mesmo para arquivos grandes (violando as diretrizes da Google Drive API v3 para arquivos >= 5MB). O `download` também lia toda a resposta em buffer de memória (`res.bytes().await`). Para saves grandes, isso causava picos severos de consumo de RAM.
- **Changed**:
  - `crates/savevault-cloud/src/google_drive.rs`:
    1. Introduzido o limiar padrão `RESUMABLE_UPLOAD_THRESHOLD_BYTES = 5 * 1024 * 1024` (5 MB, conforme documentação oficial da API do Google Drive) e método de customização `with_resumable_threshold`;
    2. Implementado `upload_resumable`: inicia a sessão de upload com metadados via `POST /files?uploadType=resumable`, obtém o cabeçalho `Location` e transmite os dados do arquivo via streaming direto do disco com `tokio_util::io::ReaderStream` e `reqwest::Body::wrap_stream`, sem jamais carregar o arquivo integralmente em memória RAM;
    3. Arquivos menores que o limiar continuam utilizando upload multipart rápido de requisição única com uso de RAM estritamente delimitado (<5MB);
    4. Implementado streaming contínuo no `download`: consome chunks assíncronos (`res.chunk().await?`) diretamente para arquivo temporário local (`.download_tmp.<uuid>`), garantindo flush em disco e substituição atômica via rename, com pegada de memória RAM irrisória (~64KB).
- **Tests**:
  - `test_resumable_streaming_upload_for_large_files` em `crates/savevault-cloud/tests/google_drive_mock.rs` (valida o fluxo completo de Resumable Upload em duas etapas para arquivos simulados grandes).
  - `test_streaming_download_bounded_ram` em `crates/savevault-cloud/tests/google_drive_mock.rs` (valida download de arquivos em chunks com validação de integridade em disco).
- **Validation**: Executado `cargo test -p savevault-cloud --test google_drive_mock` (20 testes aprovados).
---

## Auditoria Pós-Correção & Quality Gates

Realizada varredura completa conforme seções 22 a 25 e 38 do spec mestre:
- **Stub Scanner / TODO Scan**: Varredura por `TODO`, `FIXME`, `todo!`, `unimplemented!` retornou 0 ocorrências em código de produção. Ocorrências de `panic!` restritas exclusivamente a asserções de testes unitários/integração.
- **Async Concurrency & Lock Auditing**:
  - Validado que nenhum `MutexGuard<Connection>` é retido através de requisições de rede, sleeps ou hashing intensivo.
  - Escopos de lock em `savevault-sync`, `savevault-pipeline` e `savevault-app::restore` operam em transações curtas estritamente limitadas a I/O SQLite.
- **Clippy Static Analysis**: `cargo clippy --workspace --all-targets --all-features` executado com **0 erros e 0 avisos**.
- **Formatting Compliance**: `cargo fmt --check` executado com **código 0**.
- **Suíte de Testes Automatizados**: `cargo test --workspace` executado com **166+ testes aprovados (0 falhas)**:
  - `savevault-core`: 21 passed; 0 failed
  - `savevault-db`: 39 passed; 0 failed
  - `savevault-backup`: 11 passed; 0 failed
  - `savevault-discovery`: 32 passed; 0 failed
  - `savevault-monitor`: 30 passed; 0 failed
  - `savevault-sync`: 13 passed; 0 failed
  - `savevault-cloud`: 20 passed; 0 failed
  - `savevault-app`: 11 passed (todos os testes de integração de dev-ui, pipeline, reconciliação e restauração aprovados)
- **Compilação de Produção**: `cargo build --workspace --release` concluído com **código 0**.






