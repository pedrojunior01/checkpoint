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

## Tutorial R�pido de Uso

O SaveVault agora utiliza um motor de versionamento inteligente (latest e daily) voltado exclusivamente para sincroniza��o nativa na nuvem (como o Google Drive).

### Passo a Passo:
1. **Configura��o do Google Drive:**
   - Instale o aplicativo oficial do **Google Drive Desktop** (ou OneDrive/Dropbox).
   - O SaveVault salva os arquivos de backup localmente na sua pasta definida em config.toml (por padr�o: C:\Users\SEU_USUARIO\Google Drive\SaveVault_Backups).

2. **Iniciando o Monitoramento:**
   - Abra um terminal e digite o comando abaixo para rodar o motor em segundo plano:
     \\\ash
     cargo run -p savevault-app
     \\\
   - O aplicativo far� uma varredura autom�tica (via Manifest Oficial e Heur�stica) para descobrir todos os seus jogos e, em seguida, entrar� em modo **Watcher**.

3. **Iniciando o Dashboard (Interface Visual):**
   - Para monitorar os saves atrav�s de um painel web no navegador, execute:
     \\\ash
     cargo run -p savevault-app -- ui
     \\\
   - Acesse http://127.0.0.1:4040 (ou a porta informada no terminal) para visualizar a lista de jogos protegidos, status do sistema e os �ltimos snapshots realizados.

4. **Como funcionam os Saves (O motor de 2 pastas):**
   - Quando voc� joga e o jogo realiza um save (auto-save ou save manual), o SaveVault detecta a modifica��o no arquivo.
   - Ele far� um backup para a pasta latest (sobrescrevendo o anterior) e, se for o primeiro save do dia, far� uma c�pia de seguran�a para a pasta daily.
   - **Exemplo:** Google Drive/SaveVault_Backups/ELDEN RING/latest/ER0000.sl2
   - Essa estrutura latest/daily n�o consome seu HD infinitamente, � extremamente leve (15MB de RAM), e usa o motor de versionamento e deduplica��o nativo da pr�pria nuvem (voc� pode restaurar saves antigos pelo hist�rico de vers�es no site do Google Drive).

5. **Restaurando um Save Manualmente pelo SaveVault:**
   - Se preferir restaurar o �ltimo save validado pelo SaveVault, voc� pode usar a CLI:
     \\\ash
     cargo run -p savevault-app -- restore list
     cargo run -p savevault-app -- restore <id_do_snapshot>
     \\\

