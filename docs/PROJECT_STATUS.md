# Relatório de Implementação e Arquitetura - SaveVault

**Versão Atual:** v0.4.1  
**Data do Relatório:** Setembro 2026

## Visão Geral do Projeto
O SaveVault é um sistema *CLI-first* escrito em Rust projetado para detectar automaticamente jogos instalados, mapear seus diretórios de save, monitorar alterações em tempo real, criar backups incrementais (snapshots) e sincronizar esses arquivos de forma segura com a nuvem (ex: Google Drive).

A arquitetura escolhida foi a de **Workspace Modular**, separando responsabilidades em múltiplos *crates* autônomos, focando em extrema confiabilidade, baixo uso de CPU e tolerância a falhas.

---

## Crates e Módulos Implementados

### 1. `savevault-core` (Completo)
- Define as estruturas de dados universais (`Game`, `SaveLocation`, `BackupSnapshot`, `FileEntry`).
- Gerencia o motor de configuração do usuário via `%APPDATA%\SaveVault\config.toml` (ex: limites de versionamento de backups).
- Garante o isolamento de modelos para não criar dependências circulares entre crates.

### 2. `savevault-db` (Completo)
- Banco de dados **SQLite** totalmente embutido via `rusqlite` com a flag `bundled`, dispensando a instalação de drivers externos no Windows.
- **Sistema de Migrações Idempotente**: Migrações (`001_initial_schema`, `002_save_locations_unique_path`) rodam de forma determinística na inicialização.
- **Repositórios CRUD**: 
  - `GamesRepo` (com `INSERT OR IGNORE` e reaproveitamento seguro de IDs).
  - `SaveLocationsRepo` (com constraints UNIQUE de caminho + jogo).
  - `FilesRepo`, `BackupsRepo`, `SyncQueueRepo`.
- Implementação rigorosa de `ON DELETE CASCADE` para não deixar banco de dados sujo ao remover um jogo.

### 3. `savevault-discovery` (Funcionalidade Core da v0.4.1 Concluída)
- **Steam Engine**: Lê o `libraryfolders.vdf`, itera pelas bibliotecas e lê nativamente arquivos de manifesto `.acf` para extrair nome, Steam AppID e caminhos absolutos de instalação.
- **Ludusavi Manifest Engine**: 
  - Baixa de forma assíncrona o banco de dados oficial do Ludusavi (~17MB, mais de 53 mil jogos) e realiza cache por 7 dias.
  - Parseia o YAML e extrai as regras de salvamento de cada jogo aplicáveis para Windows.
- **Path Resolver (A Estrela do Discovery)**:
  - Traduz dezenas de variáveis fictícias (ex: `<winAppData>`, `<root>`) para caminhos dinâmicos baseados no PC do usuário e diretórios de instalação de cada jogo (gerados pelo detector Steam).
  - **Suporte a Glob / <storeUserId>**: Descobre proativamente diretórios que usam o ID da Steam do usuário gerando expansões e buscando caminhos válidos no sistema de arquivos. Garante a detecção autônoma de jogos complexos (ex: *Elden Ring*, *Sekiro*).

### 4. `savevault-backup` (Fundação Completa)
- Motor de hashing implementado usando criptografia moderna **BLAKE3**.
- Criado o fluxo de lógica para testes de determinação se o arquivo de save foi modificado desde o último check, economizando IO e armazenamento.

### 5. `savevault-app` (Orquestrador)
- **`savevault`**: Binário silencioso/principal. Inicializa e coordena a DB, executa todo o discovery de ponta a ponta e salva de forma idempotente sem travar.
- **`scan_report`**: Binário visual e diagnóstico que imprime no terminal uma listagem colorida e tabular com todos os jogos, caminhos encontrados e verificação ao vivo de existência (`OK` vs `Ausente`) da pasta no sistema.

### 6. Crates Prontos para Expansão (Scaffolds com build limpo)
- `savevault-monitor`: Criado. (Alvo da v0.5)
- `savevault-sync`: Criado.
- `savevault-cloud`: Criado.

---

## Bateria de Testes Automatizados (QA)
Foram implementados 45 testes unitários e de integração, garantindo o funcionamento do núcleo do SaveVault. Atualmente, a cobertura é de **100% de aprovação (45/45 testes passando)**:

1. **Testes de Hashing (`savevault-backup`: 8 testes)**
   - O BLAKE3 lida perfeitamente com arquivos vazios, é determinístico, e reflete mudanças microscópicas com mudança de hash garantida.
2. **Testes de Regras de Negócio (`savevault-core`: 10 testes)**
   - Geração forçada de UUIDs únicos, conversão sem perdas (roundtrip) do arquivo `.toml` da configuração, e separação segura de variantes e confidence scores.
3. **Testes de Integridade da DB (`savevault-db`: 16 testes)**
   - Simulação de migrações sendo aplicadas múltiplas vezes (idempotência), exclusão de jogos causando o "delete cascade" correto dos caminhos de save, e UPSERTS funcionando corretamente para não corromper IDs.
4. **Testes do Resolver e Discovery (`savevault-discovery`: 11 testes)**
   - Testes isolados garantindo que caminhos baseados em placeholders do Linux (`<xdgData>`) são devidamente limpos, e que `<winAppData>` reflete a variável de ambiente correta.

---

## Decisões Técnicas Arquiteturais Tomadas
1. **Idempotência**: Toda vez que o programa abre, ele detecta dezenas de jogos. Foi montada uma estrutura com SQLite de `INSERT OR IGNORE` e checagem de UUIDs no Rust que assegura que o sistema apenas insere o "novo" e atualiza o resto.
2. **Separação UI/Backend**: Sem logs excessivos no binário principal `savevault` para que futuramente uma interface (Tauri, por exemplo) possa orquestrá-lo invisivelmente. A parte visual fica contida utilitários como o `scan_report`.
3. **Rust Toolchain**: Tudo alinhado para a edição "2021" do Rust.

## Próximos Passos (Próximos Milestones)
- [ ] **v0.5 - Watcher Assíncrono**: Integrar a crate `notify` dentro do `savevault-monitor` para assistir todas as pastas resgatadas do banco SQLite.
- [ ] **v0.6 - Snapshot Manager**: Interligar os eventos do Watcher com o motor de hashing para comprimir e versionar `.zip`s e mover para um cofre isolado.
- [ ] **v0.7 - Fila de Sincronização**: Realizar mock e API real para envio de blocos assíncronos para provedores Cloud.
