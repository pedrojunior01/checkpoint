# 🎮 checkpoint — Documentação Técnica & Guia de Desenvolvimento

Bem-vindo à documentação oficial do **checkpoint** (versão `v0.9.1`).  
Este conjunto de documentos foi elaborado para desenvolvedores, arquitetos de software e contribuidores que desejam entender, estender ou manter o ecossistema do **checkpoint**.

---

## 📌 Visão Geral do Projeto

O **checkpoint** (estruturado a partir dos crates internos `savevault-*`) é um ecossistema nativo e modular escrito em **Rust**, desenvolvido prioritariamente para Windows (10/11 x64), com o objetivo de:
1. **Detectar automaticamente jogos instalados e seus saves** (Steam, GOG Galaxy, Emuladores e heurística genérica, enriquecidos com o catálogo de +53.000 títulos do Ludusavi Manifest).
2. **Monitorar alterações em tempo real no filesystem** de forma não-bloqueante, tolerante a arquivos bloqueados temporariamente pelo jogo (`ERROR_SHARING_VIOLATION`) e com coalescência inteligente (*debouncing*).
3. **Gerar snapshots versionados locais** com garantia de integridade criptográfica via **BLAKE3**, eliminando backups duplicados (*zero-duplication*) e oferecendo rotinas seguras de restauração com **Safety Backups** e seleção entre *Latest* e *Daily*.
4. **Sincronizar snapshots de forma assíncrona com a nuvem** através de pastas sincronizadas locais (Microsoft OneDrive, Google Drive Desktop, Dropbox) ou APIs na nuvem (Google Drive e OneDrive via OAuth 2.0 PKCE).
5. **Oferecer uma interface Desktop nativa e ultraleve** baseada em **Tauri v2** + Vanilla JS/CSS (sem NodeJS em tempo de execução), consumindo entre 15MB e 25MB de RAM e operando em segundo plano na bandeja do sistema (*System Tray*).

---

## 🗺️ Índice da Documentação

A documentação está subdividida em guias temáticos para facilitar a navegação:

| Documento | Descrição |
| :--- | :--- |
| **[Arquitetura do Sistema](ARCHITECTURE.md)** | Visão geral da arquitetura de software, diagrama de pipeline ponta a ponta, modelo de concorrência e decisões de design do checkpoint. |
| **[Guia Detalhado dos Crates](CRATES_GUIDE.md)** | Análise módulo a módulo dos 8 crates do workspace Cargo, com tipos públicos, responsabilidades e casos de borda. |
| **[Esquema do Banco de Dados](DATABASE_SCHEMA.md)** | Estrutura completa do SQLite (tabelas, chaves estrangeiras com `CASCADE`, migrações 001 a 005, índices `NOCASE` e integridade ACID). |
| **[Frontend & Contrato IPC](FRONTEND_AND_IPC.md)** | Camada Desktop Tauri v2 do checkpoint, catálogo completo dos 24 comandos IPC, gerenciamento de estado (`DevUiState`), design system, modais e poda de memória. |
| **[Guia do Desenvolvedor & Contribuidor](DEVELOPMENT_GUIDE.md)** | Como configurar o ambiente, compilar (Desktop vs. Headless), rodar a suíte de testes, adicionar novos detectores de jogos e novos provedores de nuvem. |
| **[Especificação & Requisitos](SPECIFICATION.md)** | Especificação canônica do produto checkpoint, proposta de valor, regras de negócio e limites de escopo. |
| **[Status do Projeto & Roadmap](PROJECT_STATUS.md)** | Relatório do estado atual de entrega (`v0.9.1`), auditoria de qualidade, correções recentes e próximos passos. |

---

## 🏗️ Mapa dos Crates do Workspace

O código-fonte do checkpoint é particionado em **8 crates autônomos** sob a pasta [`crates/`](../crates/):

```
checkpoint/
├── crates/
│   ├── savevault-core/       # Modelos canônicos de domínio, caminhos Windows e config.toml
│   ├── savevault-db/         # Persistência SQLite estática, migrações e repositórios CRUD
│   ├── savevault-discovery/  # Detectores de launchers (Steam, GOG, Emuladores, Heurística e Ludusavi)
│   ├── savevault-monitor/    # Watcher notify, debouncer com coalescing, verificação de estabilidade Win32 e scanner
│   ├── savevault-backup/     # Hashing BLAKE3 em streaming, versionamento de snapshots e restauração com safety backup
│   ├── savevault-cloud/      # Abstração CloudProvider, detecção de pastas locais (OneDrive) e clientes OAuth 2.0
│   ├── savevault-sync/       # Motor assíncrono de consumo da sync_queue e upload com retries e backoff exponencial
│   └── savevault-app/        # Executável do checkpoint, runtime Tauri v2 Desktop, CLI e daemon residente
├── docs/                     # Documentação técnica e guias de arquitetura
├── scripts/                  # Scripts auxiliares de build e automação
├── Cargo.toml                # Workspace Cargo raiz
└── Cargo.lock
```

---

## ⚡ Começando em 3 Minutos (Quickstart)

### Pré-requisitos
* **Sistema Operacional:** Windows 10/11 x64
* **Rust:** 1.75+ (com toolchain `msvc` recomendada: `x86_64-pc-windows-msvc`)
* **WebView2 Runtime:** Já instalado nativamente no Windows 10/11

### 1. Clonar o repositório
```powershell
git clone https://github.com/pedrojunior01/Savevalt.git checkpoint
cd checkpoint
```

### 2. Rodar a suíte completa de testes unitários e de integração
```powershell
cargo test --workspace
```
*(Todos os crates possuem cobertura automatizada com SQLite em memória e testes de concorrência).*

### 3. Executar o checkpoint com interface Desktop (Tauri v2)
A feature `desktop` já vem habilitada por padrão:
```powershell
cargo run --package savevault-app
```
*(Para compilar a versão final release: `cargo build --package savevault-app --release`, gerando `target/release/checkpoint.exe`).*

### 4. Executar em modo headless (CLI / Scanner de relatório)
```powershell
# Executa apenas a varredura e exibe o relatório no terminal
cargo run --package savevault-app --bin scan_report

# Inicia o daemon residente em modo texto (sem GUI)
cargo run --package savevault-app --no-default-features
```

---

## 🔒 Princípios de Segurança e Integridade

1. **Nunca perder um save do jogador:** Qualquer substituição de save (ex: ao restaurar um snapshot anterior) cria compulsoriamente um **Safety Backup** (`.savevault_safety_<uuid>_<n>`) no mesmo diretório antes de alterar o arquivo ativo.
2. **Escrita Atômica:** Nenhum arquivo é sobrescrito diretamente por streaming; a escrita ocorre em um arquivo de staging temporário (`.restore_tmp_<uuid>`) no mesmo volume NTFS e é promovida via renomeação atômica (`MoveFileExW` com `MOVEFILE_REPLACE_EXISTING`).
3. **Imunidade a Travamentos de Jogo:** O monitor não tenta ler arquivos enquanto jogos ativos estiverem gravando dados; códigos de erro Win32 `ERROR_SHARING_VIOLATION` (32/33) são interceptados e entram em espera exponencial sem gerar falsos positivos.
4. **Sem Dependência de Node.js em Produção:** A interface visual é Vanilla JS/CSS servida diretamente pelo executável compilado do checkpoint, sem node_modules ou pacotes npm em tempo de execução.
