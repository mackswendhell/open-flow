<p align="center"><img src="docs/logo.png" width="128" alt="Open Flow"></p>
<h1 align="center">Open Flow</h1>
<p align="center"><b>Segure uma tecla, fale, solte — o texto sai pronto onde o cursor estiver.</b><br>
Ditado inteligente para Windows e macOS: transcrição + limpeza + formatação por perfil, em ~2s e R$ 0/mês.</p>
<p align="center">
  <a href="../../releases"><img src="https://img.shields.io/github/v/release/mackswendhell/open-flow" alt="release"></a>
  <img src="https://img.shields.io/badge/plataforma-Windows%2011%20%7C%20macOS-blue" alt="Windows 11 | macOS">
</p>

Segure **Ctrl+Win** (Windows) ou **Ctrl+Option** (Mac), fale, solte. O que você disse é transcrito, limpo (sem hesitações e
autocorreções da fala) e formatado no estilo do perfil ativo — e-mail, jurídico, WhatsApp,
roteiro — direto no campo onde o cursor estiver.

Exemplo real do comportamento: ditar *"Certifico que fui até o endereço, é, não, melhor,
dirigi-me ao endereço indicado no mandado... deixa eu corrigir, o imóvel pertence ao pai da
parte..."* produz *"Certifico que me dirigi ao endereço indicado no mandado. No local, fui
informado de que o imóvel pertence ao genitor da parte..."*.

<p align="center"><img src="docs/img/ui.png" width="640" alt="Janela de configurações"></p>
<p align="center"><img src="docs/img/overlay.png" width="400" alt="Overlay de ondas durante o ditado"><br>
<i>Enquanto você fala, uma onda discreta aparece na parte inferior da tela.</i></p>

## Instalação (usuários)

Não precisa saber programar. Você vai precisar de duas chaves gratuitas (5 minutos, sem cartão
de crédito) e do instalador.

**1. Baixe e instale**

*Windows:*

- Vá em [Releases](../../releases/latest) e baixe o `OpenFlow_x.y.z_x64-setup.exe` mais recente
- Execute. O Windows SmartScreen vai avisar que o app não é reconhecido (ele não tem assinatura
  digital paga): clique em **"Mais informações" → "Executar assim mesmo"**. O código-fonte
  completo está neste repositório para quem quiser auditar.
- Ao final, o Open Flow aparece como um ícone na bandeja do sistema (perto do relógio)

*macOS (Apple Silicon):*

- Vá em [Releases — macOS](../../releases/latest) e baixe o `OpenFlow_x.y.z_aarch64.dmg` mais recente
- Abra o DMG e arraste o Open Flow para **Aplicativos**
- Na primeira abertura, o Gatekeeper vai bloquear (app sem assinatura paga da Apple): vá em
  **Ajustes → Privacidade e Segurança** e clique em **"Abrir Assim Mesmo"**
- Conceda as permissões que o app pedir: **Acessibilidade** e **Monitoramento de Entrada**
  (para o atalho global) e **Microfone**. Se alguma não aparecer, adicione o Open Flow
  manualmente em Ajustes → Privacidade e Segurança
- O app fica como um ícone na barra de menus (sem ícone no Dock)

**2. Crie as duas chaves gratuitas**

| Chave | Onde criar | Para quê |
|---|---|---|
| Groq | [console.groq.com/keys](https://console.groq.com/keys) → "Create API Key" | Transcrever sua voz (~2.000 ditados grátis/dia) |
| Gemini | [aistudio.google.com/apikey](https://aistudio.google.com/apikey) → "Create API key" | Limpar e formatar o texto (free tier) |

Ambas usam login Google e não pedem cartão.

**3. Configure (uma vez)**

- Clique no ícone do Open Flow na bandeja → abre a janela de configurações
- Na aba **Geral**, cole a chave Groq no campo "Chave da API Groq" e a chave Gemini no campo
  "Chave da API Gemini" (o ícone de olho revela o que você colou)
- Escolha seu microfone, se não quiser o padrão do sistema

**4. Use**

Clique em qualquer campo de texto (e-mail, WhatsApp Web, Word...), **segure Ctrl+Win (Windows)
ou Ctrl+Option (Mac), fale, e solte**. Uma onda discreta aparece no rodapé da tela enquanto você fala; ~2 segundos depois de
soltar, o texto limpo aparece onde o cursor estava. Na aba **Perfis** você escolhe o estilo do
texto (natural, e-mail, jurídico formal, WhatsApp curto, roteiro).

Notas:
- **Privacidade**: o áudio vai para a Groq e o texto para o Google (free tiers podem usar dados
  para treino). Para transcrição 100% local/offline, veja "Modo local" abaixo — exige GPU NVIDIA
  e Python.
- O app se registra para **iniciar com o sistema** automaticamente na primeira execução
  (dá para desligar em Configurações → Geral).
- Sem a chave Gemini o texto sai bruto (sem limpeza); sem a chave Groq (e sem modo local) o
  ditado não funciona.

### Modo local (opcional, para quem tem GPU NVIDIA)

A transcrição pode rodar 100% na sua máquina (o áudio nunca sai do PC), como fallback automático
ou como provedor principal:

```powershell
git clone https://github.com/mackswendhell/open-flow
cd open-flow
python -m venv .venv
.venv\Scripts\pip install faster-whisper nvidia-cublas-cu12 nvidia-cudnn-cu12
```

Depois ajuste `python` e `sidecar` no `%APPDATA%\OpenFlow\settings.json` para os caminhos do seu
clone, e escolha o provedor "Local" nas configurações. Na primeira execução o modelo (~1,6GB) é
baixado automaticamente.

## Arquitetura

```
[Hook de teclado global (Rust: WH_KEYBOARD_LL no Windows / CGEventTap no Mac)]
        segura Ctrl+Win (Win) ou Ctrl+Option (Mac) ──> [cpal grava WAV do microfone]
        solta ↓
[Transcrição (STT)]  Groq whisper-large-v3-turbo (nuvem, padrão)
                     ⇅ fallback automático bidirecional
                     faster-whisper large-v3-turbo local (GPU, sidecar Python residente)
        texto bruto ↓
[Reescrita]  Gemini Flash (free tier) + regras fixas + dicionário + perfil de escrita ativo
        texto final ↓
[Inserção]  clipboard + Ctrl+V/Cmd+V sintético (com backup/restauração do clipboard)
```

Durante a gravação o overlay mostra as ondas; ao soltar o atalho ele passa a três pontos
("processando") e só some quando o texto entra. Se algo falhar no caminho, o motivo aparece
ali mesmo em vez de sumir no console. `Esc` durante a gravação descarta o ditado.

- **App**: Tauri 2 (Rust) + React/TypeScript — bandeja do sistema, overlay de ondas reativas
  ao microfone (2 estilos), janela de configurações com abas (Geral, Insights, Perfis,
  Dicionário, Snippets, Histórico, Diagnóstico), temas claro/escuro.
- **Sidecar STT local**: `sidecar/stt_server.py` — servidor HTTP local com faster-whisper na
  GPU; morre junto com o app (vigia o PID pai) e recusa porta duplicada.
- **Dados**: tudo local em `%APPDATA%\OpenFlow\` (Windows) ou
  `~/Library/Application Support/OpenFlow/` (Mac) — settings.json com as chaves, history.jsonl.
  Histórico é desligável e apagável pela UI. No Windows as chaves são cifradas por usuário
  (DPAPI, prefixo `enc:`); no Mac ficam em texto, protegidas pelas permissões do perfil.
- **Plataformas**: mesmo código, com a camada específica isolada em
  `app/src-tauri/src/platform/{windows,macos}.rs` (hook de teclado, proteção de chaves,
  atalho de colar, sidecar).

## Decisões de arquitetura e porquês

| Decisão | Por quê |
|---|---|
| STT híbrido nuvem+local (Groq padrão, local fallback) | Groq free tier (~2.000/dia) libera a GPU e dá boot instantâneo; o local garante offline e privacidade total quando escolhido. Fallback automático nos dois sentidos. |
| Reescrita no Gemini Flash free tier | Menor latência (~1s) e custo zero via API oficial. Sem chave, o app degrada para texto bruto corrigido pelo próprio Whisper. |
| Não usar conta ChatGPT Plus como "API" | Não existe ponte legítima Plus→API; automação do ChatGPT web viola ToS e quebra fácil. Caminhos oficiais mapeados (Codex CLI, Claude CLI) ficam como providers futuros. |
| Tauri 2 em vez de Electron | Tray + overlay com ~11MB de exe e pouca RAM residente; UI em React/TS (stack já dominada). |
| Hook de teclado de baixo nível próprio (WH_KEYBOARD_LL / CGEventTap) | Único jeito de detectar press-and-hold global (a API de hotkey comum não emite "soltou"). Suporta combos de modificadores e modo toggle. O rdev foi removido: chamava ToUnicode dentro do hook e consumia as dead keys (~/^) do teclado ABNT2. |
| Inserção via clipboard+Ctrl+V/Cmd+V com restauração | Método mais compatível; fallback natural: o texto fica no clipboard se o app alvo bloquear paste (ex.: janelas elevadas/UIPI). |
| Port macOS no mesmo repositório (`platform/` + `#[cfg]`) | Uma base de código, sem fork. Equivalências: WH_KEYBOARD_LL→CGEventTap listen-only, Ctrl+V→Cmd+V com keycode cru. Atalho padrão do Mac é Ctrl+Option (Ctrl+Cmd foi descartado: Ctrl+Cmd+Q bloqueia a tela). |
| Sem Keychain no macOS (chaves em texto no settings.json) | A autorização de leitura de uma entrada do Keychain é amarrada ao hash do binário: todo rebuild assinado pedia a senha do keychain `login`, e quando essa senha diverge da senha da conta o app fica trancado fora das próprias chaves. São chaves de API free tier num app local — o Windows segue no DPAPI, que é transparente. |
| Cancelar ditado no `Esc`, não no Espaço | O hook é listen-only por decisão de projeto, então o sistema recebe a combinação junto: com o atalho segurado, `Ctrl+Option+Espaço` (Mac) e `Win+Espaço` (Windows) trocam a fonte de entrada e mudam o layout do teclado no meio do ditado. |
| Prompt de reescrita com regras invioláveis + perfil | Regras fixas (manter só a versão final das autocorreções, não inventar fatos) valem sempre; o perfil só muda o estilo (Jurídico formal, E-mail, WhatsApp, Roteiro, Bruto). |
| Código fora do OneDrive (`C:\dev\open-flow`) | `target/` do Rust + `node_modules` em pasta sincronizada = builds lentos, conflitos de sync e locks de linker. |

## Rodando do zero

**Windows** — pré-requisitos: Windows 11, Node 18+, Rust (rustup + MSVC Build Tools), Python 3.11+.

```powershell
# 1. sidecar (STT local de fallback)
python -m venv .venv
.venv\Scripts\pip install faster-whisper nvidia-cublas-cu12 nvidia-cudnn-cu12

# 2. app
cd app
npm install
npm run tauri build -- --no-bundle
# exe em app\src-tauri\target\release\app.exe
```

**macOS** — pré-requisitos: Node 18+, Rust (rustup). O STT local na GPU é específico do
Windows/NVIDIA; no Mac o padrão é a Groq (nuvem).

```bash
cd app
npm install
npm run tauri build
# bundle em app/src-tauri/target/release/bundle/macos/OpenFlow.app (+ .dmg em bundle/dmg/)
```

Chaves (grátis, coladas na UI em Configurações → Geral):
- Groq: console.groq.com/keys (transcrição em nuvem; opcional — sem ela, 100% local)
- Gemini: aistudio.google.com/apikey (reescrita; sem ela, sai o texto bruto)

Autostart: atalho para o exe em `shell:startup`.

## História do desenvolvimento (jul/2026)

- **Fase 0 — spike**: script Python validou a meta na máquina real: STT local 0,74s para 24s
  de áudio + Gemini ~1,3s = **2,05s** total (meta era ≤3s). O exemplo jurídico de autocorreção
  de fala foi reescrito corretamente na primeira tentativa.
- **Fase 1 — núcleo**: tray, hook press-and-hold, gravação, pipeline, inserção. Testado de
  ponta a ponta com TTS sintético + injeção de teclado.
- **Fase 2 — UI e perfis**: janela de configurações completa, perfis, dicionário, snippets,
  histórico pesquisável (JSONL), Insights (WPM, streak, heatmap), diagnóstico, temas
  claro/escuro, overlay com 2 estilos de onda e ganho automático de microfone.
- **STT híbrido**: provedor configurável Groq/local com fallback bidirecional; com Groq ativo
  o modelo local nem carrega na GPU no boot.
- **Port macOS**: a camada de plataforma foi isolada em `platform/{windows,macos}.rs` e o app
  passou a rodar nativo no Mac (Apple Silicon) — CGEventTap para o atalho, Keychain para as
  chaves, Cmd+V para inserção, ícone template na barra de menus, atalho padrão Ctrl+Option.
- **Feedback e recuperação**: um grafo do repositório (graphify) mostrou que todo o valor do app
  passa por `spawn_pipeline()` — e que ali nenhum erro tinha caminho de volta até o usuário.
  Daí saíram: erros visíveis no overlay, estado "processando" até a inserção, cancelamento por
  tecla, "Copiar último ditado" na bandeja, caminhos do sidecar editáveis na UI com o
  Diagnóstico distinguindo "não configurado" de "descarregado", e busca do Histórico com espera.
  O Keychain do macOS foi removido na mesma leva (ver decisões).

Bugs memoráveis (e suas lições, gravadas como proteções no código):
- Janela extra do Tauri v2 fora de `capabilities/default.json` → eventos negados em silêncio
  (as ondas do overlay ficavam paradas).
- `settings.json` corrompido por BOM/encoding de ferramentas externas → o app passou a tolerar
  BOM e fazer backup `.bak` em vez de descartar configurações.
- Sidecars órfãos: matar o app à força deixava o Python vivo, e o `HTTPServer` do Windows
  aceitava N processos na mesma porta — 22 órfãos chegaram a consumir 15GB de RAM. Corrigido
  com vigia do PID pai + `allow_reuse_address=False`.
- Testes de hotkey por injeção de tecla (keybd_event) passaram a ser filtrados (provável
  anti-keylogger do antivírus) — validação de atalho é sempre física.
- No Mac, o overlay só aparecia com a janela de configurações aberta: janelas pertencem a um
  único Space por padrão (corrigido com `visibleOnAllWorkspaces` + collection behavior) e o
  App Nap suspendia o event tap quando não havia janela visível (corrigido com
  `NSProcessInfo beginActivityWithOptions`).
- Tecla de cancelamento no Espaço trocou o layout ABNT2 no primeiro teste: com o atalho
  segurado a combinação vira um atalho do sistema. Passou a ser `Esc`.
- O normalizador da onda do overlay não pode ser zerado a cada gravação: sem calibração
  anterior o ruído de fundo vira o máximo da escala e as ondas abrem sozinhas no silêncio.

## Pendências mapeadas

- Assinatura de código paga (eliminar o aviso do SmartScreen no Windows e o bloqueio do
  Gatekeeper no Mac) — quando a distribuição justificar

Já entregues desde a v0.1.0: instalador NSIS, autostart com toggle, idiomas de fala/saída com
tradução, dicionário com campos separados, single-instance, chaves criptografadas via DPAPI.

## Apoie

O Open Flow é gratuito e open source. Quem faz é o [Macks Wendhell](https://www.youtube.com/@mackswendhell),
do canal **Inteligência Aplicada** (conteúdos práticos sobre IA, trabalho e produtividade).
Se o app facilita seu trabalho, você pode apoiar o desenvolvimento com uma contribuição
voluntária via Pix — a chave e o QR code estão na aba **Apoie**, dentro do próprio app.
O apoio é totalmente opcional: o Open Flow continuará gratuito e aberto para todos.
