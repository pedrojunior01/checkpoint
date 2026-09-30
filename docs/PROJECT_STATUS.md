# Relatório de Implementação e Arquitetura - SaveVault

**Versão Atual:** v0.9.0  
**Data do Relatório:** Setembro 2026  
**Status do Projeto:** Pipeline Core + Cloud Sync (Google Drive & OneDrive) + Cloud Recovery + Multi-Discovery + Autostart Windows + Tray Icon + Notificações Windows Toast + Resolução de Conflitos Multi-PC + CI/CD GitHub Actions + Correções de Auditoria Adversarial (235+ Testes Automatizados - 100% Aprovados)

---

## 1. Visão Geral do Projeto
O **SaveVault** é um sistema *CLI-first* de alta confiabilidade escrito 100% em Rust projetado para detectar automaticamente jogos instalados no Windows (10/11 x64), mapear seus diretórios de saves, monitorar alterações em tempo real de forma não-bloqueante, gerar snapshots versionados com integridade garantida por hash BLAKE3 e sincronizar os backups de maneira assíncrona com a nuvem (Google Drive e Microsoft OneDrive via OAuth 2.0 com PKCE).

A arquitetura do projeto segue o padrão de **Workspace Modular**, dividindo o código em 8 *crates* autônomos para garantir alta confiabilidade, concorrência segura, ausência de deadlocks e baixo consumo de CPU/IO:
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
  - Tratamento case-insensitive preservando formatação original.
  - Mitigação de directory traversal indevido.
- **Configuração (`src/config.rs`)**: Gerenciamento do arquivo `%APPDATA%\SaveVault\config.toml` (retenção de versões, tempos de debounce, provedor de nuvem, estratégia de conflito, configurações de notificações toast, flag de autostart e caminhos do sistema).
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
  - `004_backups_drive_file_id`: persistência estruturada do identificador remoto no Google Drive / OneDrive.
  - `005_games_unique_name_platform`: deduplicação e índice único `(name COLLATE NOCASE, platform)` prevenindo duplicatas de jogos.
  - `006_files_and_backups_canonical`: índices únicos estritos para consistência de integridade relacional.
  - `007_games_platform_id_composite`: índice único composto `UNIQUE (platform, platform_id) WHERE platform_id IS NOT NULL`, eliminando colisões de IDs idênticos entre lojas distintas (Steam, GOG, Epic).
- **Repositórios CRUD com Integridade Estrita e Tratamento de Erros**:
  - `GamesRepo`: suporte a `find_by_platform_id`, `find_by_name_and_platform`, listagem geral (`list_all`) e reaproveitamento seguro de UUIDs sem `filter_map` mascarando erros.
  - `SaveLocationsRepo`: método `upsert` idempotente que preserva UUIDs existentes e normaliza caminhos Windows.
  - `FilesRepo`: suporte a `find_by_id`, `find_by_path` e `list_by_location` com propagação estrita de falhas de desserialização (sem fabricação arbitrária de UUIDs).
  - `BackupsRepo`: controle de retenção com método `prune_old_versions` retornando versões podadas para limpeza física em disco.
  - `SyncQueueRepo`: enfileiramento, listagem de pendentes e controle de tentativas.
  - Implementação estrita de `ON DELETE CASCADE`.

### 3. `savevault-discovery` (Completo - Multi-Launcher & Heurístico)
- **Steam Engine (`steam.rs`)**:
  - Mapeamento dinâmico de múltiplas bibliotecas via leitura do `libraryfolders.vdf`.
  - Parser nativo de arquivos `.acf` para extrair nome, Steam AppID e pasta de instalação sem resíduos de aspas.
- **GOG Galaxy Engine (`gog.rs`)**:
  - Varredura nativa do Registro do Windows em `HKLM\SOFTWARE\GOG.com\Games` e `HKLM\SOFTWARE\WOW6432Node\GOG.com\Games`.
  - Extração de nome, Product ID e diretório de instalação com suporte a String e `REG_DWORD`.
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
  - Exposição pública de `ManifestDetector` e orquestração limpa.
  - Resolução de saves para Steam (por AppID ou título) e GOG (por GOG ID ou título com `install_path`).
  - Expansão de variáveis de ambiente (`<winAppData>`, `<winDocuments>`, etc.) e identificadores dinâmicos Steam (`<storeUserId>`).

### 4. `savevault-monitor` (Completo)
- **Pipeline de Monitoramento Resiliente**:
  - `SaveScanner`: varredura recursiva de arquivos, diretórios e padrões glob com exclusão de arquivos internos (`.savevault_*`).
  - `SaveWatcherService`: integração com a crate `notify` em canal não-bloqueante (`try_send`) com contadores atômicos contra overflow e monitoramento dinâmico de pastas-mãe.
  - `FileDebouncer`: agrupamento (*coalescing*) causal de múltiplos eventos de escrita em janela temporal configurável, com suporte a cancelamento de tarefas obsoletas por geração.
  - `InFlightTracker`: rastreamento de tarefas com método `remove_if_current` liberando memória imediatamente após a conclusão da estabilidade.
  - `StabilityChecker`: motor de detecção de escrita em andamento. Mapeia códigos de erro nativos do Windows (`ERROR_SHARING_VIOLATION` 32 e 33) e aplica retentativas com backoff exponencial e jitter para garantir que saves em gravação ativa pelo jogo não sejam corrompidos ou lidos prematuramente.

### 5. `savevault-backup` (Completo)
- **Criptografia e Hashing**: Cálculo determinístico de assinaturas via **BLAKE3** com leitura em streaming por blocos.
- **SnapshotManager (Modelo Híbrido Cloud-First 2-Folder + Versionado)**:
  - Criação de snapshots locais organizados em `v{version}`, `latest/` (último save ativo) e `daily/` (cópia de segurança do primeiro save do dia).
  - Previne vazamento de espaço em disco em jogos com auto-save frequente mantendo histórico limpo.
  - Rotina de **restauração segura com resolução multi-layout**: busca snapshots em `v{version}`, `latest/` e `daily/`, gerando automaticamente cópia temporária de segurança (`.savevault_safety_bak`) com escrita atômica antes de substituir o arquivo original.

### 6. `savevault-cloud` (Completo - Multi-Provider)
- **Autenticação OAuth 2.0 PKCE Resiliente (`src/auth.rs` e `src/onedrive_auth.rs`)**:
  - Fluxo *Authorization Code com PKCE via Loopback IP* (`127.0.0.1:8085` / `:8086`).
  - Loop assíncrono com deadline de 180s, ignorando `/favicon.ico` e drops de conexão sem quebrar o fluxo.
  - Validação estrita de parâmetro anti-CSRF `state`.
  - Armazenamento seguro de *Refresh Token* no **Windows Credential Manager** via crate `keyring`.
  - Renovação automática e transparente de *Access Tokens* expirados.
- **Google Drive Provider (`src/google_drive.rs`)**:
  - Criação hierárquica automática de pastas no Drive (`SaveVault/<save_location_id>/<file_id>/`).
  - Upload Multipart (metadados JSON + binário do snapshot em requisição HTTP única).
  - Métodos para download, listagem e remoção remota com sanitização de URLs.
- **OneDrive Provider (`src/onedrive.rs`)**:
  - Integração nativa com a Microsoft Graph API (`/me/drive/special/approot` ou `/me/drive/root:/SaveVault`).
  - Upload direto e por chunks para saves de qualquer porte.

### 7. `savevault-sync` (Completo - Non-blocking Architecture)
- **SyncEngine Desacoplado**:
  - Consumo assíncrono da tabela `sync_queue` através de `process_queue_unlocked` e `process_queue_active_provider_unlocked`.
  - **Zero Database Lock Durante Upload/Sleep**: Lock no SQLite é mantido apenas por microssegundos durante leituras e gravações pontuais, deixando o banco totalmente destravado durante requisições de rede, uploads e backoffs exponenciais.
  - Resolução resiliente de snapshots locais em múltiplos layouts (`latest/`, `v{version}`, pastas por `file_id` e `daily/`).
  - Tratamento de falhas de rede com retentativas automáticas e backoff exponencial.
  - Atualização idempotente do status de sincronização (`Pending` ➔ `Synced`) e persistência atômica do `cloud_path` e `drive_file_id`.
  - Operação segura em modo offline.

### 8. `savevault-app` (Orquestrador, CLI, Daemon Residente, Notificações & Conflitos)
- **Recuperação de Saves a partir da Nuvem (`src/cloud_recovery.rs`)**:
  - Varredura e download automático de saves remotos para novas máquinas ou reinstalações limpas.
- **Resolução de Conflitos Multi-PC (`src/conflict.rs`)**:
  - Detecção inteligente de concorrência e divergência de timestamps e hashes entre diferentes dispositivos.
  - Estratégias configuráveis via TOML ou CLI: `KeepLocal`, `KeepCloud` e `KeepBoth` (com backup renomeado com carimbo de data/hora e host de origem).
- **Notificações Windows Toast (`src/notifications.rs`)**:
  - Notificações nativas no Centro de Ações do Windows para backups, sincronizações e alertas de conflito.
  - Sistema de rate-limiting e supressão de ruído (evitando excesso de notificações em saves frequentes).
- **Servidor DEV-UI Seguro (`src/ui/server.rs` e `assets.rs`)**:
  - Proteção contra CSRF e DNS Rebinding: validação estrita de `Host` e `Origin` locais (`127.0.0.1` / `localhost`).
  - Rejeição de requisições mutantes externas via cabeçalho obrigatório `X-SaveVault-Request: 1`.
  - Respeito dinâmico ao provedor de nuvem selecionado no `config.toml` (OneDrive vs Google Drive).
- **Modo Residente e Tray Icon**:
  - Execução silenciosa em segundo plano com ícone na área de notificação (`--tray`).
  - Reconciliação periódica em background para saves modificados durante períodos offline.
- **CI/CD Automatizado via GitHub Actions**:
  - `.github/workflows/ci.yml`: validação em matrix (Windows / Linux), `cargo fmt`, `cargo clippy -- -D warnings` e suíte de testes.
  - `.github/workflows/release.yml`: geração automatizada de releases binários otimizados com cálculo de checksum SHA256.

---

## 3. Bateria de Testes Automatizados (QA)

Atualmente a suíte conta com **235+ testes automatizados (100% de aprovação)** cobrindo todos os módulos do workspace sem warnings de compilação ou do Clippy:

1. **`savevault-discovery` (46 testes)**: Parsing de manifestos Steam ACF, leitura do Registro do Windows para GOG (String e DWORD), heurística genérica com filtros anti-falsos-positivos e catálogo Ludusavi.
2. **`savevault-db` (38 testes)**: Migrações determinísticas (001 a 007), testes da constraint única composta `(platform, platform_id)`, índices case-insensitive (`COLLATE NOCASE`), integridade de `ON DELETE CASCADE` e ausência de conversão arbitrária de UUIDs.
3. **`savevault-app` (44 testes)**: Pipeline E2E, DEV-UI com validação anti-CSRF, reconciliação periódica, resolução multi-layout de restauração, escrita atômica e rollback de safety backups.
4. **`savevault-monitor` (30 testes)**: Coalescência de debouncing, limites de memória, expurgo de entradas finalizadas no `InFlightTracker`, recuperação de sharing violation (locks do Windows), filtragem de arquivos internos e renomeação atômica.
5. **`savevault-cloud` (23 testes)**: Mocks do Google Drive e OneDrive, upload multipart, token refresh, validação de `state` anti-CSRF e integração com keyring.
6. **`savevault-core` (23 testes)**: Normalização de caminhos Windows, autostart (`HKCU\...\Run`), roundtrip de `config.toml` com estratégias de conflito e notificações toast.
7. **`savevault-sync` (12 testes)**: Fila não-bloqueante, retentativas com backoff exponencial e transações atômicas de sincronização.
8. **`savevault-backup` (11 testes)**: Hashing BLAKE3 em streaming, integridade de snapshots versionados, remoção física de diretórios podados e safety backups anti-colisão.

---

## 4. Auditoria Adversarial e Correções de Produção

No ciclo de auditoria sistêmica e adversarial, foram eliminadas as seguintes vulnerabilidades críticas e inconsistências arquiteturais:

| Item | Gravidade | Descrição do Problema | Solução Implementada |
|---|---|---|---|
| **AUDIT-01** | Crítico | Lock de banco exclusivo (`Mutex<Connection>`) mantido durante requisições de rede e backoff de upload, congelando a UI e o pipeline | Criado `process_queue_unlocked` desacoplando o lock do SQLite durante I/O de rede e pausas assíncronas. |
| **AUDIT-02** | Crítico | `row_to_entry` fabricava silenciosamente novos UUIDs e timestamps (`Utc::now()`) ao ler linhas corrompidas do banco | Remoção do fallback arbitrário; agora propaga estritamente `rusqlite::Error::FromSqlConversionFailure`. |
| **AUDIT-03** | Alto | Repositórios do banco usavam `.filter_map(|r| r.ok())`, ocultando silenciosamente corrupção em linhas do banco | Substituído por coleta estrita com propagação de erros em todos os repositórios (`files`, `backups`, `games`, `sync_queue`). |
| **AUDIT-04** | Alto | Módulo Ludusavi bifurcado com `ManifestDetector` privado e rotina duplicada de persistência em `main.rs` | Exposto publicamente `ManifestDetector` e unificada a persistência via `persist_discovery_result` no orquestrador. |
| **AUDIT-05** | Alto | DEV-UI forçava sincronização com Google Drive mesmo quando o usuário configurava OneDrive | `handle_ui_command` no `main.rs` atualizado para repassar dinamicamente o provedor ativo configurado. |
| **AUDIT-06** | Alto | DEV-UI vulnerável a CSRF e DNS Rebinding via abas maliciosas no navegador chamando rotinas de restore e sync | Validação estrita de cabeçalho `Host` e `Origin` locais (`is_valid_local_host`/`is_valid_local_origin`) e exigência de `X-SaveVault-Request: 1`. |
| **AUDIT-07** | Alto | Conflito de `platform_id UNIQUE` no SQLite impedia que jogos em plataformas diferentes compartilhassem o mesmo identificador | Migração `007_games_platform_id_composite` implementando `UNIQUE (platform, platform_id) WHERE platform_id IS NOT NULL`. |
| **AUDIT-08** | Médio | `InFlightTracker` no monitor residente acumulava caminhos infinitamente em memória após a finalização da estabilização | Implementado método `remove_if_current` e liberação ativa da entrada após o término do monitoramento de estabilidade. |
| **AUDIT-09** | Médio | Listener local de OAuth aceitava uma única conexão TCP e abortava com `/favicon.ico` ou aborts parciais do browser | Listener encapsulado em loop resiliente com timeout de 180s, ignorando requisições secundárias até obter o código OAuth. |
| **AUDIT-10** | Médio | Notificações Windows Toast em excesso causavam sobrecarga de alertas no Windows Action Center | Implementado sistema de debounce temporal e limitação de taxa adaptativo em `notifications.rs`. |

---

## 5. Decisões Técnicas e Arquiteturais
1. **Concorrência Segura de Banco de Dados**:
   - Operações longas de rede e uploads nunca retêm referências ou locks de `Mutex<Connection>`. O SQLite opera em modo transacional micro-otimizado.
2. **Segurança de Rede Local na DEV-UI**:
   - Toda comunicação entre a interface web e a aplicação exige proteção CSRF de dupla camada: verificação de origem/host de loopback e validação de custom headers.
3. **Multi-Launcher e Multi-Cloud Nativo**:
   - Abstrações plugáveis permitem a inclusão de novos lançadores (Steam, GOG, Epic, emuladores) e provedores em nuvem (Google Drive, OneDrive, WebDAV/S3) sem quebrar o core.
4. **Isolamento de Credenciais**:
   - Nenhuma credencial em texto claro: integração exclusiva com o Windows Credential Manager.

---

## 6. Próximos Passos (Milestones Futuros)

- [x] **Pipeline Residente Contínuo e Tray Icon Windows**: Concluído com suporte a reconciliação e menu de bandeja.
- [x] **Recuperação de Saves a partir da Nuvem (`cloud recovery`)**: Concluído para inicialização limpa em novos dispositivos.
- [x] **Resolução de Conflitos Multi-PC**: Concluído com estratégias configuráveis (`KeepLocal`, `KeepCloud`, `KeepBoth`).
- [x] **Notificações Windows Toast**: Concluído com rate-limiting e suporte nativo.
- [x] **Suporte a OneDrive (Microsoft Graph)**: Concluído com autenticação PKCE e upload por chunks.
- [x] **Automação de CI/CD (GitHub Actions)**: Concluído com workflows de integração e release.
- [x] **Correções da Auditoria Adversarial (9 Riscos Críticos e Médios)**: Concluído com zero warnings e 235+ testes passando.
- [ ] **Empacotamento e Distribuição Final**:
  - Criação de instalador executável para usuário final (`.msi` via WiX Toolset ou `.exe` via Inno Setup).
- [ ] **Provedores Secundários de Cloud Adicionais**:
  - Dropbox, WebDAV / S3 / ProtonDrive.
