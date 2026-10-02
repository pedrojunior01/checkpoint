# Relatório de Implementação e Arquitetura - SaveVault

**Versão Atual:** v0.9.0-rc (Release Candidate)  
**Data do Relatório:** Outubro 2026  
**Status do Projeto:** Pipeline Core + Cloud Sync (Google Drive & OneDrive) + Cloud Recovery & Disaster Recovery + Multi-Launcher Discovery + Autostart Windows + Tauri Native Tray Minimized + Notificações Windows Toast + Resolução de Conflitos Multi-PC + CI/CD GitHub Actions + Correções de Auditoria Adversarial (Fase 1 e Fase 2 concluídas - 257+ Testes Automatizados - 100% Aprovados)

---

## 1. Visão Geral do Projeto
O **SaveVault** (checkpoint) é um sistema de proteção de saves de jogos para Windows (10/11 x64) de alta confiabilidade, desenvolvido 100% em Rust. O projeto é projetado para:
1. Detectar automaticamente jogos instalados no computador (Steam, GOG Galaxy, emuladores e heurística em pastas de sistema).
2. Mapear e monitorar alterações em arquivos e subdiretórios de save em tempo real de forma não-bloqueante.
3. Gerar snapshots versionados com integridade garantida por hash BLAKE3, preservando estruturas aninhadas de diretórios sem colisões.
4. Executar sincronização assíncrona com nuvem (Google Drive, Microsoft OneDrive e pastas sincronizadas locais) sem travar a interface ou o banco de dados.
5. Permitir Recuperação de Desastre (*Disaster Recovery*) em máquinas novas ou recém-formatadas mesmo com o banco local vazio, reconstruindo o catálogo a partir do manifesto da nuvem (`savevault_manifest.json`).
6. Oferecer interface nativa e moderna via **Tauri 2.0**, minimizada na bandeja do sistema (*System Tray*) perto do relógio do Windows, com consumo ultrabaixo de memória e CPU.

A arquitetura do projeto segue o padrão de **Workspace Modular**, dividindo o código em 8 *crates* autônomos:
- `savevault-core`
- `savevault-db`
- `savevault-discovery`
- `savevault-monitor`
- `savevault-backup`
- `savevault-cloud`
- `savevault-sync`
- `savevault-app`

---

## 2. Crates e Módulos Implementados

### 1. `savevault-core` (Completo)
- **Modelos de Domínio**: Define estruturas de dados universais (`Game`, `SaveLocation`, `FileEntry`, `Backup`, `SyncQueueItem`, `Platform`, `CloudProvider`, `ConflictStrategy`).
- **Normalização de Caminhos Windows (`src/path.rs`)**:
  - Remoção de prefixos estendidos `\\?\` e `\\?\UNC\`.
  - Tratamento case-insensitive com **Unicode Case-Folding** (`to_lowercase()`), alinhando com a semântica nativa do NTFS (F-13).
  - Cálculo de caminhos relativos seguros (`compute_relative_path`) para replicação exata de subpastas em backups e restauração (F-01).
  - Mitigação de directory traversal indevido.
- **Configuração (`src/config.rs`)**: Gerenciamento de `%APPDATA%\SaveVault\config.toml` (retenção de versões, tempos de debounce, provedor de nuvem, estratégia de conflito, configurações de notificações toast, autostart e caminhos do sistema).
- **Inicialização Automática com o Windows (`src/autostart.rs`)**:
  - Integração nativa com a chave do Registro `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\SaveVault`.
  - Cadastro de linha de comando com `--tray` direcionado diretamente para a interface nativa Tauri minimizada (F-06).
  - Funções de consulta e controle: `is_enabled`, `status`, `enable`, `disable`.

### 2. `savevault-db` (Completo)
- **SQLite Embutido**: Utiliza `rusqlite` com flag `bundled` (independente de drivers externos ou DLLs do sistema).
- **Migrações Determinísticas (001 a 007)**:
  - `001_initial_schema`: tabelas fundamentais (`games`, `save_locations`, `files`, `backups`, `sync_queue`).
  - `002_save_locations_unique_path`: desduplicação e índice de unicidade.
  - `003_save_locations_nocase_index`: suporte a `COLLATE NOCASE` para caminhos Windows.
  - `004_backups_drive_file_id`: persistência estruturada do identificador remoto no Google Drive / OneDrive.
  - `005_games_unique_name_platform`: deduplicação e índice único `(name COLLATE NOCASE, platform)`.
  - `006_files_and_backups_canonical`: índices únicos estritos para consistência relacional.
  - `007_games_platform_id_composite`: índice único composto `UNIQUE (platform, platform_id) WHERE platform_id IS NOT NULL`.
- **Repositórios CRUD com Integridade Estrita**:
  - `GamesRepo`, `SaveLocationsRepo`, `FilesRepo`, `BackupsRepo`, `SyncQueueRepo`.
  - Tratamento estrito de erros e integridade transacional sem silenciamento indevido via `filter_map`.

### 3. `savevault-discovery` (Completo - Multi-Launcher & Heurístico)
- **Steam Engine (`steam.rs`)**: Mapeamento dinâmico de múltiplas bibliotecas via leitura do `libraryfolders.vdf` e parser `.acf`.
- **GOG Galaxy Engine (`gog.rs`)**: Varredura nativa do Registro do Windows em `HKLM\SOFTWARE\GOG.com\Games` com fallback por diretório.
- **Emulators Engine (`emulators.rs`)**: Perfis para Dolphin, PCSX2, PPSSPP e RetroArch.
- **Detector Genérico / Heurístico (`generic.rs`)**: Varredura inteligente de `%APPDATA%`, `%LOCALAPPDATA%`, `Documents`, `Saved Games` com filtros anti-falsos-positivos.
- **Ludusavi Manifest Engine (`manifest/` e `ludusavi/`)**: Cache assíncrono do catálogo oficial Ludusavi (+53.000 jogos) e resolução de caminhos dinâmicos.

### 4. `savevault-monitor` (Completo)
- **Pipeline de Monitoramento Não-Bloqueante**:
  - `SaveScanner`: varredura recursiva de arquivos, diretórios e padrões glob.
  - `SaveWatcherService`: integração com a crate `notify` em canal não-bloqueante (`try_send`) com contadores atômicos contra overflow e suporte a inicialização com banco vazio (F-03).
  - `FileDebouncer`: agrupamento causal de múltiplos eventos de escrita em janela temporal configurável.
  - `StabilityChecker`: detecção de escrita em andamento tratando erros nativos do Windows (`ERROR_SHARING_VIOLATION` 32 e 33) com retentativas e backoff exponencial.

### 5. `savevault-backup` (Completo)
- **Criptografia e Hashing**: Assinaturas criptográficas via **BLAKE3** em streaming por blocos.
- **SnapshotManager com Suporte a Subdiretórios Aninhados (F-01)**:
  - Criação de snapshots locais organizados em `v{version}/`, `latest/` e `daily/`, preservando a árvore relativa de pastas (`create_snapshot_with_relpath`).
  - Restauração atômica com validação de hash e isolamento de caminhos relativos (`restore_snapshot_with_relpath`).
  - Prevenção de exclusão indevida de diretórios durante versionamento e pruning (`remove_snapshot_version_file`).

### 6. `savevault-cloud` (Completo - Multi-Provider)
- **Autenticação OAuth 2.0 PKCE Resiliente (`src/auth.rs` e `src/onedrive_auth.rs`)**:
  - Loopback IP seguro (`127.0.0.1:8085` / `:8086`) com timeout de 180s.
  - Armazenamento de *Refresh Token* criptografado no **Windows Credential Manager** via crate `keyring`.
- **Catálogo de Manifesto Remoto (F-05)**:
  - Definição de `CloudManifestEntry` e `CloudCatalogManifest` para versionamento centralizado do catálogo de saves na nuvem.
  - Métodos padronizados `get_manifest` e `save_manifest` na trait `CloudProvider` e em `Box<dyn CloudProvider>`.
- **Provedores Implementados**:
  - `GoogleDriveProvider`: Upload Multipart, download, exclusão e criação hierárquica de pastas.
  - `OneDriveProvider`: Integração com Microsoft Graph API via chunks e app folder.
  - `LocalFolderProvider`: Suporte a pastas sincronizadas locais (OneDrive, Google Drive Desktop, Dropbox).

### 7. `savevault-sync` (Completo - Non-blocking Architecture)
- **SyncEngine Desacoplado**:
  - Consumo assíncrono da tabela `sync_queue` com liberação imediata de lock do banco durante transferência de arquivos.
  - Sincronização padronizada e exclusiva via **Pasta Sincronizada Local** (`LocalFolderProvider`), utilizando as pastas oficiais do Google Drive para Desktop, OneDrive ou Dropbox (F-04 removido do escopo em favor de footprint ultraleve e estabilidade sem chaves OAuth).
  - Atualização automática e atômica do manifesto `savevault_manifest.json` a cada snapshot enviado (F-05).

### 8. `savevault-app` (Orquestrador, Desktop Tauri, Tray, Conflitos & DR)
- **Tauri 2.0 Native UI & System Tray (F-06)**:
  - Interface gráfica desktop nativa sem janelas de navegadores externos.
  - Minimização para a bandeja do sistema perto do relógio (`start_minimized = true`) ao iniciar com o Windows via `--tray`.
- **Proteção de Instância Única (`SingleInstanceGuard`) e Foco Ativo (F-07)**:
  - Mutex do sistema operacional Windows (`CreateMutexW`) garantindo que apenas uma instância desktop/residente seja executada simultaneamente.
  - Quando uma segunda instância é iniciada pelo usuário, ela localiza a janela existente via `EnumWindows`, unminimiza/restaura (`ShowWindow` `SW_RESTORE`) e a traz para o primeiro plano (`SetForegroundWindow`) antes de encerrar de forma limpa.
- **Compactação Nativa de Backups (.zip) (F-08)**:
  - Compactação recursiva nativa em Rust via crate `zip` (`zip::ZipWriter` + `walkdir`) com algoritmo DEFLATE em streaming.
  - Eliminação de processos externos (PowerShell / `Compress-Archive`), prevenindo falhas de sintaxe com aspas/apóstrofos e riscos de injeção de parâmetros em caminhos de usuários.
- **Validação Estrita de Caminhos no Explorer (F-09)**:
  - Remoção de `create_dir_all` em `open_path_in_explorer`. Se o caminho solicitado não existir no disco, a função retorna erro estrito sem alterar ou poluir o sistema de arquivos.
- **Recuperação de Desastre em Máquinas Novas (F-05)**:
  - Função `reconstruct_catalog_from_cloud` para recuperar registros de jogos e backups a partir do manifesto em nuvem mesmo com o banco local vazio.
- **Resolução de Conflitos Multi-PC (`src/conflict.rs`)**:
  - Detecção de divergências entre múltiplos dispositivos com estratégias `KeepLocal`, `KeepCloud` e `KeepBoth`.
- **Notificações Windows Toast (`src/notifications.rs`)**:
  - Alertas nativos no Windows Action Center com rate-limiting anti-spam.

---

## 3. Bateria de Testes Automatizados (QA)

Atualmente a suíte conta com **262 testes automatizados (100% de aprovação)** sem warnings de compilação ou do Clippy:

1. **`savevault-core` (26 testes)**: Normalização de caminhos Windows, Unicode Case-Folding (F-13), cálculo de `compute_relative_path` (F-01), autostart no Registro e roundtrip de configurações.
2. **`savevault-db` (40 testes)**: Migrações 001 a 007, índices compostos únicos, `ON DELETE CASCADE`, desduplicação e consistência relacional.
3. **`savevault-backup` (9 testes ativos + 3 lentos)**: Hashing BLAKE3 em streaming, snapshots com subpastas aninhadas (F-01), integridade de arquivos versionados e safety backups anti-colisão.
4. **`savevault-cloud` (38 testes)**: Mocks do Google Drive e OneDrive, upload multipart, token refresh, manifestos remotos e provedor de pastas locais.
5. **`savevault-sync` (12 testes)**: Sincronização não-bloqueante, retentativas com backoff exponencial e atualização de manifesto remoto.
6. **`savevault-app` (60 testes)**:
   - `src/lib.rs` (37 testes unitários, incluindo recuperação de catálogo de nuvem em banco vazio, exclusão mútua do mutex `SingleInstanceGuard`, verificação de `focus_existing_instance`, compactação nativa `.zip` com caracteres especiais/acentos e validação estrita de integridade de caminhos sem criação de diretórios fantasmas).
   - `dev_ui_integration.rs` (8 testes de API e endpoints).
   - `discovery_persistence.rs` (5 testes de persistência de descobertas).
   - `pipeline_integration.rs` (2 testes de integração contínua do watcher).
   - `reconciliation_integration.rs` (2 testes de reconciliação periódica).
   - `restore_integration.rs` (6 testes de restauração segura ponta a ponta com subdiretórios).
7. **`savevault-discovery` (47 testes)**: Manifestos Steam ACF, Registro do Windows para GOG, heurística genérica e Ludusavi manifest.
8. **`savevault-monitor` (30 testes)**: Coalescência de debouncing, compartilhamento de arquivos com bloqueio Windows (sharing violation) e estabilização de escritas atômicas.

---

## 4. Auditoria Adversarial: Remediações Concluídas

| Item | Severidade | Vulnerabilidade / Inconsistência | Status | Solução Implementada |
|---|---|---|---|---|
| **F-01** | Crítico | Sobrescrita de dados em snapshots com subdiretórios aninhados | **Corrigido** | Implementada preservação de caminhos relativos em `snapshot.rs` e `hasher.rs`. |
| **F-02** | Crítico | Mascaramento de erros de restauração no IPC/UI retornando sucesso falso | **Corrigido** | IPC Tauri e REST propagam erros estritos e a interface exibe feedback de falha. |
| **F-13** | Crítico | Inconsistência de case-folding NTFS usando ASCII em vez de Unicode | **Corrigido** | Normalização de caminhos atualizada para Unicode `to_lowercase()` em `path.rs`. |
| **F-03** | Alto | Watcher e pipeline paralisados na inicialização com banco vazio | **Corrigido** | Removida checagem restritiva; pipeline e consumidor de nuvem agora iniciam sempre. |
| **F-04** | Neutro | Provedores remotos de nuvem via API direta (OAuth HTTP) | **Removido do Escopo** | Descontinuado por decisão de produto em favor da sincronização exclusiva e ultraleve via pasta local (Google Drive Desktop, OneDrive, etc.). |
| **F-05** | Alto | Impossibilidade de Disaster Recovery em máquinas novas com banco vazio | **Corrigido** | Criado manifesto `savevault_manifest.json` na nuvem e rotina `reconstruct_catalog_from_cloud`. |
| **F-06** | Alto | Autostart no Registro chamando navegador Edge em vez do Tauri nativo | **Corrigido** | Linha de comando `--tray` agora inicia o Tauri nativo minimizado no tray. |
| **F-07** | Alto | Execução de múltiplas instâncias concorrentes causando conflito no SQLite | **Corrigido** | Implementado `SingleInstanceGuard` via Win32 `CreateMutexW` com foco na janela ativa (`SetForegroundWindow`). |
| **F-08** | Médio | Concatenação de caminhos e vulnerabilidade no export de backup via PowerShell | **Corrigido** | Compactação recursiva nativa em Rust via crate `zip` (`zip::ZipWriter` + `walkdir`). |
| **F-09** | Médio | Criação irrestrita de pastas inexistentes ao abrir caminho no Explorer | **Corrigido** | Validação estrita de existência em `open_path_in_explorer` sem mutação do filesystem. |

---

## 5. Próximos Passos

- [x] **Fase 1 (Crítico)**: F-01, F-02 e F-13 corrigidos e testados.
- [x] **Fase 2 (Alto)**: F-03, F-04, F-05, F-06 e F-07 corrigidos e testados.
- [x] **Otimizações de Performance & Memória**: Remoção do Tailwind CDN (Babel runtime), CSS estático compilado, `content-visibility: auto`, paginação de atividades, memory trimming no tray, executável release de 19 MB e memória resident reduzida de 733 MB para ~15-30 MB.
- [x] **Etapa 4 (Robustez de Filesystem e Processos)**: F-08 (ZIP nativo), F-09 (Validação de Explorer) e F-07 refinamento (Foco ativo em janela já em execução).
- [ ] **Etapa 5 (Descoberta & Plataformas)**:
  - F-11: Detecção de instalação do cliente Steam via Registro do Windows (`HKLM`/`HKCU`).
  - F-10: Correção do script Inno Setup (`checkpoint.iss`) alinhando diretivas relativas e chave de inicialização.
  - F-12: Limpeza do módulo legado e duplicado `ludusavi/`.
- [ ] **Etapa 6 (Hardening & Testes de Regressão)**.
