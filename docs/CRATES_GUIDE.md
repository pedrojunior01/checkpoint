# 📦 Guia Detalhado dos Crates — checkpoint

Este guia fornece um mergulho técnico em cada um dos **8 crates** do workspace Cargo do **checkpoint** (que utiliza a convenção de nomenclatura de pacotes `savevault-*`), detalhando suas responsabilidades, contratos públicos, tratamento de concorrência e particularidades do ambiente Windows.

---

## Índice dos Crates
1. [`savevault-core`](#1-savevault-core) — Modelos de Domínio, Caminhos Windows e Configuração
2. [`savevault-db`](#2-savevault-db) — Camada de Persistência SQLite Estática e Migrações
3. [`savevault-discovery`](#3-savevault-discovery) — Descoberta de Jogos (Steam, GOG, Emuladores, Heurística e Ludusavi)
4. [`savevault-monitor`](#4-savevault-monitor) — Monitoramento em Tempo Real, Debouncing e Estabilização
5. [`savevault-backup`](#5-savevault-backup) — Hashing BLAKE3 em Streaming e Gestão de Snapshots
6. [`savevault-cloud`](#6-savevault-cloud) — Abstração de Nuvem, Pastas Locais (OneDrive) e OAuth 2.0
7. [`savevault-sync`](#7-savevault-sync) — Motor de Sincronização Assíncrona e Fila Transacional
8. [`savevault-app`](#8-savevault-app) — Orquestrador, Interface Desktop Tauri v2 e CLI

---

## 1. `savevault-core`

* **Caminho:** `crates/savevault-core`
* **Dependências Externas Principais:** `serde`, `uuid`, `chrono`, `thiserror`, `toml`, `winreg`

### Responsabilidade
É o coração do sistema, definindo as estruturas de dados universais e contratos canônicos sem acoplamento com bancos de dados, rede ou interface gráfica. Centraliza regras de negócio de caminhos no Windows e integração com o Registro do Windows para inicialização automática com o sistema.

### Tipos e Estruturas Principais (`models.rs`)
* `Game`: Entidade representativa de um jogo (`id: Uuid`, `name: String`, `platform: Platform`, `platform_id: Option<String>`, `confidence: Confidence`, `created_at: DateTime<Utc>`).
* `SaveLocation`: Associação do jogo a um caminho no disco (`id: Uuid`, `game_id: Uuid`, `path: PathBuf`, `location_type: SaveLocationType`).
* `FileEntry`: Registro de arquivo monitorado (`id: Uuid`, `save_location_id: Uuid`, `path: PathBuf`, `hash: String`, `size_bytes: u64`, `modified_at: DateTime<Utc>`).
* `Backup`: Snapshot histórico versionado (`id: Uuid`, `file_id: Uuid`, `version: u32`, `cloud_path: Option<String>`, `drive_file_id: Option<String>`, `sync_status: SyncStatus`, `created_at: DateTime<Utc>`).
* `SyncQueueItem`: Registro na fila transacional de upload/deleção (`id: Uuid`, `backup_id: Uuid`, `operation: SyncOperation`, `status: SyncStatus`, `attempts: u32`, `created_at: DateTime<Utc>`).
* **Enums de Domínio:**
  * `Platform`: `Steam`, `Gog`, `Dolphin`, `Pcsx2`, `Ppsspp`, `RetroArch`, `Generic`.
  * `Confidence`: `High`, `Medium`, `Low`.
  * `SyncStatus`: `Pending`, `Synced`, `Failed`, `PendingOffline`.
  * `SyncOperation`: `Upload`, `Delete`.
  * `SaveLocationType`: `File`, `Directory`, `Registry`, `Glob`.

### Manipulação Lógica de Caminhos Windows (`path.rs`)
* `normalize_windows_path_str(raw: &str)`: Normaliza strings de caminhos para persistência no banco sem depender da existência física no disco. Remove prefixos `\\?\` e `\\?\UNC\`, colapsa separadores duplicados (`\\` → `\`), garante letras de drive em maiúsculo (`c:` → `C:`) e trata UNC (`\\servidor\pasta`).
* `canonicalize_windows_path_str(raw: &str)`: Cria representação única para chaves em tabelas hash (case-folded para minúsculas), respeitando a semântica case-insensitive do NTFS.
* `is_internal_savevault_file(path: &Path)`: Detecta arquivos de trabalho gerados pelo próprio checkpoint (`.savevault_safety_*`, `.restore_tmp_*`, `.tmp`, `.crswap`), evitando que o watcher entre em loop recursivo.

### Gerenciamento de Autostart (`autostart.rs`)
* Manipula a chave `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` através da crate `winreg`.
* Adiciona aspas ao caminho do executável para tolerar espaços em nomes de pastas e insere o parâmetro `--tray` para inicialização silenciosa na bandeja do sistema.

---

## 2. `savevault-db`

* **Caminho:** `crates/savevault-db`
* **Dependências Externas Principais:** `rusqlite` (com flag `bundled`), `uuid`, `chrono`, `tracing`, `savevault-core`

### Responsabilidade
Camada de persistência relacional do SaveVault. Utiliza SQLite estaticamente embutido, eliminando dependências de DLLs externas no Windows. Gerencia esquemas, migrações versionadas e repositórios tipados com integridade referencial `ON DELETE CASCADE`.

### Conexão e Pragma (`connection.rs`)
* `Database::open(path: &Path)`: Abre a conexão, aplica `PRAGMA foreign_keys = ON;`, ativa `PRAGMA journal_mode = WAL;` (Write-Ahead Logging para permitir leituras concorrentes simultâneas com escritores) e executa as migrações pendentes.
* `Database::open_in_memory()`: Instância volátil para testes unitários em memória.

### Migrações Determinísticas (`migrations.rs`)
A tabela `__migrations` armazena o histórico em ordem cronológica:
* `001_initial_schema`: Criação das tabelas fundamentais (`games`, `save_locations`, `files`, `backups`, `sync_queue`).
* `002_save_locations_unique_path`: Desduplicação de locais e índice de unicidade por jogo e caminho.
* `003_save_locations_nocase_index`: Adiciona índice `COLLATE NOCASE` para caminhos de save no Windows.
* `004_backups_drive_file_id`: Adiciona coluna `drive_file_id` em `backups` para identificadores remotos na nuvem.
* `005_games_unique_name_platform`: Cria índice único `(name COLLATE NOCASE, platform)` prevenindo duplicatas de jogos.

### Repositórios Tipados (`repositories/`)
* `GamesRepo`: Suporte a `find_by_platform_id`, `find_by_name_and_platform`, `list_all`, `update_name` e deleção em cascata.
* `SaveLocationsRepo`: Método `upsert` idempotente que utiliza `ON CONFLICT(game_id, path COLLATE NOCASE) DO UPDATE SET location_type = excluded.location_type RETURNING ...`, preservando os UUIDs primários existentes para não quebrar chaves estrangeiras.
* `FilesRepo`: Upsert e busca por caminho canônico.
* `BackupsRepo`: Gestão de histórico, controle de retenção com `prune_old_versions` transacional (se a versão possui vínculo na nuvem, enfileira deleção remota antes da remoção).
* `SyncQueueRepo`: Fila de tarefas para o worker assíncrono.

---

## 3. `savevault-discovery`

* **Caminho:** `crates/savevault-discovery`
* **Dependências Externas Principais:** `winreg`, `serde_yaml`, `reqwest`, `savevault-core`

### Responsabilidade
Motor de detecção de jogos e localização de saves. Identifica os títulos instalados no sistema operacional e localiza onde seus saves são gravados através de múltiplos detectores especializados e cruzamento com a base aberta do Ludusavi Manifest (+53.000 títulos).

### Detectores Especializados
* **Steam (`steam.rs`):**
  * Analisa o arquivo `libraryfolders.vdf` do Steam para descobrir todas as pastas de biblioteca espalhadas em diferentes discos (`C:\`, `D:\`, etc.).
  * Faz o parse dos arquivos `appmanifest_<appid>.acf` para extrair nome oficial, AppID e diretório de instalação real.
* **GOG Galaxy (`gog.rs`):**
  * Varre o Registro do Windows em `HKLM\SOFTWARE\GOG.com\Games` e a chave de compatibilidade 64-bit `HKLM\SOFTWARE\WOW6432Node\GOG.com\Games`.
  * Extrai Product ID, caminho de instalação e parâmetros `SAVEPATH` e `WORKINGDIR`. Possui fallback para varredura de diretórios padrões (`C:\GOG Games`).
* **Emuladores (`emulators.rs`):**
  * Perfis de mapeamento pré-configurados para Dolphin, PCSX2, PPSSPP e RetroArch.
* **Heurística Genérica (`generic.rs`):**
  * Varre pastas de alta densidade (`%APPDATA%`, `%LOCALAPPDATA%`, `Saved Games`, `Documents`) com algoritmo de pontuação estrito ("preferir falsos negativos a falsos positivos").
  * Exige $\ge 2$ arquivos de save típicos (`.sav`, `.dat`, `.save`, `.sg*`, `.slot`) ou nomes indicadores de progresso (`save01`, `quicksave`).
  * Blacklist extensiva contra diretórios de sistema, drivers, navegadores e ferramentas de desenvolvimento.
* **Ludusavi Manifest Integration (`ludusavi/` e `manifest/`):**
  * Faz download assíncrono do YAML oficial com cache local de 7 dias.
  * O `PathResolver` expande placeholders Win32 (`<winAppData>`, `<winDocuments>`, etc.) e IDs de usuário Steam (`<storeUserId>`), rejeitando placeholders de outros sistemas operacionais.
  * Proteção de identidade: se um jogo possui Store ID (Steam/GOG), recusa casamentos nominais caso o manifesto aponte para um jogo com Store ID divergente.

---

## 4. `savevault-monitor`

* **Caminho:** `crates/savevault-monitor`
* **Dependências Externas Principais:** `notify` (v6 com backend Windows), `tokio`, `savevault-core`

### Responsabilidade
Pipeline em tempo real de vigilância do sistema de arquivos e validação de estabilidade. Converte notificações brutas do Windows em eventos consolidados e estáveis (`StableFileEvent`).

### Componentes Internos
* **`SaveScanner` (`scanner.rs`):**
  * Realiza varreduras físicas de arquivos, diretórios e padrões glob.
  * Proteção contra traversal: impõe profundidade máxima (`max_depth = 10`), ignora links simbólicos e junction points e rejeita componentes `..` em padrões glob.
  * Suporta detecção de arquivos únicos (ex: `S0000.sl2` do Sekiro) e diretórios completos.
* **`FileDebouncer` (`debouncer.rs`):**
  * Loop assíncrono com `tokio::select!` e branch prioritário `biased;`.
  * Coalesce rajadas de eventos de escrita emitidos por jogos durante operações de autosave.
  * Janela máxima de espera (`max_wait_duration = 2000ms`) para prevenir starvation de arquivos que sofrem escrita contínua.
  * Limite de saturação de memória (`max_pending_paths = 5000`) com contador atômico de overflow.
  * `InFlightTracker`: Rastreia gerações (`generation: u64`) e aciona tokens de cancelamento atômicos (`cancel_token = true`) para anular verificações de estabilidade defasadas.
* **`StabilityChecker` (`stability.rs`):**
  * Intercepta erros Win32 de arquivos abertos com trava exclusiva (`ERROR_SHARING_VIOLATION` 32/33 e `ERROR_LOCK_VIOLATION`), aplicando retentativas com backoff exponencial sem falhar prematuramente.
  * Se o erro for de permissão permanente (`ERROR_ACCESS_DENIED` 5), aborta imediatamente.
  * Limita concorrência a no máximo 8 verificações simultâneas via `tokio::sync::Semaphore`.
* **`SaveWatcherService` (`watcher.rs`):**
  * Orquestra o watcher do `notify` em canal `mpsc` assíncrono.
  * Suporte a pastas ainda não criadas: se um diretório de save ainda não existe, registra um watch no ancestral existente mais próximo e promove o watch assim que o jogo cria a subpasta.

---

## 5. `savevault-backup`

* **Caminho:** `crates/savevault-backup`
* **Dependências Externas Principais:** `blake3`, `tokio`, `savevault-core`

### Responsabilidade
Motor criptográfico e gerenciador de snapshots locais. Realiza cálculo determinístico de hash com leitura em streaming e gerencia a criação e restauração atômica de versões de saves.

### Principais Funcionalidades
* **Hashing em Streaming (`hasher.rs`):**
  * `hash_file(path)`: Lê o arquivo em chunks de 64 KB (`[0u8; 65536]`) através de `tokio::io::AsyncReadExt::read`.
  * Garante uso constante de memória RAM ($O(1)$) independentemente do tamanho do arquivo.
* **Estrutura de Snapshots Particionados (`snapshot.rs`):**
  * Armazena versões sob `<backup_root>/<sanitized_game>/latest/<filename>` e `<daily>/<filename>` (atualizado no máximo uma vez por dia por data local).
  * Sanitiza nomes de pastas para caracteres permitidos no Windows (`\ / : * ? " < > |`).
* **Restauração em 4 Passos com Safety Backup:**
  1. **Validação Prévia:** Garante que o snapshot de origem existe e está acessível antes de qualquer alteração no destino.
  2. **Safety Backup Obrigatório:** Se o arquivo de save atual existe, gera cópia de segurança prévia com UUID e contador (`<filename>.savevault_safety_<uuid>_<n>`).
  3. **Staging Intermediário:** Copia o snapshot para arquivo temporário no mesmo volume (`<filename>.restore_tmp_<uuid>`).
  4. **Rename Atômico Win32:** Promove o temporário para o destino ativo via `tokio::fs::rename` (operação atômica `MoveFileExW` no NTFS).

---

## 6. `savevault-cloud`

* **Caminho:** `crates/savevault-cloud`
* **Dependências Externas Principais:** `reqwest`, `keyring`, `tokio`, `savevault-core`

### Responsabilidade
Camada de abstração e drivers para armazenamento e recuperação de snapshots na nuvem. Oferece suporte nativo a pastas locais sincronizadas pelo Windows (OneDrive, Google Drive Desktop, Dropbox) e integrações diretas via APIs HTTP REST (Google Drive e OneDrive via OAuth 2.0 PKCE).

### Trait `CloudProvider` (`lib.rs`)
Contrato assíncrono implementado por todos os provedores:
* `upload(&self, local_path, cloud_path) -> CloudResult<String>`
* `download(&self, cloud_file_id, dest_path) -> CloudResult<()>`
* `delete(&self, cloud_file_id) -> CloudResult<()>`
* `get_manifest(&self) -> CloudResult<Option<CloudCatalogManifest>>`
* `save_manifest(&self, manifest) -> CloudResult<()>`

### Provedores Implementados
* **`LocalFolderProvider` (`local_folder.rs`):**
  * Não requer contas ou credenciais; opera diretamente sobre a pasta local de sincronização da nuvem (ex: `C:\Users\Nome\OneDrive\SaveVault`).
  * Realiza cópias atômicas e sanitiza caminhos relativos contra path traversal (`..`).
* **`GoogleDriveProvider` (`google_drive.rs`):**
  * Implementação da API Google Drive v3 com uploads multipart em um único request HTTP.
  * Estrutura hierárquica automática de pastas no Drive (`SaveVault/<save_location_id>/<file_id>/`).
* **`OneDriveProvider` (`onedrive.rs`):**
  * Cliente da Microsoft Graph API com controle de fragmentos de upload.
* **Autenticação Segura no Windows (`auth.rs` e `onedrive_auth.rs`):**
  * Fluxo OAuth 2.0 Authorization Code com PKCE e loopback IP local (`127.0.0.1:8085`).
  * Tokens de atualização (*Refresh Tokens*) armazenados no **Windows Credential Manager** via crate `keyring`.

---

## 7. `savevault-sync`

* **Caminho:** `crates/savevault-sync`
* **Dependências Externas Principais:** `tokio`, `rusqlite`, `savevault-core`, `savevault-db`, `savevault-cloud`

### Responsabilidade
Motor transacional de sincronização em segundo plano. Consome a tabela `sync_queue` do SQLite e orquestra a comunicação com o provedor de nuvem configurado.

### `SyncEngine` e Decisões de Design
* **Desacoplamento de Locks (`process_queue_unlocked`):**
  * Uploads de arquivos grandes ou esperas de rede não podem reter a trava do banco de dados SQLite.
  * O motor adquire o lock do SQLite por frações de milissegundo para coletar o próximo item pendente e libera o lock do banco durante a chamada de upload na nuvem. Readquire o lock apenas ao final para atualizar o status e deletar o item da fila.
* **Resiliência a Falhas de Rede com Backoff Exponencial:**
  * Configuração padrão: `max_attempts = 5`, `base_backoff_ms = 2000`.
  * Pausa exponencial: $\Delta t = \text{base\_backoff\_ms} \times 2^{(\text{attempts} - 1)}$.
  * Ao atingir 5 tentativas sem sucesso, marca o item e o backup correspondente como `Failed` para não drenar banda ou bateria continuamente.
* **Atomicidade Transacional:** Ao concluir o upload, uma transação SQLite atômica atualiza `backups` com `cloud_path`, `drive_file_id` e status `synced`, e remove o registro da `sync_queue`.

---

## 8. `savevault-app`

* **Caminho:** `crates/savevault-app`
* **Dependências Externas Principais:** `tauri` (v2, opcional via feature `desktop`), `tokio`, `tracing`, todos os crates internos

### Responsabilidade
Orquestrador central do **checkpoint**. Une os componentes de persistência, monitoramento, reconciliação e sincronização em uma aplicação unificada, disponibilizando tanto interface desktop gráfica nativa (Tauri v2) quanto modo daemon/CLI.

### Modos de Operação
* **Desktop GUI (Tauri v2):** Executado por padrão no Windows quando compilado com `--features desktop`. Renderiza a WebView2 ultraleve sem decorações nativas, ícone persistente na bandeja do sistema (*System Tray*) e controle de ciclo de vida da janela.
* **Headless Daemon (CLI):** Executado em servidores ou terminais sem dependência do WebView2. Monitora o filesystem e executa a reconciliação em loop perpétuo.
* **Ferramentas Especializadas:**
  * `scan_report` (`src/bin/scan_report.rs`): Utilitário de diagnóstico que varre os jogos instalados e exibe um relatório formatado de saves no terminal.
  * Subcomandos CLI de restauração e listagem (`checkpoint restore <id>`, `checkpoint list`).

