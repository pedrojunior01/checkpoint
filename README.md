# SaveVault

SaveVault é uma solução em Rust desenvolvida para automatizar a identificação, o backup e a sincronização de arquivos de salvamento (saves) de jogos no Windows. O sistema opera em segundo plano, garantindo a proteção e o versionamento dos dados sem interromper a experiência do usuário.

## Funcionalidades Principais

- Detecção automática de jogos e plataformas (Steam, emuladores e executáveis genéricos).
- Monitoramento continuo de arquivos de save com prevenção de backups prematuros durante gravações.
- Hashing com algoritmo BLAKE3 para detecção de alterações reais nos arquivos.
- Versionamento local de snapshots com suporte a restauração segura.
- Persistência de estado e histórico utilizando banco de dados SQLite.

## Requisitos do Sistema

- Sistema Operacional: Windows 10 ou 11 (64-bit).
- Rust SDK (versão 1.80 ou superior com Cargo) instalado para compilação a partir do código-fonte.

## Instalação e Compilação

1. Clone o repositório para o seu ambiente local:

```powershell
git clone https://github.com/pedrojunior01/Savevalt.git
cd Savevalt
```

2. Realize a verificação das dependências e compilação do projeto:

```powershell
cargo build --release
```

O executável final será gerado no diretório `target/release/savevault.exe`.

## Como Usar

### Execução Direta via Cargo

Para iniciar o aplicativo em modo de desenvolvimento ou uso direto:

```powershell
cargo run -p savevault-app
```

### Execução do Binário Compilado

Após realizar o build em modo release, você pode executar o aplicativo diretamente ou configurá-lo para inicialização com o sistema:

```powershell
.\target\release\savevault.exe
```

Ao ser executado pela primeira vez, o SaveVault realizará as seguintes etapas automatizadas:

1. Criação do arquivo de configuração padrão em `%APPDATA%\SaveVault\config.toml`.
2. Inicialização do banco de dados SQLite em `%APPDATA%\SaveVault\savevault.db`.
3. Escaneamento e registro automático dos jogos instalados e seus respectivos diretórios de save.

### Configuração

As opções de retenção de versões e caminhos de armazenamento podem ser ajustadas editando o arquivo de configuração localizado em:

```text
%APPDATA%\SaveVault\config.toml
```

## Execução de Testes Automáticos

Para validar a integridade dos módulos e executar a suíte de testes de garantia de qualidade:

```powershell
cargo test --workspace
```

## Licença

Este projeto é distribuído sob a licença MIT. Para mais detalhes, consulte o arquivo LICENSE.
