# Open Flow — instruções do projeto

Ditado inteligente para Windows e macOS: segura o atalho, fala, solta — o texto transcrito,
limpo e formatado entra onde o cursor estiver. Tauri 2 (Rust) + React/TS, com sidecar Python
opcional para STT local na GPU.

Arquitetura, decisões e histórico de bugs estão no [README.md](README.md) — não duplicar aqui.
Este arquivo é **versionado** e vale nas duas máquinas. O que é específico de uma delas vai em
`CLAUDE.local.md`, que fica fora do git.

## Duas máquinas, um repositório

O Macks trabalha alternando entre um **Windows** e um **macOS**, e o build de cada sistema só
sai na máquina dele. Isso é a origem da maior parte da confusão histórica do projeto, então:

- **Antes de qualquer coisa, `git fetch` e ver se a outra máquina deixou commit para trás.** É
  comum começar uma sessão uma release atrás.
- **Base de código única.** O código específico de SO vive em
  `app/src-tauri/src/platform/{windows,macos}.rs`, atrás de `#[cfg]`, com a mesma interface
  pública (ver `platform/mod.rs`). Mexeu em um, confira o par — o outro SO não compila aqui
  para avisar.
- **Uma correção em `lib.rs` vale para os dois sistemas.** Depois de lançar de um lado, o outro
  fica sem a correção até alguém buildar lá. Registrar isso em "Estado atual".
- **Ao fechar uma release, atualizar a seção "Estado atual" abaixo.** É por ela que o agente da
  outra máquina descobre o que ficou pendente do lado dele.

## Estado atual (2026-08-21)

- **Versão**: v0.1.15 no Windows, v0.1.14 no macOS. As correções da v0.1.15 são de `lib.rs`,
  comuns aos dois SOs — **buildar no Mac e anexar o DMG à tag v0.1.15**, que hoje só tem os
  instaladores Windows. O mesmo vale para o `.dmg` da v0.1.14, que já existe.
- **v0.1.15 trouxe**: (1) o overlay volta ao topo — `set_always_on_top(true)` sozinho não fazia
  nada, porque o tao só chama `SetWindowPos` quando a flag muda de valor e a janela já nasce
  `alwaysOnTop`; a correção é alternar `false`/`true`. (2) O fix do fallback do Gemini vindo do
  Mac (9b9ce95): só 404 rebaixa a sessão, reserva virou `gemini-flash-lite-latest`, teto por
  chamada de 30s para 4s com uma segunda tentativa.
- **Pendente de confirmação em uso**: a assinatura do Whisper aparecia em ~3% dos ditados, então
  a prova de que o `strip_credit` resolveu vem com volume, não com sessão de teste.
- **Modelo do Gemini vive no `settings.json`, não no default.** Trocar o default só afeta
  settings novo: em cada máquina a chave `gemini_model` precisa ser trocada à mão, ou o app
  segue no modelo antigo. Toda troca de modelo futura precisa desse passo nos dois lados.
- **Fonte da verdade da versão**: `app/src-tauri/tauri.conf.json` (o `version` do
  `app/package.json` está em 0.1.0 e é ignorado — não é ele que nomeia os instaladores).

## Release

Versão nova: subir `version` em `tauri.conf.json` **e** em `Cargo.toml`, buildar, commit
incluindo o `Cargo.lock`, `git tag` e `gh release create` com os instaladores que a máquina
gerou. A outra máquina depois anexa o instalador dela **à tag existente**, com
`gh release upload` — sem criar release nova.

**NUNCA APONTAR OS LINKS DE DOWNLOAD DO README PARA `/releases/latest` NEM PARA UMA TAG
ESPECÍFICA.** Os dois links de instalador do README vão para `../../releases` — a lista inteira
— e ficam assim. `/releases/latest` é UMA release só, a do último build feito: depois de um
build Windows ela não tem `.dmg`, depois de um build Mac não tem `.exe`. Foi isso que cruzou os
links a cada release e fez cada máquina "consertar" apontando para a sua própria tag, quebrando
o outro lado. **NÃO EXISTE URL DO GITHUB QUE RESOLVA PARA "a release mais recente que tem
.dmg"** — o único link que nunca mente é a lista. Só volte a usar `/latest` se toda release
passar a sair com os dois instaladores juntos.

## Ao trabalhar no código

- **`spawn_pipeline()` em `lib.rs` é o caminho de todo o valor do app** — gravação, STT,
  reescrita, inserção. Erro que não chega ao overlay por ali é erro que o usuário nunca vê.
- **Nunca voltar a esconder a janela do overlay.** Ela nasce `visible: true` e fica aberta para
  sempre; `overlay_hide` só emite evento e quem apaga o conteúdo é o React (estado `idle`).
  Chamar `hide()`/`show()` nessa janela é o que fazia o overlay sumir no Windows depois de um
  tempo de uso — a janela reabria certinha e a webview não pintava mais. Duas correções falharam
  antes por atacar "a janela não aparece" em vez de "a janela aparece e não pinta"; o que
  resolveu a dúvida foi medir a janela pelo Win32 durante um ditado real.
- **Mudou o prompt em `RULES`?** O `history.jsonl` é a suíte de regressão: os `raw` de ditados
  reais já gravados rodam no prompt velho e no novo sem precisar ditar de novo. Foi assim que
  se descobriu que "quebre onde o foco muda" falhava no texto de 943 caracteres. Script de
  referência em `scratchpad/ab_regra4.py` (reproduz `build_prompt()` lendo o próprio `lib.rs`);
  `bench/bench_rewrite.py` faz o mesmo para comparar modelo e timeout contra a API.
- **Numeração das regras do prompt**: a regra de grafias do dicionário é injetada como "6." em
  `build_prompt()`. Ao acrescentar regra nova em `RULES`, conferir a colisão.
- **O sidecar não sobe no boot com a Groq ativa e configurada.** A condição em `run()` é
  `stt_provider != "groq" || !groq_ready()` — ele também sobe se a Groq estiver selecionada mas
  sem chave válida. Com a Groq funcionando, o modelo nem carrega na GPU; não estranhar a
  ausência do processo Python.
- **Testar atalho global é sempre físico.** Injeção de tecla por API é filtrada pelo sistema e
  dá falso negativo.
- **Chaves de API nunca entram em commit, issue ou log.** Elas vivem no `settings.json` do
  usuário, fora do repositório. Encoding do arquivo é UTF-8: ferramentas que gravam BOM já o
  quebraram antes; o app tolera e faz `.bak`, mas evitar reescrevê-lo por fora.

## Pendências conhecidas

- O comentário no topo de `platform/mod.rs` ainda diz `secrets: DPAPI (Windows) ⇄ Keychain
  (macOS)`, mas o Keychain foi removido do macOS na v0.1.7 (as chaves ficam em texto lá).
  Comentário desatualizado, não o código.
- Assinatura de código paga, para eliminar o aviso do SmartScreen no Windows e o bloqueio do
  Gatekeeper no Mac — quando a distribuição justificar.

@CLAUDE.local.md
