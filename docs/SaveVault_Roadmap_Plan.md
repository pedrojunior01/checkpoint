# SaveVault: Roadmap e Plano de Melhorias 🚀

Este documento serve como um mapa de ideias e planejamento estratégico do que pode ser adicionado ao SaveVault no futuro, junto com uma avaliação técnica sobre a real necessidade de cada item.

Nossa arquitetura atual (Motor Cloud-Native com 2 pastas `latest/daily`) já alcançou o **estado da arte** em otimização de disco e consumo de RAM (apenas ~15MB). A partir de agora, as melhorias devem focar em **Usabilidade**, **Segurança de Dados** e **Alcance de Plataformas**.

---

## 1. Interface Gráfica Desktop (Tauri)
Atualmente, o SaveVault roda via terminal (CLI) ou por um painel web local provisório. Para que usuários não-técnicos possam usar a ferramenta (e para lançarmos o projeto ao público geral), precisamos de um aplicativo de verdade.

- **Necessidade:** **ALTA (Prioridade Máxima)**
- **Proposta:**
  - Importar o trabalho visual já iniciado na antiga branch `cabeçaderola`.
  - Criar um ícone de bandeja (System Tray) perto do relógio do Windows, permitindo que o motor rode 100% invisível em segundo plano.
  - Painel para o usuário ligar/desligar o monitoramento de um jogo específico com 1 clique.
  - Tela visual com botão de "Restaurar Save" (listando as datas, eliminando a necessidade de usar comandos difíceis no terminal).

## 2. Motor de Escrita Atômica (Anti-Corrupção)
Hoje, se houver uma queda brusca de energia no exato milissegundo em que o SaveVault está copiando seu save para a pasta `latest`, o arquivo copiado pode ficar pela metade (corrompido).
- **Necessidade:** **MÉDIA-ALTA (Para garantir segurança grau-militar)**
- **Proposta:** 
  - Alterar o módulo `restore.rs` e `snapshot.rs` para escrever primeiro num arquivo `.tmp` invisível. 
  - Após a gravação, o sistema calcula o Hash BLAKE3 do `.tmp`. Se estiver 100% igual ao original do jogo, ele renomeia o arquivo para o formato final quase instantaneamente. 
  - Isso garante imunidade total contra corrupção.

## 3. Notificações Nativas do Windows (Toast)
Os jogadores costumam ficar ansiosos se o backup realmente ocorreu após fechar o jogo. Como o terminal ficará invisível, o aplicativo ficará "silencioso demais".
- **Necessidade:** **MÉDIA (Foco em Experiência do Usuário)**
- **Proposta:**
  - Disparar uma notificação discreta do Windows 10/11 dizendo: *"✅ Save do Elden Ring salvo em segurança"*.
  - Colocar um *debouncer* para evitar "spam" (ex: se o jogo salva 3 vezes num minuto, notificar apenas no final).

## 4. Expansão de Lojas: Epic Games e Xbox Game Pass
Embora o manifesto do `Ludusavi` já encontre muitos jogos baixados no Game Pass e na Epic Games usando apenas o nome do jogo, não temos integração "pura" e direta nos registros da Epic/Xbox como temos para Steam e GOG.
- **Necessidade:** **MÉDIA**
- **Proposta:**
  - Escrever um `epic.rs` e `xbox.rs` dentro de `crates/savevault-discovery/src/` para ler as APIs locais ou o banco de dados interno desses aplicativos instalados no Windows, garantindo 100% de taxa de detecção nessas lojas secundárias.

## 5. Gerenciamento e Limpeza Automática do Banco de Dados
A nossa estrutura de pastas (Google Drive) já se auto-limpa graças ao modelo `latest`. No entanto, o banco de dados interno local SQLite vai crescendo infinitamente com "registros" de histórico a cada vez que o jogo salva.
- **Necessidade:** **BAIXA (Para o futuro longo)**
- **Proposta:**
  - Criar uma rotina agendada (ex: a cada 30 dias) para apagar registros muito velhos de "atividades/históricos" no arquivo `savevault.db`, garantindo que ele não passe de alguns Megabytes com o passar dos anos de gameplay.

---

## ❓ Perguntas Abertas (Para sua Revisão)

> [!IMPORTANT]
> **Qual é o seu objetivo de curto prazo?**
> Você quer focar os esforços da nossa próxima sessão inteiramente em trazer a **Interface Tauri (Item 1)** para o projeto e abandonar as telas pretas do terminal de vez, ou prefere primeiro blindar a "Engine" no código aplicando a **Escrita Atômica (Item 2)**?

## 🛠 Plano de Verificação e Execução
Qualquer item que escolhermos iniciar seguirá o padrão restrito:
1. Extração isolada do recurso (seja da branch antiga ou código novo).
2. Refatoração para garantir compatibilidade com as 2 pastas Cloud.
3. Bateria completa de testes (`cargo test`) antes de confirmar a compilação do executável.
