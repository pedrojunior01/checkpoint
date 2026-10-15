# 📊 Relatório de Implementação e Status do Projeto — checkpoint

**Versão Atual:** v0.9.1 (Release Candidate)  
**Data do Relatório:** Outubro 2026  
**Status do Projeto:** Pipeline Core + Interface Desktop Tauri v2 + System Tray + Multi-Cloud (Google Drive & OneDrive) + Single-File Saves + Monitoramento Otimizado + Camadas Latest/Daily de Restauração Segura

---

## 1. Visão Geral do Projeto
O **checkpoint** (estruturado internamente pelos crates `savevault-*`) é um sistema completo e modular escrito em **Rust**, desenvolvido prioritariamente para Windows 10/11 x64, projetado para:
1. Detectar automaticamente jogos instalados no PC (Steam, GOG Galaxy, Emuladores, Heurística genérica e Ludusavi Manifest).
2. Monitorar em tempo real os diretórios e arquivos de save de forma não-bloqueante e resiliente a travas do jogo (`ERROR_SHARING_VIOLATION`).
3. Gerar snapshots versionados locais criptografados por hash **BLAKE3** com deduplicação perfeita.
4. Sincronizar os backups assincronamente com múltiplos provedores de nuvem (Google Drive via OAuth 2.0 PKCE e Microsoft OneDrive via detecção de pasta local e Microsoft Graph).
5. Fornecer uma interface gráfica desktop nativa e moderna via **Tauri v2** + Vanilla JS/CSS, operando em segundo plano na bandeja do sistema (*System Tray*) com consumo ultraleve de RAM (15MB a 25MB).

---

## 2. Resumo dos Módulos Entregues

| Crate | Status | Cobertura / Entregas Principais |
| :--- | :--- | :--- |
| **`savevault-core`** | **100% Completo** | Modelos de domínio canônicos, manipulação lógica de caminhos Windows, detecção de arquivos internos e gerenciamento de Autostart (`HKCU\...\Run`). |
| **`savevault-db`** | **100% Completo** | SQLite bundled em modo WAL, migrações determinísticas 001 a 005, índices `COLLATE NOCASE`, integridade `ON DELETE CASCADE` e repositórios com upsert idempotente. |
| **`savevault-discovery`** | **100% Completo** | Mapeamento Steam ACF/VDF, GOG Registry (32 e 64-bit), emuladores, heurística genérica anti-falsos-positivos e catálogo Ludusavi (+53.000 jogos). Suporte a arquivos únicos (.sl2, .sav, .dat). |
| **`savevault-monitor`** | **100% Completo** | Scanner físico com limites de traversal, `FileDebouncer` com coalescência, `StabilityChecker` com retry exponencial contra sharing violations e watcher dinâmico de pastas recém-criadas. |
| **`savevault-backup`** | **100% Completo** | Hashing BLAKE3 em streaming constante $O(1)$ de RAM, snapshots versionados (`latest` e `daily`), poda transacional e restauração segura em 4 etapas com Safety Backups. |
| **`savevault-cloud`** | **100% Completo** | Abstração `CloudProvider`, detecção automática de pastas locais do OneDrive e Google Drive, clientes OAuth 2.0 PKCE com armazenamento seguro no Windows Credential Manager e manifesto central de Disaster Recovery (`savevault_manifest.json`). |
| **`savevault-sync`** | **100% Completo** | Motor `SyncEngine` assíncrono com desacoplamento de lock do banco, retentativas com backoff exponencial (até 5 tentativas), dead-lettering e transações atômicas de upload/deleção. |
| **`savevault-app`** | **100% Completo** | Orquestrador unificado do checkpoint, interface gráfica Tauri v2 Desktop (frameless, temas claro/escuro, responsiva), bandeja do sistema (*System Tray*), poda ativa de memória Working Set, 20 comandos IPC e utilitários CLI (`scan_report`, `restore`). |

---

## 3. Bateria de Testes Automatizados (QA)

A suíte de testes automatizados conta com **100% de aprovação** em todos os crates do workspace:
```
cargo test --workspace
```
* **`savevault-discovery` (19 testes)**: Parsing de manifestos Steam ACF, leitura do Registro GOG, expansão segura de variáveis e resolução de manifestos.
* **`savevault-db` (26 testes)**: Migrações, unicidade case-insensitive, persistência referencial, repositórios e idempotência de upsert.
* **`savevault-app` (24 testes)**: Pipeline E2E, dev-ui, worker de sync, reconciliação periódica, recuperação após crash e restauração segura.
* **`savevault-monitor` (26 testes)**: Coalescência de debouncing, buffer limits, tolerância a sharing violations Win32 e renomeação atômica.
* **`savevault-cloud` (13 testes)**: Detecção de pastas locais, fluxo OAuth, mock de upload/delete e persistência de manifesto.
* **`savevault-core` (16 testes)**: Normalização de paths sem prefixos `\\?\`, manipulação de maiúsculas/minúsculas e persistência de config.
* **`savevault-sync` (12 testes)**: Fila transacional, retentativas com backoff e isolamento de falhas.
* **`savevault-backup` (12 testes)**: Hashing BLAKE3 em streaming, criação de snapshots particionados, rotação de versões e safety backups.

---

## 4. Registro de Correções e Melhorias Recentes (v0.9.0-rc)

| Item | Área | Descrição do Problema | Solução Implementada |
| :--- | :--- | :--- | :--- |
| **FIX-01** | `discovery` / `monitor` | Jogos com arquivo de save único (ex: `S0000.sl2` do Sekiro) eram categorizados como diretório, resultando em 0 arquivos escaneados e travando a criação de snapshots. | O scanner agora valida se o caminho é arquivo fisicamente no disco (`is_file()`), o watcher monitora a pasta pai com modo não-recursivo e o discovery infere dinamicamente o tipo de arquivo por extensão conhecida. |
| **FIX-02** | `state.rs` / `app.js` | A lista de "Jogos Recentes" exibia jogos na ordem arbitrária de inserção no banco de dados, ignorando a atividade recente. | Adicionado campo `last_backup_time` no `LocationView`, consulta agregada `MAX(b.created_at)` no SQLite e ordenação decrescente por data de backup. |
| **FIX-03** | `frontend` | A aba "Atividade" e a lista inicial não se atualizavam automaticamente quando novos saves eram gerados enquanto a aba estava aberta. | O timer periódico de 10s agora atualiza a aba ativa (`loadInicio` ou `loadAtividade`), e o feed de atividade agora traz os snapshots mais recentes no topo. |
| **FIX-04** | `cloud` | Falta de suporte simples a nuvem local para usuários sem conta ou chaves de API do Google Cloud. | Implementado `LocalFolderProvider` e detecção automática de pastas locais do **Microsoft OneDrive** (`detect_cloud_folders`). |
| **FIX-05** | `desktop` | Consumo de RAM da WebView2 ultrapassando a proposta leve do projeto. | Implementada rotina `trim_process_memory` que descarta o Working Set do processo principal e de todos os subprocessos filhos da WebView2 em minimizações, ocultações e a cada 30 segundos, além de flags rígidas no Chromium V8 (32MB max heap). |
| **FIX-06** | `frontend` | Falhas visuais em modo escuro (textos sumindo e falta de contraste) e ícones de nuvem genéricos. | Correção de variáveis CSS de contraste no `.dark` e inclusão de ícones vetoriais SVG oficiais para Google Drive e Microsoft OneDrive. |
| **FIX-07** | `branding` / `cloud` | Renomeação global de SaveVault para **checkpoint**, necessitando migração transparente de dados legados no Google Drive e `%APPDATA%`. | Implementada migração automática e retrocompatibilidade (`savevault_cloud::auto_migrate_legacy_cloud_folder` para `G:\Meu Drive\checkpoint`), suporte unificado a `%APPDATA%\checkpoint` e `%APPDATA%\SaveVault`, e definição da feature `desktop` como padrão no `Cargo.toml`. |
| **FIX-08** | `core` / `monitor` | O No Man's Sky acumulava 46 arquivos monitorados e inundava a aba "Atividade" com backups de miniaturas e texturas transitórias `.DDS` em subpastas de cache (`st_...\cache`). | Adicionada filtragem inteligente com `is_ignored_relative_save_path` e `filter_entry` no `SaveScanner` e `SaveWatcherService` (ignorando pastas `cache`, `CrashDumps`, `crashes`, `shadercache` e arquivos `.dds`, `.dmp`, `.mdmp`, `.crash`). Realizada limpeza do banco e do disco, reduzindo o NMS para estritamente seus 8 saves reais. |
| **FIX-09** | `state.rs` / `frontend` | Jogos genéricos (`KOFXV`, `MortalShell2`, `RSDragonwilds`, `PioneerGame`) exibiam apenas crachás com iniciais (KO, MO, RS, PI) sem capas oficiais. | Implementado dicionário inteligente de aliases (`map_game_name_to_steam_id`), motor dinâmico assíncrono de busca na Steam Store Search API (`resolve_cover_by_name`), persistência em `steam_covers.json` e substituição dinâmica de crachás no Vanilla JS. |
| **FIX-10** | `restore` / `frontend` | Comportamento e distinção opaca entre saves `latest` e `daily`, gerando incerteza ao usuário final sobre qual versão restaurar em caso de corrupção. | Implementada diferenciação explícita no backend (`target_type: Option<String>`), comandos dedicados de restauração (`restore_game_saves`) e botões claros no modal de detalhes do jogo (**Restaurar Último Save** vs **Restaurar Save Diário**) e na aba Atividade, acompanhados de banner informativo e Safety Backup prévio. |
| **FIX-11** | `discovery` / `state` | A Steam incluía indevidamente a pasta inteira de instalação (`steamapps\common\<Game>`) como save location quando o manifest não mapeava o jogo, resultando em backups de arquivos de 18 GB (`.pak`) e DLLs. | Removido o fallback perigoso no `savevault-discovery`, restringindo a busca estritamente a pastas de save autênticas (`Steam/userdata/<id>/<app_id>` e pastas `saves/savedata`). Adicionada rotina de auto-limpeza em `DevUiState::get_locations` que expurga registros antigos apontando para `steamapps\common`. |
| **FIX-12** | `jogos` / `state` | Impossibilidade de remover jogos indesejados detectados automaticamente ou limpar o histórico de jogos monitorados. | Adicionado comando `remove_game_location` e `clean_unrecognized_games`, botão de remoção individual com confirmação no modal do jogo e tabela de blacklist (`ignored_locations`) no SQLite para evitar re-detecção involuntária. |
| **FIX-13** | `discovery` | O botão de varredura executava busca profunda excessiva em todo o sistema de arquivos, causando lentidão perceptível. | O scanner de descoberta heurística foi refatorado para restringir a busca exclusivamente aos diretórios legítimos de saves (subpastas de AppData Local/Roaming, Saved Games, Documents, emuladores e pastas de plataformas suportadas). |
| **FIX-14** | `frontend` | A seção de jogos recentes mostrava apenas 4 itens e o botão de varredura ficava travado em animação caso houvesse duplicidade de chamadas. | Expandida a lista de jogos recentes na tela inicial para 6-8 títulos e unificados os gatilhos de varredura com debounce e feedback via toast. |

---

## 5. Próximos Passos (Para a v1.0 Final)

- [x] Interface Desktop Definitiva com Tauri v2 (`checkpoint.exe`).
- [x] Ícone na bandeja do Windows (System Tray) para monitoramento silencioso minimizado.
- [x] Suporte integrado a Microsoft OneDrive e Google Drive (`G:\Meu Drive\checkpoint`).
- [x] Otimização rigorosa de consumo de RAM (15MB - 25MB).
- [x] Suporte a saves de arquivo único e diretórios completos.
- [x] Ordenação dinâmica da tela inicial e feed de atividades em tempo real.
- [x] Filtragem inteligente anti-ruído de caches e dumps de engines (`cache/`, `.dds`, `.dmp`).
- [x] Resolução automática de capas oficiais em alta resolução para jogos genéricos e abreviações.
- [x] Restauração granular segura em 4 etapas com separação visual de camadas (`latest` e `daily`).
- [x] Prevenção de falso-positivo de instalação Steam e exclusão individual de jogos indesejados.
- [x] Script de instalador Windows Inno Setup (`installer/checkpoint.iss`) e script de empacotamento (`scripts/package-release.ps1`).
- [x] Pipeline de CI/CD automatizado no GitHub Actions (`.github/workflows/ci.yml` e `release.yml`).
- [x] Licença aberta canônica MIT (`LICENSE`).
- [x] Documentação bilíngue e guias comunitários de contribuição (`CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`).
- [ ] Suporte a múltiplos perfis de save por jogo (slots manuais customizados).

