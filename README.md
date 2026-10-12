# checkpoint 🎮

> **Motor inteligente de backup automático e sincronização de saves de jogos para Windows.**  
> Escrito 100% em **Rust**, com interface Desktop nativa via **Tauri v2**, consumo ultraleve (**15MB a 25MB RAM**), residência silenciosa na bandeja do sistema (*System Tray*) e suporte híbrido à nuvem (**Google Drive** e **Microsoft OneDrive**).

---

## 🚀 O que é o checkpoint?

O **checkpoint** opera silenciosamente em segundo plano enquanto você joga, com uma missão simples: **garantir que nenhum minuto do seu progresso seja perdido**.

Ele detecta seus jogos instalados e seus respectivos diretórios de save, monitora gravações em tempo real com debouncing inteligente, calcula hashes criptográficos **BLAKE3** para eliminar duplicações, armazena snapshots locais versionados e sincroniza tudo de forma transacional com a nuvem (suportando tanto pastas locais sincronizadas como o Google Drive Desktop e OneDrive quanto APIs diretas).

---

## ✨ Funcionalidades Principais

| Recurso | Descrição |
| :--- | :--- |
| 🔍 **Discovery Multilocalizador** | Identifica jogos via Steam (`.acf`/`.vdf`), GOG Galaxy (Registro 32/64-bit), emuladores (Dolphin, PCSX2, PPSSPP, RetroArch) e heurística genérica para jogos isolados. |
| 📖 **Catálogo Ludusavi (+53.000 Jogos)** | Mapeamento automático de caminhos de save com resolução de variáveis Win32 (`<winAppData>`, `<winDocuments>`). |
| 🖼️ **Capas em Alta Definição (600x900)** | Resolução oficial de artes verticais da Steam tanto para títulos de loja quanto para jogos genéricos via Steam Store Search API e cache persistente (`steam_covers.json`). |
| 🧹 **Filtro Inteligente Anti-Ruído** | Poda preventiva de caches gráficos temporários (arquivos `.DDS`, subpastas `CACHE`, `SHADERCACHE`, `.crash`), evitando falsas notificações e spam no banco de dados. |
| 👁️ **Monitoramento Não-Bloqueante** | Watcher Win32 (`ReadDirectoryChangesW`) com debouncing adaptativo e tolerância nativa a travas de escrita de jogos (`ERROR_SHARING_VIOLATION`). |
| 🔐 **Integridade Criptográfica BLAKE3** | Cálculo de hash em streaming constante $O(1)$ de RAM (blocos de 64 KB) com deduplicação perfeita (*zero duplication*). |
| ☁️ **Nuvem Híbrida & Disaster Recovery** | Sincronização automática para Google Drive (`G:\Meu Drive\checkpoint`) e Microsoft OneDrive, com manifesto central (`savevault_manifest.json`) para restaurar tudo em PCs novos. |
| 🖥️ **Interface Desktop Nativa (Tauri v2)** | UI moderna frameless, temas claro e escuro, sem dependência de Node.js em produção, operando na bandeja do sistema (*System Tray*). |
| ⚡ **Otimização de RAM (15MB - 25MB)** | Poda ativa de memória Working Set (`SetProcessWorkingSetSize`) do processo principal e subprocessos da WebView2 ao minimizar ou ficar ocioso. |
| 🛡️ **Restauração Segura em 4 Etapas** | Geração obrigatória de **Safety Backups** (`.savevault_safety_*`) antes de tocar no arquivo ativo, com substituição atômica Win32 (`MoveFileExW`). |

---

## 📋 Requisitos

* **Sistema Operacional:** Windows 10 (Build 19041+) ou Windows 11 x64.
* **Rust:** 1.75+ com toolchain MSVC (`stable-x86_64-pc-windows-msvc`).
* **WebView2 Runtime:** Já instalado nativamente no Windows 10/11.

---

## ⚡ Instalação & Compilação

Clone o repositório e compile o projeto com o Cargo:

```powershell
git clone https://github.com/pedrojunior01/Savevalt.git checkpoint
cd checkpoint
```

### 1. Executar a Interface Desktop (Modo Padrão)
A feature `desktop` já vem habilitada por padrão:
```powershell
cargo run -p savevault-app
```

### 2. Compilar o Binário Release Standalone Otimizado
```powershell
cargo build -p savevault-app --release
```
O executável final independente será gerado em:
```
target\release\checkpoint.exe
```

### 3. Executar em Modo Headless (Sem Interface Gráfica / CLI)
Para servidores, terminais puros ou automações:
```powershell
# Executa apenas a varredura e imprime o relatório formatado
cargo run -p savevault-app --bin scan_report

# Executa o daemon contínuo em segundo plano no terminal
cargo run -p savevault-app --no-default-features
```

---

## 🧪 Bateria de Testes Automatizados

O projeto conta com mais de 115 testes cobrindo todos os fluxos críticos de persistência, concorrência, caminhos Windows e sincronização:

```powershell
cargo test --workspace
```

---

## 📁 Estrutura do Workspace Cargo

O ecossistema é particionado em 8 crates modulares altamente coesos e desacoplados:

```
checkpoint/
├── crates/
│   ├── savevault-core/       # Modelos canônicos de domínio, caminhos Windows e autostart
│   ├── savevault-db/         # Persistência SQLite bundled em modo WAL com migrações
│   ├── savevault-discovery/  # Detectores Steam, GOG, Emuladores, Heurística e Ludusavi
│   ├── savevault-monitor/    # Watcher notify, debouncer, checagem de estabilidade e poda de cache
│   ├── savevault-backup/     # Hashing BLAKE3 em streaming e gerenciador de snapshots
│   ├── savevault-cloud/      # Abstração de nuvem, pastas locais (OneDrive/GDrive) e OAuth 2.0
│   ├── savevault-sync/       # Motor assíncrono transacional de sincronização com retry
│   └── savevault-app/        # Executável checkpoint.exe, Tauri v2 Desktop, IPC e CLI
├── docs/                     # Documentação técnica e guias de arquitetura
│   ├── ARCHITECTURE.md       # Diagrama de pipeline ponta a ponta e concorrência
│   ├── CRATES_GUIDE.md       # Análise técnica aprofundada dos 8 crates
│   ├── DATABASE_SCHEMA.md    # Esquema SQLite, migrações e chaves CASCADE
│   ├── DEVELOPMENT_GUIDE.md  # Guia para novos desenvolvedores e boas práticas
│   ├── FRONTEND_AND_IPC.md   # Catálogo completo dos comandos IPC e design system
│   ├── PROJECT_STATUS.md     # Relatório de status e histórico de correções
│   ├── README.md             # Índice da documentação técnica
│   └── SPECIFICATION.md      # Especificação funcional e regras de negócio
└── Cargo.toml                # Workspace Cargo raiz
```

---

## 📖 Documentação Completa

Para aprofundar nos detalhes de implementação, consulte a pasta [`docs/`](docs/):

* [🏛️ Arquitetura do Sistema](docs/ARCHITECTURE.md)
* [📦 Guia Detalhado dos Crates](docs/CRATES_GUIDE.md)
* [🖥️ Frontend & Contrato IPC](docs/FRONTEND_AND_IPC.md)
* [🗄️ Esquema do Banco de Dados](docs/DATABASE_SCHEMA.md)
* [🛠️ Guia do Desenvolvedor](docs/DEVELOPMENT_GUIDE.md)
* [📊 Status do Projeto & Roadmap](docs/PROJECT_STATUS.md)

---

## 📄 Licença

Distribuído sob a licença **MIT**. Consulte o arquivo [LICENSE](LICENSE) para mais detalhes.
