# 🖥️ Frontend & Contrato IPC — SaveVault

Este documento detalha a camada de interface desktop do **SaveVault**, baseada em **Tauri v2** + **Vanilla JS/CSS**, cobrindo o catálogo de comandos IPC, o gerenciamento de estado no backend (`DevUiState`), o design system responsivo e as otimizações agressivas de consumo de memória RAM.

---

## 1. Arquitetura da Interface Desktop

O SaveVault opta por uma abordagem de **altíssimo desempenho e zero overhead**, dispensando NodeJS, React, Electron ou bundlers pesados em tempo de execução:

```
crates/savevault-app/ui/frontend/
├── index.html   # Estrutura semântica (Titlebar, Sidebar, Abas, Modais, Toasts)
├── style.css    # Design System (~1000 linhas de CSS puro com variáveis modernas)
└── app.js       # Controlador Vanilla JS (ES6+) que gerencia estado e chamadas IPC
```

### Características da Camada Visual:
* **Janela Frameless Customizada:** A titlebar nativa do Windows é substituída por uma barra personalizada com controle de arrasto (`data-tauri-drag-region`) e botões nativos estilizados (Minimizar, Maximizar, Fechar).
* **System Tray Integrada:** Ao fechar a janela no botão "X", o evento `CloseRequested` é interceptado via `api.prevent_close()` e a janela é apenas ocultada (`window.hide()`), continuando a operar em segundo plano na bandeja do sistema com menu de contexto.
* **Otimização de Memória (15MB - 25MB RAM):** A cada evento de minimizar, ocultar, perda de foco ou a cada 30 segundos, o processo invoca `SetProcessWorkingSetSize` tanto no processo principal quanto em todos os processos filhos da WebView2 (renderer, GPU process, network service).
* **Temas Claro e Escuro Nativo:** Suporte completo com persistência em `localStorage` e variáveis CSS dinâmicas (`.dark`).

---

## 2. Catálogo Completo de Comandos IPC

Todas as chamadas frontend utilizam o wrapper assíncrono oficial do Tauri v2:
```javascript
const { invoke } = window.__TAURI__.core;
const resultado = await invoke("nome_do_comando", { parametro: valor });
```

Abaixo está a especificação completa de todos os **20 comandos IPC** registrados no backend (`tauri_app.rs`):

| Comando | Parâmetros Rust / JS | Retorno (Rust / JS) | Descrição e Comportamento |
| :--- | :--- | :--- | :--- |
| `window_minimize` | `window: WebviewWindow` | `Result<(), String>` | Minimiza a janela e força a liberação imediata de memória ociosa (`trim_process_memory`). |
| `window_toggle_maximize`| `window: WebviewWindow` | `Result<(), String>` | Alterna a janela entre o estado maximizado e a dimensão restaurada normal. |
| `window_close` | `window: WebviewWindow` | `Result<(), String>` | Oculta a janela (`window.hide()`), mantendo o daemon ativo no System Tray. |
| `get_config` | *(nenhum)* | `Result<ConfigDto, String>` | Retorna os parâmetros de configuração ativos no `config.toml` e o status de autostart no Registro. |
| `save_config` | `new_cfg: ConfigDto` | `Result<String, String>` | Valida e salva novas configurações no `config.toml` e atualiza a chave `HKCU\...\Run` do Windows. |
| `open_path_in_explorer` | `path: String` | `Result<(), String>` | Abre o diretório no Windows Explorer (`explorer.exe`), criando a pasta se ela ainda não existir. |
| `open_folder` | `path: String` | `Result<(), String>` | Alias ergonômico para `open_path_in_explorer`. |
| `get_dashboard_status` | *(State Arc<DevUiState>)* | `Result<DashboardSummary, String>` | Retorna contadores agregados para os KPIs (total de jogos, saves monitorados, pendências em nuvem). |
| `get_locations` | *(State Arc<DevUiState>)* | `Result<Vec<LocationView>, String>` | Retorna os jogos monitorados ordenados pela **data do backup mais recente decrescente**, associando capas da Steam. |
| `get_snapshots` | *(State Arc<DevUiState>)* | `Result<Vec<SnapshotView>, String>` | Lista todos os snapshots versionados ordenados cronologicamente do mais novo para o mais antigo. |
| `get_safety_backups` | *(State Arc<DevUiState>)* | `Result<Vec<SafetyBackupGroup>, String>` | Varre o disco procurando cópias de escape automáticas (`.savevault_safety_*`) geradas para cada jogo. |
| `get_reconciliation_status`| *(State Arc<DevUiState>)* | `Result<ReconciliationView, String>` | Informa se o reconciliador está rodando, o intervalo e os resultados da última varredura. |
| `trigger_reconcile` | *(State Arc<DevUiState>)* | `Result<String, String>` | Dispara manualmente uma varredura de reconciliação sob demanda e reporta os arquivos processados. |
| `restore_snapshot` | `snapshot_id: String` | `Result<RestoreApiResponse, String>` | Executa a rotina de restauração segura em 4 etapas (validação de hash, Safety Backup e rename atômico). |
| `add_game` | `name: String`, `path: String` | `Result<LocationView, String>` | Cadastra manualmente um novo jogo (`Platform::Generic`) e seu local de save no SQLite. |
| `optimize_database` | *(State Arc<DevUiState>)* | `Result<OptimizationResult, String>` | Remove itens concluídos antigos (>30 dias) da fila de sync e executa `VACUUM` no SQLite. |
| `export_backup_zip` | *(State Arc<DevUiState>)* | `Result<ExportResult, String>` | Compacta todo o diretório de backups em um arquivo `.zip` na pasta Downloads do usuário via PowerShell. |
| `get_cloud_config` | *(State Arc<DevUiState>)* | `Result<CloudConfigView, String>` | Retorna a configuração de nuvem atual e pastas de nuvem detectadas na máquina (OneDrive, etc.). |
| `update_cloud_config` | `provider: Option<String>`, `local_sync_path: Option<String>`, `enabled: Option<bool>` | `Result<CloudConfigView, String>` | Atualiza os parâmetros de nuvem e notifica o `SyncWorker` em tempo de execução via canal interno. |
| `resolve_game_cover` | `app_id: String` | `Result<Option<String>, String>` | Resolve a imagem oficial da capa do jogo via cache em memória, arquivo local ou API da Steam. |

---

## 3. Gerenciamento de Estado no Backend (`DevUiState`)

O `DevUiState` atua como facade unificado de estado em memória para toda a aplicação:

```rust
pub struct DevUiState {
    pub db: Arc<Mutex<Connection>>,
    pub db_path: PathBuf,
    pub backup_root: PathBuf,
    pub reconciler: Option<Arc<Reconciler>>,
    pub reconciliation_interval: Duration,
    pub watcher_active: AtomicBool,
    pub reconciler_active: AtomicBool,
    event_counter: AtomicU64,
    recent_events: Mutex<VecDeque<DevUiEvent>>,
    last_error: Mutex<Option<String>>,
    pub sync_worker: Mutex<Option<Arc<SyncWorker>>>,
    pub steam_cover_cache: std::sync::Mutex<HashMap<String, String>>,
}
```

### Cache de Capas em 3 Níveis (`steam_covers.json`):
1. **Nível 1 (Memória RAM):** `steam_cover_cache: Mutex<HashMap<String, String>>`.
2. **Nível 2 (Arquivo Local):** `%APPDATA%\SaveVault\steam_covers.json`. Cada nova capa descoberta via API é persistida atomicamente no disco.
3. **Nível 3 (Rede com Fallback):** Consulta HTTP à API oficial da Steam com timeout de 3.5s (`https://store.steampowered.com/api/appdetails?appids={app_id}&filters=basic`). Se indisponível ou offline, recorre à URL estática da Cloudflare CDN Steam. Se falhar, renderiza um avatar com gradiente CSS e as iniciais do jogo.

---

## 4. Componentes e Padrões da Interface

### 4.1. Navegação por Abas
A navegação ocorre sem recarregamento de página (*Single Page Application*):
* **Início (`nav-inicio`):** Cards de KPIs (Jogos, Saves, Tempo), status da sincronização com a nuvem e lista dinâmica de **Jogos Recentes** com selos de tempo relativo (`Backup há X min`, `agora mesmo`).
* **Jogos (`nav-jogos`):** Lista completa de jogos com campo de pesquisa em tempo real, caminhos físicos e modal de detalhes.
* **Atividade (`nav-atividade`):** Feed cronológico decrescente de todos os snapshots com filtro por jogo e botão de restauração.
* **Nuvem (`nav-nuvem`):** Status dos provedores, detecção automática de pastas locais (OneDrive/Google Drive) e botão de alternância.
* **Configurações (`nav-config`):** Parâmetros de retenção de versões, tempo de debounce, autostart e ferramentas de manutenção (otimização do banco e exportação em ZIP).

### 4.2. Diálogos e Feedback Visual
* **Modal de Confirmação In-App (`showConfirmDialog`):** Diálogo assíncrono baseado em `Promise<boolean>` estilizado com a paleta do app, eliminando o feio e bloqueante `window.confirm()` nativo do browser.
* **Sistema de Toasts Flutuantes (`showToast`):** Notificações de feedback visual no canto da tela com auto-dismiss após 4 segundos e estilos distintos para `success`, `error` e `info`.
* **Formatação de Tempo Relativo (`timeAgo`):** Converte strings ISO 8601 em descrições amigáveis em português (`agora mesmo`, `há 15 min`, `há 2h`, `10 de out`).
* **Polling Leve Inteligente:** Um `setInterval` de 10 segundos atualiza periodicamente apenas a aba que está visível no momento (`nav-inicio` ou `nav-atividade`), sem desperdiçar ciclos de CPU em abas ocultas.
