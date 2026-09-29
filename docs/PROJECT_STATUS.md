# Relatório de Implementação e Arquitetura - SaveVault

**Versão Atual:** v0.8.0  
**Data do Relatório:** Setembro 2026  
**Status do Projeto:** Pipeline Core + Cloud Sync (Google Drive) + Multi-Discovery + Autostart Windows + Tray Icon + Correções de Auditoria Fases 1 a 3 (221 Testes Automatizados)

---

## 1. Visão Geral do Projeto
O **SaveVault** é um sistema *CLI-first* escrito 100% em Rust projetado para detectar automaticamente jogos instalados no Windows (10/11 x64), mapear seus diretórios de saves, monitorar alterações em tempo real de forma não-bloqueante, gerar snapshots versionados com integridade garantida por hash BLAKE3 e sincronizar os backups de maneira assíncrona com a nuvem (Google Drive via OAuth 2.0).

A arquitetura do projeto segue o padrão de **Workspace Modular**, dividindo o código em 8 *crates* autônomos para garantir alta confiabilidade, concorrência segura, ausência de deadlocks e baixo consumo de CPU/IO.

---

## 2. Crates e Módulos Implementados

### 1. `savevault-core` (Completo)
- **Modelos de Domínio**: Define estruturas de dados universais (`Game`, `SaveLocation`, `FileEntry`, `Backup`, `SyncQueueItem`, `Platform`).
- **Normalização de Caminhos Windows (`src/path.rs`)**:
  - Remoção de prefixos estendidos `\\?\` e `\\?\UNC\`.
  - Tratamento case-insensitive preservando formatação original.
  - Mitigação de directory traversal indevido.
- **Configuração**: Gerenciamento do arquivo `%APPDATA%\SaveVault\config.toml` (retenção de versões, tempos de debounce, flag de autostart e caminhos do sistema).
- **Inicialização Automática com o Windows (`src/autostart.rs`)**:
  - Integração nativa com a chave do Registro `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\SaveVault`.
  - Detecção e canonicidade de caminhos de executáveis com tratamento seguro de aspas e espaços.
  - Funções de consulta e controle: `is_enabled`, `status`, `enable`, `disable`.

### 2. `savevault-db` (Completo)
- **SQLite Embutido**: Utiliza `rusqlite` com flag `bundled` (independente de drivers externos ou DLLs do sistema).
- **Migrações Determinísticas**:
  - `001_initial_schema`: criação das tabelas fundamentais (`games`, `save_locations`, `files`, `backups`, `sync_queue`).
  - `002_save_locations_unique_path`: desduplicação e índice de unicidade.
  - `003_save_locations_nocase_index`: suporte a `COLLATE NOCASE` para caminhos Windows.
  - `004_backups_drive_file_id`: persistência estruturada do identificador remoto no Google Drive.
  - `005_games_unique_name_platform`: deduplicação e índice único `(name COLLATE NOCASE, platform)` prevenindo duplicatas de jogos.
- **Repositórios CRUD com Integridade Referencial**:
  - `GamesRepo`: suporte a `find_by_platform_id`, `find_by_name_and_platform`, listagem geral (`list_all`) e reaproveitamento seguro de UUIDs.
  - `SaveLocationsRepo`: método `upsert` idempotente que preserva UUIDs existentes e normaliza caminhos Windows.
  - `FilesRepo`: suporte a `find_by_id` e `find_by_path`.
  - `BackupsRepo`: controle de retenção com método `prune_old_versions` retornando versões podadas para limpeza física em disco.
  - `SyncQueueRepo`: enfileiramento, listagem de pendentes e controle de tentativas.
  - Implementação estrita de `ON DELETE CASCADE`.

### 3. `savevault-discovery` (Completo - Multi-Launcher & Heurístico)
- **Steam Engine (`steam.rs`)**:
  - Mapeamento dinâmico de múltiplas bibliotecas via leitura do `libraryfolders.vdf`.
  - Parser nativo de arquivos `.acf` para extrair nome, Steam AppID e pasta de instalação sem resíduos de aspas.
- **GOG Galaxy Engine (`gog.rs`)**:
  - Varredura nativa do Registro do Windows em `HKLM\SOFTWARE\GOG.com\Games` e `HKLM\SOFTWARE\WOW6432Node\GOG.com\Games`.
  - Extração de nome, Product ID e diretório de instalação.
  - Detecção de saves locais via campos do registro (`WORKINGDIR`, `SAVEPATH`) e subpastas típicas (`saves/`, `savegames/`).
  - Fallback automático por varredura de pastas de instalação (`C:\Program Files (x86)\GOG Galaxy\Games`).
- **Emulators Engine (`emulators.rs`)**:
  - Perfis pré-configurados e detecção de dados para Dolphin, PCSX2, PPSSPP e RetroArch.
- **Detector Genérico / Heurístico (`generic.rs`)**:
  - Varredura inteligente de diretórios de alta probabilidade (`%APPDATA%`, `%LOCALAPPDATA%`, `Documents`, `Saved Games`, `My Games`).
  - Regras estritas anti-falsos-positivos: ignora diretórios de sistema/apps (`Microsoft`, `Windows`, `Discord`, `Steam`, `node_modules`, etc.).
  - Critério de pontuação: exige $\ge 2$ arquivos de save típicos (`.sav`, `.dat`, `.save`, `.sg*`, `.slot`) ou nomenclaturas de progresso (`save01`, `slot_1`, `quicksave`).
  - Deduplicação cruzada: ignora pastas já cobertas por instalações conhecidas da Steam e do GOG.
- **Ludusavi Manifest Engine (`manifest/` e `ludusavi/`)**:
  - Download e cache assíncrono do catálogo oficial do Ludusavi (~17MB, +53.000 jogos).
  - Resolução de saves para Steam (por AppID ou título) e GOG (por GOG ID ou título com `install_path`).
  - Expansão de variáveis de ambiente (`<winAppData>`, `<winDocuments>`, etc.) e identificadores dinâmicos Steam (`<storeUserId>`).

### 4. `savevault-monitor` (Completo)
- **Pipeline de Monitoramento Resiliente**:
  - `SaveScanner`: varredura recursiva de arquivos, diretórios e padrões glob.
  - `SaveWatcherService`: integração com a crate `notify` em canal não-bloqueante (`try_send`) com contadores atômicos contra overflow e monitoramento dinâmico de pastas-mãe.
  - `FileDebouncer`: agrupamento (*coalescing*) causal de múltiplos eventos de escrita em janela temporal configurável, evitando sobrecarga de IO.
  - `StabilityChecker`: motor de detecção de escrita em andamento. Mapeia códigos de erro nativos do Windows (`ERROR_SHARING_VIOLATION` 32 e 33) e aplica retentativas com backoff exponencial e jitter para garantir que saves em gravação ativa pelo jogo não sejam corrompidos ou lidos prematuramente.

### 5. `savevault-backup` (Completo)
- **Criptografia e Hashing**: Cálculo determinístico de assinaturas via **BLAKE3** com leitura em streaming por blocos.
- **SnapshotManager (Novo Modelo Cloud-First 2-Folder)**:
  - Criação de snapshots locais organizados em duas pastas por jogo: `latest/` (último save ativo) e `daily/` (cópia de segurança do primeiro save do dia).
  - Previne vazamento de espaço em disco em jogos com auto-save frequente.
  - Rotina de **restauração segura com resolução multi-layout**: busca snapshots em `v{version}`, `latest/` e `daily/`, gerando automaticamente cópia temporária (`.savevault_safety_bak`) antes de substituir o arquivo original.

### 6. `savevault-cloud` (Completo)
- **Autenticação OAuth 2.0 (`src/auth.rs`)**:
  - Fluxo *Authorization Code com PKCE via Loopback IP* (`127.0.0.1:8085`).
  - Abertura automática do navegador padrão do sistema para consentimento do usuário.
  - Escopo restrito de segurança: `https://www.googleapis.com/auth/drive.file` (acesso exclusivo aos arquivos criados pelo próprio SaveVault).
  - Armazenamento seguro de *Refresh Token* no **Windows Credential Manager** via crate `keyring`.
  - Renovação automática e transparente de *Access Tokens* expirados.
- **Google Drive Provider (`src/google_drive.rs`)**:
  - Criação hierárquica automática de pastas no Drive (`SaveVault/<save_location_id>/<file_id>/`).
  - Upload Multipart (metadados JSON + binário do snapshot em requisição HTTP única).
  - Métodos implementados para download e remoção remota de arquivos com timeouts e sanitização de URLs.

### 7. `savevault-sync` (Completo)
- **SyncEngine**:
  - Consumo assíncrono da tabela `sync_queue`.
  - Resolução resiliente de snapshots locais em múltiplos layouts (`latest/`, `v{version}`, pastas por `file_id` e `daily/`).
  - Tratamento de falhas de rede com backoff exponencial (até 5 tentativas antes de marcar como falha definitiva).
  - Atualização idempotente do status de sincronização (`Pending` ➔ `Synced`) e persistência atômica do `cloud_path` e `drive_file_id`.
  - Operação segura em modo offline (ignora silenciosamente uploads pendentes quando o usuário não estiver autenticado).

### 8. `savevault-app` (Orquestrador, CLI & Daemon Residente)
- **Modo Residente Contínuo**:
  - Execução contínua monitorando em tempo real todos os saves cadastrados, com reconciliação periódica de arquivos alterados offline.
- **CLI Completa**:
  - `savevault`: executa discovery, persistência e inicia o monitoramento residente.
  - `savevault --once`: executa discovery pontual sem manter o processo ativo.
  - `savevault add <nome> <caminho>`: cadastro manual imediato de jogos e locais de save.
  - `savevault list`: exibe todos os jogos e diretórios cadastrados no banco.
  - `savevault restore <id>`: restaura snapshot histórico com safety backup prévio.
  - `savevault restore list`: lista snapshots disponíveis para restauração.
  - `savevault autostart enable | disable | status`: gerencia a inicialização automática com o Windows no Registro (`HKCU\...\Run`).
  - `savevault auth login | status | logout`: gerencia o login OAuth 2.0 com o Google Drive.
  - `savevault sync`: força o processamento manual da fila de sincronização na nuvem.
  - `savevault --ui`: inicia o servidor local da DEV-UI para testes e inspeção no navegador.
  - `savevault --help` / `savevault --version`: visualização rápida de ajuda e versão.
- **DEV-UI REST Endpoints**:
  - `GET /api/autostart`: consulta o estado de inicialização automática.
  - `POST /api/autostart/toggle`: ativa/desativa a inicialização com o Windows via interface web.

---

## 3. Bateria de Testes Automatizados (QA)

Atualmente a suíte conta com **218 testes automatizados (100% de aprovação)** cobrindo todos os módulos do workspace:

1. **`savevault-discovery` (46 testes)**: Parsing de manifestos Steam ACF, leitura do Registro do Windows para GOG (com suporte a String e DWORD numérico), heurística genérica com filtros anti-falsos-positivos, isolamento do detector e testes de integração com o catálogo Ludusavi.
2. **`savevault-db` (37 testes)**: Idempotência de migrações (incluindo migração 005), índices case-insensitive (`COLLATE NOCASE`), `ON DELETE CASCADE`, consistência de UUIDs, deduplicação de jogos e operações CRUD.
3. **`savevault-app` (36 testes)**: Testes de integração E2E do pipeline de monitoramento residente, DEV-UI, reconciliação periódica e restauração segura com escrita atômica e rollback.
4. **`savevault-monitor` (30 testes)**: Coalescência de debouncing, limites de memória, recuperação de sharing violation (locks do Windows), filtragem de arquivos internos e renomeação atômica.
5. **`savevault-cloud` (23 testes)**: Testes com mock do Google Drive, upload multipart, token refresh, validação de `state` anti-CSRF e keyring.
6. **`savevault-core` (23 testes)**: Normalização de paths Win32, inicialização automática com o Windows (`HKCU\...\Run`), roundtrip do arquivo `config.toml`, detecção de arquivos internos e modelos.
7. **`savevault-sync` (12 testes)**: Consumo da fila, retentativas com backoff exponencial e transações atômicas de sync.
8. **`savevault-backup` (11 testes)**: Hashing BLAKE3 em streaming, integridade de snapshots no modelo 2-folder, remoção física de diretórios podados e safety backups anti-colisão.

---

## 4. Auditoria Adversarial e Correções de Produção

No ciclo de auditoria sistêmica e adversarial, foram eliminados os seguintes bugs e vulnerabilidades:

| Item | Gravidade | Descrição do Problema | Solução Implementada |
|---|---|---|---|
| **BUG-01** | Crítico | Jogos sem `platform_id` (emuladores/genéricos/manuais) duplicavam registros a cada scan | Criada Migração `005_games_unique_name_platform`, lookup por `(name COLLATE NOCASE, platform)` e `upsert` em `SaveLocationsRepo`. |
| **BUG-02** | Alto | `prune_old_versions` removia apenas linhas do SQLite, deixando arquivos órfãos ocupando gigabytes em disco | `prune_old_versions` retorna backups podados e `SnapshotManager::remove_snapshot_version` remove pastas `v{version}` físicas. |
| **BUG-03** | Alto | Restaurações criavam arquivos `.savevault_safety_` que eram re-capturados pelo watcher como saves novos | Adicionado filtro `is_internal_savevault_file` no `SaveScanner` e `match_path_to_locations` do watcher. |
| **BUG-04** | Alto | DEV-UI exposta com `Access-Control-Allow-Origin: *` sem validação de origem | Restrito CORS a `http://127.0.0.1:4040` e validação estrita de header `Origin` rejeitando origens externas com `403 Forbidden`. |
| **BUG-05** | Médio | OAuth loopback sem parâmetro `state` contra CSRF e sem alerta se `refresh_token` não fosse retornado | Adicionado UUID `state` validado no callback e verificação prévia no Windows Credential Manager. |
| **BUG-06** | Médio | Servidor HTTP lia requisições com buffer TCP único, quebrando payloads POST fragmentados | Implementada leitura de loop HTTP respeitando `Content-Length` com limite contra DoS (2MB). |
| **BUG-09** | Médio | `config.general.debounce_ms` era ignorado pelo watcher | Propagação de `debounce_duration` configurada para `SaveWatcherConfig`. |
| **BUG-10** | Médio | Paths Win32 estendidos (`\\?\` e `\\?\UNC\`) quebravam normalização | Tratamento e remoção segura de prefixos em `crates/savevault-core/src/path.rs`. |
| **BUG-11** | Baixo | Registro GOG salvando `gameID` como `REG_DWORD` falhava ao ler como String | Fallback com tentativa de leitura numérica `u32` e fallback por nome de subchave. |
| **BUG-12** | Alto | Arquitetura 2-folder (`latest`/`daily`) quebrava restaurações e uploads que buscavam estruturas de versão `v{version}` | Implementada resolução de snapshots em cascata (`v{version}` ➔ `latest` ➔ `daily` ➔ layouts legados) em `restore.rs` e `savevault-sync`. |
| **STACK** | Crítico | Frame assíncrono estourando stack padrão de 1MB da thread inicial do Windows | Despacho síncrono imediato para comandos de ajuda e runner Tokio em thread dedicada com stack de 16MB. |

---

## 5. Decisões Técnicas e Arquiteturais
1. **Configuração de Toolchain do Cargo (`.cargo/config.toml`)**:
   - Mapeamento explícito de `target-dir = "C:/svt"` para contornar limitações de linkers no Windows com caminhos que possuem acentuação ou espaços (ex: `Programação`).
   - Suporte transparente à toolchain GNU (`stable-x86_64-pc-windows-gnu` + MinGW64) e MSVC.
2. **Segurança de Credenciais**:
   - Nenhum segredo ou token em texto plano no disco.
   - Utilização do cofre de credenciais nativo do Windows (Windows Credential Manager via `keyring`).
   - Disponibilização de template seguro `.env.example` mantendo o `.env` protegido via `.gitignore`.
3. **Tolerância a Falhas**:
   - Falhas de conexão à nuvem não bloqueiam o monitoramento nem corrompem a gravação de snapshots locais.

---

## 6. Próximos Passos (Milestones Futuros)

- [x] **Pipeline Residente Contínuo no `savevault-app`**: Implementado com modo daemon contínuo e reconciliação periódica.
- [x] **CLI de Restauração (`savevault restore`)**: Implementado com safety backups e escrita atômica.
- [x] **Ampliadores de Descoberta (GOG & Genérico & Manual)**:
  - [x] Leitura de chaves do Registro do Windows (`HKLM\SOFTWARE\GOG.com\Games` e WoW6432Node).
  - [x] Heurística para busca de saves em jogos independentes fora do manifesto (`generic.rs`).
  - [x] Comandos CLI `savevault add` e `savevault list`.
- [x] **Inicialização Automática com o Windows**:
  - [x] Integração com `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\SaveVault`.
  - [x] Comandos CLI: `savevault autostart enable | disable | status`.
  - [x] Endpoints REST na DEV-UI: `GET /api/autostart` e `POST /api/autostart/toggle`.
- [x] **System Tray & Modo Silencioso Windows**:
  - [x] Ícone na bandeja do Windows (`tray-icon`) com menu de contexto e ações rápidas.
  - [x] Modo silencioso em segundo plano com flag `--tray`.
- [x] **Correções Críticas de Auditoria (Fases 1 a 3)**:
  - [x] Armazenamento físico versionado (`v{version}`) coexistindo com Cloud Sync e eliminação de fallback acidental a `latest`.
  - [x] Migração 006: índices únicos e deduplicação para `files` e `backups` com `upsert` canônico.
  - [x] Proteção CORS estrita contra prefix bypass na DEV-UI.
  - [x] Detecção de emuladores integrada gerando `SaveLocation` automaticamente.
  - [x] Suíte de 221 testes automatizados (0 falhas) cobrindo todo o workspace.
- [ ] **Empacotamento e Distribuição**:
  - Criação de instalador executável para usuário final (`.msi` ou `.exe`).
- [ ] **Provedores Secundários de Cloud (Pós-MVP)**:
  - OneDrive (Microsoft Graph API), Dropbox, WebDAV / S3.
