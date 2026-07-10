# Open Flow

App Windows de ditado inteligente, inspirado no Wispr Flow. Segure **Ctrl+Win**, fale, solte —
o texto sai transcrito, limpo (sem hesitações e autocorreções da fala) e formatado no estilo do
perfil ativo, inserido direto no campo onde o cursor estiver. Latência medida: **~2s** do soltar
a tecla ao texto pronto. Custo de operação: **R$ 0/mês**.

Exemplo real do comportamento: ditar *"Certifico que fui até o endereço, é, não, melhor,
dirigi-me ao endereço indicado no mandado... deixa eu corrigir, o imóvel pertence ao pai da
parte..."* produz *"Certifico que me dirigi ao endereço indicado no mandado. No local, fui
informado de que o imóvel pertence ao genitor da parte..."*.

## Arquitetura

```
[Hook de teclado global (Rust/rdev)] ── segura Ctrl+Win ──> [cpal grava WAV do microfone]
        solta ↓
[Transcrição (STT)]  Groq whisper-large-v3-turbo (nuvem, padrão)
                     ⇅ fallback automático bidirecional
                     faster-whisper large-v3-turbo local (GPU, sidecar Python residente)
        texto bruto ↓
[Reescrita]  Gemini Flash (free tier) + regras fixas + dicionário + perfil de escrita ativo
        texto final ↓
[Inserção]  clipboard + Ctrl+V sintético (com backup/restauração do clipboard)
```

- **App**: Tauri 2 (Rust) + React/TypeScript — bandeja do sistema, overlay de ondas reativas
  ao microfone (2 estilos), janela de configurações com abas (Geral, Insights, Perfis,
  Dicionário, Snippets, Histórico, Diagnóstico), temas claro/escuro.
- **Sidecar STT local**: `sidecar/stt_server.py` — servidor HTTP local com faster-whisper na
  GPU; morre junto com o app (vigia o PID pai) e recusa porta duplicada.
- **Dados**: tudo local em `%APPDATA%\OpenFlow\` (settings.json com as chaves, history.jsonl).
  Histórico é desligável e apagável pela UI.

## Decisões de arquitetura e porquês

| Decisão | Por quê |
|---|---|
| STT híbrido nuvem+local (Groq padrão, local fallback) | Groq free tier (~2.000/dia) libera a GPU e dá boot instantâneo; o local garante offline e privacidade total quando escolhido. Fallback automático nos dois sentidos. |
| Reescrita no Gemini Flash free tier | Menor latência (~1s) e custo zero via API oficial. Sem chave, o app degrada para texto bruto corrigido pelo próprio Whisper. |
| Não usar conta ChatGPT Plus como "API" | Não existe ponte legítima Plus→API; automação do ChatGPT web viola ToS e quebra fácil. Caminhos oficiais mapeados (Codex CLI, Claude CLI) ficam como providers futuros. |
| Tauri 2 em vez de Electron | Tray + overlay com ~11MB de exe e pouca RAM residente; UI em React/TS (stack já dominada). |
| Hook de teclado de baixo nível (rdev) | Único jeito de detectar press-and-hold global (a API de hotkey comum não emite "soltou"). Suporta combos de modificadores (Ctrl+Win) e modo toggle. |
| Inserção via clipboard+Ctrl+V com restauração | Método mais compatível do Windows; fallback natural: o texto fica no clipboard se o app alvo bloquear paste (ex.: janelas elevadas/UIPI). |
| Prompt de reescrita com regras invioláveis + perfil | Regras fixas (manter só a versão final das autocorreções, não inventar fatos) valem sempre; o perfil só muda o estilo (Jurídico formal, E-mail, WhatsApp, Roteiro, Bruto). |
| Código fora do OneDrive (`C:\dev\open-flow`) | `target/` do Rust + `node_modules` em pasta sincronizada = builds lentos, conflitos de sync e locks de linker. |

## Rodando do zero

Pré-requisitos: Windows 11, Node 18+, Rust (rustup + MSVC Build Tools), Python 3.11+.

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

## Pendências mapeadas

- Prévia editável antes de inserir (modo revisão)
- Providers de reescrita alternativos: Claude CLI (assinatura), Codex CLI (ChatGPT Plus)
- Instalador NSIS + auto-update; empacotar o sidecar sem depender do Python local
- Chave Gemini/Groq via DPAPI em vez de texto no settings.json
- Regras por aplicativo, comandos de voz, single-instance
