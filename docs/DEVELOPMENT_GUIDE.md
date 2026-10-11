# 🛠️ Guia do Desenvolvedor & Contribuidor — SaveVault

Este guia orienta engenheiros de software e novos contribuidores sobre como configurar o ambiente de desenvolvimento, compilar o projeto em diferentes modalidades, executar a suíte de testes e estender a base de código do **SaveVault**.

---

## 1. Pré-Requisitos do Ambiente

* **Sistema Operacional:** Windows 10 (Build 19041+) ou Windows 11 x64.
* **Rust:** Versão 1.75 ou superior (instale via [rustup.rs](https://rustup.rs)).
* **Toolchain Recomendada:** `stable-x86_64-pc-windows-msvc` acompanhada das Ferramentas de Compilação do Visual Studio C++ (MSVC).
* **WebView2 Runtime:** Já instalado por padrão no Windows 10/11. Se necessário, baixe o instalador oficial da Microsoft.
* **Git:** Para versionamento.

---

## 2. Compilação e Execução

O workspace Cargo suporta compilação em múltiplos modos através de feature flags:

### 2.1. Modo Desktop com Interface Gráfica (Tauri v2)
Este é o modo padrão de produção e desenvolvimento da interface visual:
```powershell
# Compila e roda em modo debug (com logs no console)
cargo run --package savevault-app --features desktop

# Compila o binário release standalone altamente otimizado
cargo build --package savevault-app --features desktop --release
```
O executável release gerado estará em `target/release/savevault.exe`.

### 2.2. Modo Headless / Daemon Residente (CLI)
Para servidores, testes em CI ou terminais sem dependência gráfica do WebView2:
```powershell
# Compila apenas a versão terminal
cargo build --package savevault-app

# Executa o daemon contínuo em background no terminal
cargo run --package savevault-app
```

### 2.3. Executáveis Utilitários e Diagnósticos
* **Relatório de Varredura Steam/GOG (`scan_report`):**
  Varre os jogos instalados na máquina, detecta saves e exibe um relatório com legendas e status:
  ```powershell
  cargo run --package savevault-app --bin scan_report
  ```

---

## 3. Execução de Testes Automatizados

O SaveVault possui **100% de cobertura nos fluxos críticos** com mais de 110 testes automatizados:

### 3.1. Rodar toda a suíte do workspace
```powershell
cargo test --workspace
```

### 3.2. Rodar testes de um crate específico
```powershell
# Testes do motor de monitoramento e watcher
cargo test --package savevault-monitor

# Testes de persistência e migrações SQLite
cargo test --package savevault-db

# Testes da camada de nuvem e OneDrive
cargo test --package savevault-cloud

# Testes de orquestração e interface
cargo test --package savevault-app
```

### 3.3. Rodar testes exibindo a saída padrão (`stdout`)
```powershell
cargo test --package savevault-monitor -- --nocapture
```

---

## 4. Como Adicionar um Novo Detector de Jogos

Para dar suporte a um novo launcher (ex: Epic Games Store, Ubisoft Connect ou EA App), implemente a trait `SaveDetector` no crate `savevault-discovery`:

1. Crie um novo arquivo em `crates/savevault-discovery/src/epic.rs`.
2. Implemente a trait `SaveDetector`:
```rust
use crate::save_detector::{SaveDetector, SaveDiscoveryResult, SaveDiscoveryError};
use savevault_core::models::{Game, SaveLocation, SaveLocationType};

pub struct EpicGamesDetector;

impl SaveDetector for EpicGamesDetector {
    fn name(&self) -> &'static str {
        "Epic Games Launcher"
    }

    fn detect_save_locations(&self, game: &Game) -> Result<SaveDiscoveryResult, SaveDiscoveryError> {
        // 1. Identificar caminho de instalação através de manifestos .item ou registro
        // 2. Localizar arquivos de save correspondentes
        // 3. Retornar SaveDiscoveryResult::Found(vec![...])
    }
}
```
3. Registre o detector na função `run_all` em `crates/savevault-discovery/src/lib.rs`.
4. Adicione testes unitários em `crates/savevault-discovery/src/tests.rs`.

---

## 5. Como Adicionar um Novo Provedor de Nuvem

Para integrar um novo provedor de nuvem (ex: Dropbox API, MEGA, WebDAV ou AWS S3):

1. Crie o módulo em `crates/savevault-cloud/src/novo_provedor.rs`.
2. Implemente a trait assíncrona `CloudProvider`:
```rust
use async_trait::async_trait;
use crate::{CloudProvider, CloudResult, CloudCatalogManifest};
use std::path::Path;

pub struct NovoProvedor {
    // Configurações, tokens ou chaves
}

#[async_trait]
impl CloudProvider for NovoProvedor {
    async fn upload(&self, local_path: &Path, cloud_path: &str) -> CloudResult<String> {
        // Envia o arquivo e retorna o remote_file_id opaco
    }

    async fn download(&self, cloud_file_id: &str, dest_path: &Path) -> CloudResult<()> {
        // Baixa o arquivo do identificador remoto
    }

    async fn delete(&self, cloud_file_id: &str) -> CloudResult<()> {
        // Deleta o snapshot na nuvem
    }

    async fn get_manifest(&self) -> CloudResult<Option<CloudCatalogManifest>> {
        // Recupera o savevault_manifest.json da nuvem
    }

    async fn save_manifest(&self, manifest: &CloudCatalogManifest) -> CloudResult<()> {
        // Persiste o savevault_manifest.json na nuvem
    }
}
```
3. Adicione o provedor na enum e na função de resolução `resolve_active_cloud_provider` em `crates/savevault-sync/src/lib.rs`.
4. Adicione testes unitários e de integração mockados.

---

## 6. Padrões de Código e Boas Práticas

### 6.1. Manipulação de Caminhos no Windows
* **Nunca utilize concatenação simples de strings** (`format!("{}/{}", a, b)`). Sempre use `Path::join` ou `PathBuf`.
* **Nunca confie que arquivos existem no disco** durante o discovery; utilize `savevault_core::path::normalize_windows_path_str` para normalização puramente lógica.
* **Trate sempre case-insensitivity** no Windows usando `canonicalize_windows_path_str` ou `COLLATE NOCASE` no SQLite.

### 6.2. Tratamento de Erros
* Em bibliotecas (`core`, `db`, `monitor`, `backup`), prefira tipos de erro fortemente tipados com `thiserror`.
* Na camada de orquestração (`savevault-app`), utilize `anyhow::Result` para enriquecer contextos de erro com `.context("...")`.

### 6.3. Logs e Diagnóstico com Tracing
* O projeto utiliza a crate `tracing`.
* Níveis recomendados:
  * `trace!`: Detalhes finos de bytes e iterações de loop.
  * `debug!`: Caminhos avaliados, cheques de debounce e consultas SQLite.
  * `info!`: Inicialização de serviços, novo snapshot detectado e uploads concluídos.
  * `warn!`: Arquivos temporariamente bloqueados por jogos (`ERROR_SHARING_VIOLATION`) ou retries de rede.
  * `error!`: Falhas não recuperáveis que exigem atenção imediata.
* Para habilitar logs detalhados durante o desenvolvimento no PowerShell:
  ```powershell
  $env:SAVEVAULT_LOG="debug"
  cargo run --package savevault-app --features desktop
  ```

