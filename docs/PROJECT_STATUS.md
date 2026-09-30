# Relatório de Implementação e Arquitetura - SaveVault

**Versão Atual:** v0.5.0  
**Data do Relatório:** Setembro 2026  
**Status do Projeto:** Pipeline Core + Cloud Sync (Google Drive) Operacionais

---

## 1. Visão Geral do Projeto
O **SaveVault** é um sistema *CLI-first* escrito 100% em Rust projetado para detectar automaticamente jogos instalados no Windows (10/11 x64), mapear seus diretórios de saves, monitorar alterações em tempo real de forma não-bloqueante, gerar snapshots versionados com integridade garantida por hash BLAKE3 e sincronizar os backups de maneira assíncrona com a nuvem (Google Drive via OAuth 2.0).

A arquitetura do projeto segue o padrão de **Workspace Modular**, dividindo o código em 8 *crates* autônomos para garantir alta confiabilidade, concorrência segura, ausência de deadlocks e baixo consumo de CPU/IO.

---

## 2. Crates e Módulos Implementados

### 1. `savevault-core` (Completo)
- **Modelos de Domínio**: Define estruturas de dados universais (`Game`, `SaveLocation`, `FileEntry`, `Backup`, `SyncQueueItem`).
- **Normalização de Caminhos Windows (`src/path.rs`)**:
  - Remoção de prefixos estendidos `\\?\` e `\\?\UNC\`.
  - Tratamento case-insensitive preservando formatação original.
  - Mitigação de directory traversal indevido.
- **Configuração**: Gerenciamento do arquivo `%APPDATA%\SaveVault\config.toml` (retenção de versões, tempos de debounce e caminhos do sistema).

### 2. `savevault-db` (Completo)
- **SQLite Embutido**: Utiliza `rusqlite` com flag `bundled` (independente de drivers externos ou DLLs do sistema).
- **Migrações Determinísticas**:
  - `001_initial_schema`: criação das tabelas fundamentais (`games`, `save_locations`, `files`, `backups`, `sync_queue`).
  - `002_save_locations_unique_path`: desduplicação e índice de unicidade.
  - `003_save_locations_nocase_index`: suporte a `COLLATE NOCASE` para caminhos Windows.
- **Repositórios CRUD com Integridade Referencial**:
  - `GamesRepo`: suporte a `find_by_platform_id` e reaproveitamento seguro de UUIDs.
  - `SaveLocationsRepo`: método `upsert` idempotente que preserva UUIDs existentes.
  - `FilesRepo`: suporte a `find_by_id` e `find_by_path`.
  - `BackupsRepo`: controle de retenção com método `prune_old_versions`.
  - `SyncQueueRepo`: enfileiramento, listagem de pendentes e controle de tentativas.
  - Implementação estrita de `ON DELETE CASCADE`.

### 3. `savevault-discovery` (Completo)
- **Steam Engine**:
  - Mapeamento dinâmico de múltiplas bibliotecas via leitura do `libraryfolders.vdf`.
  - Parser nativo de arquivos `.acf` para extrair nome, Steam AppID e pasta de instalação sem resíduos de aspas.
- **Ludusavi Manifest Engine**:
  - Download assíncrono do catálogo oficial do Ludusavi (~17MB, +53.000 jogos) com cache local por 7 dias.
  - Parser nativo de YAML com filtragem de tags e regras específicas para Windows.
- **Path Resolver & Suporte a Globs**:
  - Resolução de variáveis de ambiente (`<winAppData>`, `<winDocuments>`, `<winLocalAppData>`, `<root>`).
  - Resolução dinâmica de identificadores de conta Steam (`<storeUserId>`) via expansão de globs reais no disco.

### 4. `savevault-monitor` (Completo)
- **Pipeline de Monitoramento Resiliente**:
  - `SaveScanner`: varredura recursiva de arquivos, diretórios e padrões glob.
  - `SaveWatcherService`: integração com a crate `notify` em canal não-bloqueante (`try_send`) com contadores atômicos contra overflow e monitoramento dinâmico de pastas-mãe.
  - `FileDebouncer`: agrupamento (*coalescing*) causal de múltiplos eventos de escrita em janela temporal configurável, evitando sobrecarga de IO.
  - `StabilityChecker`: motor de detecção de escrita em andamento. Mapeia códigos de erro nativos do Windows (`ERROR_SHARING_VIOLATION` 32 e 33) e aplica retentativas com backoff exponencial e jitter para garantir que saves em gravação ativa pelo jogo não sejam corrompidos ou lidos prematuramente.

### 5. `savevault-backup` (Completo)
- **Criptografia e Hashing**: Cálculo determinístico de assinaturas via **BLAKE3**.
- **SnapshotManager**:
  - Criação de snapshots locais versionados (`backup_root/<file_id>/v<version>/filename`).
  - Rotina de **restauração segura**: gera automaticamente cópia temporária (`.savevault_safety_bak`) antes de substituir o arquivo original, permitindo desfazer rollbacks acidentais.

### 6. `savevault-cloud` (Funcional)
- **Autenticação OAuth 2.0 (`src/auth.rs`)**:
  - Fluxo *Authorization Code com PKCE via Loopback IP* (`127.0.0.1:8085`).
  - Abertura automática do navegador padrão do sistema para consentimento do usuário.
  - Escopo restrito de segurança: `https://www.googleapis.com/auth/drive.file` (acesso exclusivo aos arquivos criados pelo próprio SaveVault).
  - Armazenamento seguro de *Refresh Token* no **Windows Credential Manager** via crate `keyring`.
  - Renovação automática e transparente de *Access Tokens* expirados.
- **Google Drive Provider (`src/google_drive.rs`)**:
  - Criação hierárquica automática de pastas no Drive (`SaveVault/<save_location_id>/<file_id>/`).
  - Upload Multipart (metadados JSON + binário do snapshot em requisição HTTP única).
  - Métodos implementados para download e remoção remota de arquivos.

### 7. `savevault-sync` (Funcional)
- **SyncEngine**:
  - Consumo assíncrono da tabela `sync_queue`.
  - Tratamento de falhas de rede com backoff exponencial (até 5 tentativas antes de marcar como falha definitiva).
  - Atualização idempotente do status de sincronização (`Pending` ➔ `Synced`) e persistência do caminho remoto (`cloud_path`).
  - Operação segura em modo offline (ignora silenciosamente uploads pendentes quando o usuário não estiver autenticado).

### 8. `savevault-app` (Orquestrador & CLI)
- **Binário Principal (`savevault`)**:
  - Orquestra descoberta, resolução e persistência em banco.
  - Aciona o consumidor de sincronização após cada ciclo.
  - Suporte a subcomandos de linha de comando:
    - `savevault auth login`: inicia o fluxo de login no navegador.
    - `savevault auth status`: verifica se há credenciais válidas salvas.
    - `savevault auth logout`: desvincula e limpa o cofre do Windows.
    - `savevault sync`: executa manualmente o processamento da fila de sincronização.
- **Binário de Diagnóstico (`scan_report`)**:
  - Exibe tabela formatada com jogos detectados, AppIDs, status da pasta no disco (`OK` vs `Ausente`) e métricas do catálogo.

---

## 3. Bateria de Testes Automatizados (QA)

Atualmente a suíte conta com **121 testes automatizados (100% de aprovação)** cobrindo todos os módulos críticos:

1. **`savevault-db` (28 testes)**: Idempotência de migrações, índices case-insensitive (`COLLATE NOCASE`), `ON DELETE CASCADE`, consistência de UUIDs e operações CRUD.
2. **`savevault-monitor` (30 testes)**: Coalescência de debouncing, limites de memória, recuperação de sharing violation (locks do Windows), deleção e renomeação atômica.
3. **`savevault-discovery` (28 testes)**: Parsing de manifestos Steam ACF, resolução de variáveis de ambiente Windows, filtros de tag e integração com fixtures oficiais do Ludusavi.
4. **`savevault-core` (22 testes)**: Normalização de paths, roundtrip do arquivo `config.toml`, modelos e validação de UUIDs.
5. **`savevault-backup` (8 testes)**: Determinismo e velocidade do hash BLAKE3, integridade de snapshots versionados e rollback com backup de segurança.
6. **`savevault-cloud` (5 testes)**: Decodificação de parâmetros de requisição HTTP e parsing do fluxo OAuth 2.0.

---

## 4. Decisões Técnicas e Arquiteturais
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

## 5. Próximos Passos (Milestones Futuros)

- [ ] **Daemon de Monitoramento Ativo no `savevault-app`**: Integrar o loop assíncrono do `SaveWatcherService` diretamente ao binário principal para manter o processo em execução contínua em segundo plano após o discovery inicial.
- [ ] **CLI de Restauração (`savevault restore`)**: Criar comandos de terminal para listar snapshots históricos de um jogo e acionar a restauração segura via linha de comando.
- [ ] **Ampliadores de Descoberta (GOG & Genérico)**:
  - Implementar leitura das chaves de registro do Windows (`HKLM\SOFTWARE\GOG.com\Games`).
  - Implementar heurísticas para busca de saves em jogos independentes fora do manifesto.
- [ ] **Interface Desktop & System Tray (v1.0)**:
  - Ícone na bandeja do Windows (System Tray) para monitoramento silencioso.
  - Interface gráfica opcional (Tauri / egui) para visualização amigável de status e backups gugu.

