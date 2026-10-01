# SaveVault

> Motor de backup automático de saves de jogos para Windows, feito em Rust.  
> Silencioso, leve (~15MB RAM) e com sincronização nativa na nuvem.

---

## O que é o SaveVault?

SaveVault é uma solução em Rust que roda em segundo plano e protege automaticamente os arquivos de save dos seus jogos no Windows. Ele detecta jogos instalados, monitora alterações nos arquivos de save em tempo real e mantém backups sincronizados com a nuvem — sem interromper sua gameplay e sem consumir espaço infinito no disco.

---

## Funcionalidades

| Recurso | Descrição |
|---|---|
| 🔍 **Discovery Automático** | Detecta jogos via Steam, GOG, emuladores e heurística genérica |
| 📖 **Manifesto Ludusavi** | Base de dados de +30.000 jogos com caminhos de save conhecidos |
| 👁️ **Watcher em Tempo Real** | Monitora saves com debounce inteligente (evita backups duplicados) |
| 🔐 **Integridade BLAKE3** | Cada save é verificado por hash antes e após a cópia |
| ☁️ **Motor de 2 Pastas** | Estrutura `latest`/`daily` para sync nativo via Google Drive, OneDrive ou Dropbox |
| 🔄 **Sincronização Cloud** | Fila de upload atômica com retry e backoff exponencial |
| 🗑️ **Pruning Automático** | Limpa registros do banco SQLite com mais de 30 dias automaticamente |
| 🔁 **Restauração Segura** | CLI para restaurar qualquer snapshot com safety backup do estado atual |
| 🛡️ **Deduplicação** | Previne entradas duplicadas quando um jogo é detectado por múltiplas fontes |

---

## Requisitos

- **Windows 10 ou 11** (64-bit)
- **Rust 1.80+** com Cargo (para compilar a partir do código-fonte)

---

## Instalação

```powershell
git clone https://github.com/pedrojunior01/Savevalt.git
cd Savevalt
cargo build --release
```

O executável será gerado em `target/release/savevault.exe`.

---

## Como Usar

### 1. Iniciar o monitoramento contínuo

```powershell
cargo run -p savevault-app
```

Na primeira execução, o SaveVault irá automaticamente:
1. Criar o arquivo de configuração em `%APPDATA%\SaveVault\config.toml`
2. Inicializar o banco de dados SQLite em `%APPDATA%\SaveVault\savevault.db`
3. Escanear os jogos instalados e mapear seus diretórios de save
4. Entrar em modo **Watcher** — monitorando saves em tempo real

### 2. Abrir o Dashboard Web (Dev UI)

```powershell
cargo run -p savevault-app -- ui
```

Acesse `http://127.0.0.1:4040` no navegador para ver:
- Lista de jogos protegidos e seus locais de save
- Status do sistema e da sincronização
- Histórico dos últimos snapshots realizados

### 3. Adicionar um jogo manualmente

```powershell
cargo run -p savevault-app -- add "Nome do Jogo" "C:\Caminho\Para\Saves"
```

### 4. Restaurar um save

```powershell
# Listar snapshots disponíveis
cargo run -p savevault-app -- restore list

# Restaurar um snapshot específico pelo ID
cargo run -p savevault-app -- restore <id_do_snapshot>
```

A restauração cria automaticamente um **safety backup** do arquivo atual antes de sobrescrever.

---

## Como funciona o Motor de 2 Pastas

Quando o jogo salva, o SaveVault detecta a mudança e cria:

```
📁 SaveVault_Backups/
└── 📁 ELDEN RING/
    ├── 📁 latest/        ← Sobrescrito a cada save (o mais recente)
    │   └── ER0000.sl2
    └── 📁 daily/         ← Copiado uma vez por dia (segurança diária)
        └── ER0000.sl2
```

**Por que essa estrutura?**
- Ocupa espaço mínimo no disco (sem versões `v1`, `v2`, `v3`...)
- A pasta `latest` sincroniza com o Google Drive, que mantém o **histórico de versões nativo** no servidor — você pode recuperar qualquer ponto no tempo pelo site do Google Drive
- A pasta `daily` garante um ponto de restauração seguro mesmo sem internet

---

## Configuração

Edite o arquivo em `%APPDATA%\SaveVault\config.toml` para ajustar:

```toml
[general]
log_level = "info"
debounce_ms = 2000          # Espera X ms após o último save antes de fazer backup
reconciliation_interval_secs = 300

[paths]
backup_dir = "C:\\Users\\SEU_USUARIO\\Google Drive\\SaveVault_Backups"
```

---

## Testes

```powershell
cargo test --workspace
```

Cobertura atual: **~120 testes** passando em todos os módulos (app, backup, sync, discovery, monitor, db).

---

## Estrutura do Projeto

```
SaveValt/
├── crates/
│   ├── savevault-app/        # Aplicação principal, CLI e Dev-UI
│   ├── savevault-backup/     # Motor de snapshot (BLAKE3, latest/daily)
│   ├── savevault-cloud/      # Integração Google Drive
│   ├── savevault-core/       # Modelos de dados e utilitários de caminho
│   ├── savevault-db/         # Repositórios SQLite e migrações
│   ├── savevault-discovery/  # Detecção de jogos (Steam, GOG, Ludusavi, genérico)
│   ├── savevault-monitor/    # Watcher de filesystem e debouncer
│   └── savevault-sync/       # Motor de sincronização cloud com retry
└── docs/
    └── PROJECT_STATUS.md
```

---

## Licença

Distribuído sob a licença **MIT**. Consulte o arquivo `LICENSE` para detalhes.
