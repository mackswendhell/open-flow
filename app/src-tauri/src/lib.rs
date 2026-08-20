use std::collections::HashSet;
use std::io::Cursor;
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::{Deserialize, Serialize};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition};

mod platform;

const RULES: &str = "Você é um editor de ditado. Receberá a transcrição literal de uma fala em pt-BR.\n\
Invioláveis: NÃO adicione fatos, NÃO resuma, NÃO omita conteúdo.\n\
Edição — padrão conservador; o Estilo no fim pode pedir mais liberdade:\n\
1. Preserve as palavras do falante. Não troque expressões por sinônimos mais formais, não reordene nem reescreva frases que já se entendem. Corrija apenas ortografia, concordância e pontuação.\n\
2. Repetição de palavra ou frase é ênfase do falante: MANTENHA. Remova só hesitações e falsos inícios (\"é...\", \"hum\", \"a-a-a\").\n\
3. Descarte a versão anterior apenas quando o falante se retrata de forma explícita (\"não, na verdade é X\", \"me corrigindo, são 30\"). Conectores como \"quer dizer\", \"tipo\" e \"então\" sozinhos NÃO são correção.\n\
4. Estrutura: quando o falante marca a virada em voz alta, obedeça — é instrução, não licença para reformatar. \"outra coisa\", \"outro assunto\", \"mudando de assunto\", \"agora\" abrindo tema novo: parágrafo novo (linha em branco). \"primeiro/segundo/terceiro\", \"três coisas que eu queria\": uma linha por item, numerada. Isso vale igual em fala curta. Sem marca falada, entregue um bloco só: não force formato nem crie tópicos por conta própria.\n\
5. Fala longa (acima de ~400 caracteres) nunca sai em bloco único, mesmo que trate de um assunto só: divida em parágrafos de no máximo 4 frases, cortando no ponto final mais próximo da virada de foco.\n";

#[derive(Serialize, Deserialize, Clone, Default)]
struct Profile {
    name: String,
    style: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct Snippet {
    trigger: String,
    text: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Settings {
    hotkey: String,
    cancel_key: String,
    mode: String,
    mic: Option<String>,
    stt_provider: String,
    groq_api_key: Option<String>,
    groq_model: String,
    speech_lang: String,
    output_lang: String,
    gemini_model: String,
    gemini_api_key: Option<String>,
    wave_style: String,
    theme: String,
    profiles: Vec<Profile>,
    active_profile: String,
    dictionary: Vec<String>,
    snippets: Vec<Snippet>,
    history_enabled: bool,
    python: String,
    sidecar: String,
    stt_port: u16,
}

fn default_profiles() -> Vec<Profile> {
    let p = |name: &str, style: &str| Profile { name: name.into(), style: style.into() };
    vec![
        p("Bruto corrigido", "coringa do dia a dia (resposta de conversa, WhatsApp, prompt para IA): texto natural com o tom e as palavras do falante preservados."),
        p("Jurídico formal", "certidão ou comunicação de oficial de justiça, linguagem jurídica formal brasileira (ex.: \"Certifico e dou fé que...\", \"genitor\", \"Ante o exposto, devolvo o presente mandado\")."),
        p("E-mail profissional", "e-mail profissional claro e cordial, com saudação e fecho quando fizer sentido. Aqui pode reescrever para deixar formal: cada ideia em sua própria linha, separadas por linha em branco, no padrão corporativo."),
        p("WhatsApp curto", "mensagem curta e informal de WhatsApp, direta, sem formalidade excessiva."),
        p("Roteiro de vídeo", "roteiro falado para vídeo do YouTube: frases curtas, ritmo de fala natural, tom de conversa."),
    ]
}

#[cfg(windows)]
fn default_hotkey() -> String {
    "Ctrl+Win".into()
}

// Ctrl+Option: a combinação mais "vazia" do macOS. Ctrl+Cmd foi descartado
// porque letra acidental durante o hold dispara combos do sistema
// (Ctrl+Cmd+Q trava a tela).
#[cfg(target_os = "macos")]
fn default_hotkey() -> String {
    "Ctrl+Option".into()
}

#[cfg(windows)]
fn default_sidecar_paths() -> (String, String) {
    (
        r"C:\dev\open-flow\.venv\Scripts\python.exe".into(),
        r"C:\dev\open-flow\sidecar\stt_server.py".into(),
    )
}

#[cfg(target_os = "macos")]
fn default_sidecar_paths() -> (String, String) {
    let home = dirs::home_dir().unwrap_or_default();
    (
        home.join("dev/open-flow/.venv/bin/python").to_string_lossy().into_owned(),
        home.join("dev/open-flow/sidecar/stt_server.py").to_string_lossy().into_owned(),
    )
}

impl Default for Settings {
    fn default() -> Self {
        let (python, sidecar) = default_sidecar_paths();
        Settings {
            hotkey: default_hotkey(),
            // Esc e não Espaço: com o atalho segurado, Ctrl+Option+Espaço (mac) e
            // Win+Espaço (Windows) são atalhos do sistema que trocam a fonte de
            // entrada — o hook é listen-only, então o sistema recebe a combinação
            // junto e o layout do teclado muda no meio do ditado.
            cancel_key: "ESC".into(),
            mode: "hold".into(),
            mic: None,
            stt_provider: "groq".into(),
            groq_api_key: None,
            groq_model: "whisper-large-v3-turbo".into(),
            speech_lang: "pt".into(),
            output_lang: "same".into(),
            gemini_model: "gemini-3.5-flash-lite".into(),
            gemini_api_key: None,
            wave_style: "tech".into(),
            theme: "dark".into(),
            profiles: default_profiles(),
            active_profile: "Bruto corrigido".into(),
            dictionary: Vec::new(),
            snippets: Vec::new(),
            history_enabled: true,
            // ponytail: caminhos desta máquina; viram recurso empacotado na fase 3
            python,
            sidecar,
            stt_port: 17765,
        }
    }
}

struct Shared {
    settings: Mutex<Settings>,
    groups: Mutex<Vec<Vec<u32>>>,
    cancel: Mutex<Option<u32>>,
    sidecar: Mutex<Option<std::process::Child>>,
    insert_lock: Mutex<()>,
    /// último texto inserido: a colagem pode falhar sem erro (janela sem foco,
    /// campo não editável) e o clipboard volta ao valor antigo 300ms depois —
    /// sem esta cópia o ditado some sem deixar rastro
    last_text: Mutex<String>,
    /// cada exibição do overlay ganha um número; quem termina tarde só mexe na
    /// janela se ainda for o dono dela (ditar de novo enquanto o anterior processa)
    overlay_gen: AtomicU64,
}

impl Settings {
    fn groq_ready(&self) -> bool {
        self.groq_api_key.as_deref().map(|k| !k.is_empty()).unwrap_or(false)
    }
}

fn config_dir() -> std::path::PathBuf {
    dirs::config_dir().unwrap().join("OpenFlow")
}

fn settings_path() -> std::path::PathBuf {
    config_dir().join("settings.json")
}

// --- Segredos: DPAPI (Windows) ⇄ Keychain (macOS); no disco fica "enc:..." ---
const ENC_PREFIX: &str = "enc:";

/// `name` identifica o item no cofre da plataforma (o Keychain precisa; o DPAPI ignora)
fn decrypt_key(name: &str, v: &Option<String>) -> Option<String> {
    v.as_ref()
        .map(|s| match s.strip_prefix(ENC_PREFIX) {
            Some(stored) => platform::unprotect_secret(name, stored).unwrap_or_default(),
            None => s.clone(),
        })
        .filter(|s| !s.is_empty())
}

fn encrypt_key(name: &str, v: &Option<String>) -> Option<String> {
    v.as_ref().filter(|s| !s.is_empty()).map(|s| {
        if s.starts_with(ENC_PREFIX) {
            s.clone()
        } else {
            platform::protect_secret(name, s)
                .map(|b| format!("{ENC_PREFIX}{b}"))
                .unwrap_or_else(|| s.clone())
        }
    })
}

/// grava no disco com as chaves criptografadas (em memória ficam em texto)
fn persist_settings(s: &Settings) -> Result<(), String> {
    let mut disk = s.clone();
    disk.gemini_api_key = encrypt_key("gemini_api_key", &disk.gemini_api_key);
    disk.groq_api_key = encrypt_key("groq_api_key", &disk.groq_api_key);
    let path = settings_path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, serde_json::to_string_pretty(&disk).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn history_path() -> std::path::PathBuf {
    config_dir().join("history.jsonl")
}

/// retorna (settings, primeira_execucao)
fn load_settings() -> (Settings, bool) {
    let path = settings_path();
    if let Ok(txt) = std::fs::read_to_string(&path) {
        // tolera BOM que editores/PowerShell inserem
        match serde_json::from_str::<Settings>(txt.trim_start_matches('\u{feff}')) {
            Ok(mut s) => {
                if s.profiles.is_empty() {
                    s.profiles = default_profiles();
                }
                s.gemini_api_key = decrypt_key("gemini_api_key", &s.gemini_api_key);
                s.groq_api_key = decrypt_key("groq_api_key", &s.groq_api_key);
                return (s, false);
            }
            Err(e) => {
                // nunca descartar o arquivo do usuário em silêncio
                eprintln!("[settings] arquivo ilegível ({e}); backup em settings.json.bak");
                let _ = std::fs::copy(&path, path.with_extension("json.bak"));
            }
        }
    }
    let s = Settings::default();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::write(&path, serde_json::to_string_pretty(&s).unwrap());
    (s, true)
}

/// keycodes por plataforma (VK no Windows, CGKeyCode no mac) vêm de platform::
fn parse_hotkey(s: &str) -> Vec<Vec<u32>> {
    s.split('+')
        .map(|t| {
            let name = t.trim().to_uppercase();
            platform::modifier_group(&name)
                .unwrap_or_else(|| vec![platform::key_from_name(&name)])
        })
        .collect()
}

/// vazio = cancelamento desligado
fn parse_cancel_key(s: &str) -> Option<u32> {
    let name = s.trim().to_uppercase();
    if name.is_empty() {
        return None;
    }
    Some(platform::key_from_name(&name))
}

type LevelFn = Box<dyn FnMut(f32) + Send>;

struct Recorder {
    stream: Option<cpal::Stream>,
    buf: Arc<Mutex<Vec<f32>>>,
    rate: u32,
    channels: u16,
}

impl Recorder {
    fn new() -> Self {
        Recorder { stream: None, buf: Arc::new(Mutex::new(Vec::new())), rate: 16000, channels: 1 }
    }

    fn start(&mut self, mic: Option<&str>, mut on_level: LevelFn) -> Result<(), String> {
        let host = cpal::default_host();
        let device = match mic {
            Some(name) => host
                .input_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().map(|n| n.contains(name)).unwrap_or(false))
                .ok_or_else(|| format!("microfone '{name}' não encontrado"))?,
            None => host.default_input_device().ok_or("sem microfone padrão")?,
        };
        let config = device.default_input_config().map_err(|e| e.to_string())?;
        self.rate = config.sample_rate().0;
        self.channels = config.channels();
        self.buf.lock().unwrap().clear();
        let buf = self.buf.clone();
        let err_fn = |e| eprintln!("[audio] erro no stream: {e}");
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device
                .build_input_stream(
                    &config.into(),
                    move |data: &[f32], _: &_| {
                        let rms = (data.iter().map(|s| s * s).sum::<f32>()
                            / data.len().max(1) as f32)
                            .sqrt();
                        on_level(rms);
                        buf.lock().unwrap().extend_from_slice(data);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| e.to_string())?,
            cpal::SampleFormat::I16 => device
                .build_input_stream(
                    &config.into(),
                    move |data: &[i16], _: &_| {
                        let mut b = buf.lock().unwrap();
                        let start = b.len();
                        b.extend(data.iter().map(|s| *s as f32 / 32768.0));
                        let chunk = &b[start..];
                        let rms = (chunk.iter().map(|s| s * s).sum::<f32>()
                            / chunk.len().max(1) as f32)
                            .sqrt();
                        drop(b);
                        on_level(rms);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| e.to_string())?,
            f => return Err(format!("formato de sample não suportado: {f}")),
        };
        stream.play().map_err(|e| e.to_string())?;
        self.stream = Some(stream);
        Ok(())
    }

    fn stop(&mut self) -> (Vec<f32>, u32) {
        self.stream = None;
        let raw = std::mem::take(&mut *self.buf.lock().unwrap());
        let ch = self.channels as usize;
        let mono: Vec<f32> = if ch > 1 { raw.chunks(ch).map(|c| c[0]).collect() } else { raw };
        (mono, self.rate)
    }
}

/// maior RMS em janelas de 100ms — proxy de "houve fala?"
fn peak_rms(samples: &[f32], rate: u32) -> f32 {
    let win = (rate as usize / 10).max(1);
    samples
        .chunks(win)
        .map(|c| (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt())
        .fold(0.0f32, f32::max)
}

/// alucinações típicas do Whisper em áudio sem fala
fn is_hallucination(text: &str, audio_secs: f32) -> bool {
    if audio_secs >= 2.0 {
        return false;
    }
    let norm: String = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    let norm = norm.trim();
    // sem frase anterior para servir de âncora, a assinatura de legenda vem
    // sozinha e o strip_credit não a alcança (histórico: 0,8s de áudio)
    norm.starts_with("legenda")
        || matches!(norm, "e aí" | "eaí" | "obrigado" | "obrigada" | "tchau" | "valeu" | "thank you")
}

fn wav_bytes(samples: &[f32], rate: u32) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut cursor, spec).unwrap();
        for s in samples {
            w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
        }
        w.finalize().unwrap();
    }
    cursor.into_inner()
}

fn sidecar_healthy(port: u16) -> bool {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()
        .and_then(|c| c.get(format!("http://127.0.0.1:{port}/health")).send().ok())
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

fn ensure_sidecar(shared: &Shared, s: &Settings) -> Result<(), String> {
    if sidecar_healthy(s.stt_port) {
        return Ok(());
    }
    {
        let mut guard = shared.sidecar.lock().unwrap();
        if sidecar_healthy(s.stt_port) {
            return Ok(());
        }
        if let Some(c) = guard.as_mut() {
            let _ = c.kill();
        }
        *guard = platform::spawn_sidecar(s);
        if guard.is_none() {
            return Err("não foi possível iniciar a transcrição local".into());
        }
    }
    println!("[stt] aquecendo transcrição local (primeira vez leva ~30s)...");
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        if sidecar_healthy(s.stt_port) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    Err("transcrição local não ficou pronta em 120s".into())
}

fn transcribe_groq(wav: Vec<u8>, s: &Settings) -> Result<String, String> {
    let key = s.groq_api_key.clone().filter(|k| !k.is_empty()).ok_or("chave Groq ausente")?;
    let part = reqwest::blocking::multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;
    let mut form = reqwest::blocking::multipart::Form::new()
        .part("file", part)
        .text("model", s.groq_model.clone())
        .text("response_format", "json");
    if s.speech_lang != "auto" {
        form = form.text("language", s.speech_lang.clone());
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post("https://api.groq.com/openai/v1/audio/transcriptions")
        .bearer_auth(key)
        .multipart(form)
        .send()
        .map_err(|e| format!("Groq: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Groq HTTP {}: {}", resp.status(), resp.text().unwrap_or_default()));
    }
    let v: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
    Ok(v["text"].as_str().unwrap_or("").trim().to_string())
}

/// O Whisper foi treinado em legendas e às vezes assina o fim da transcrição:
/// "Legenda Adriana Zanotto", "Legendas pela comunidade Amara.org". Não dá para
/// resolver no prompt — as regras mandam NÃO omitir conteúdo, então o Gemini
/// preserva a assinatura fielmente e ela cai no campo do usuário. 8 dos 282
/// ditados do histórico vieram assim, sempre coladas depois da última frase.
///
/// Só corta a cauda depois do último ponto final: "legenda" no meio da fala é
/// palavra legítima do falante (aparece no histórico em "com título, legenda,
/// nome das variáveis") e não pode sumir.
fn strip_credit(text: &str) -> &str {
    const MARCAS: [&str; 3] = ["legenda", "legendado", "subtitle"];
    let t = text.trim_end();
    let corpo = t.trim_end_matches(['.', '!', '?', ' ']);
    // fim de frase é pontuação seguida de espaço: o ponto de "Amara.org" não conta
    let Some(i) = corpo
        .char_indices()
        .filter(|(j, c)| {
            matches!(c, '.' | '!' | '?' | '\n')
                && corpo[j + c.len_utf8()..].chars().next().map_or(true, char::is_whitespace)
        })
        .map(|(j, _)| j)
        .next_back()
    else {
        return t;
    };
    let cauda = corpo[i + 1..].trim().to_lowercase();
    let curta = cauda.split_whitespace().count() <= 5;
    if curta && (MARCAS.iter().any(|m| cauda.starts_with(m)) || cauda.contains("amara.org")) {
        return t[..=i].trim_end();
    }
    t
}

fn run_stt(wav: Vec<u8>, shared: &Shared, s: &Settings) -> Result<String, String> {
    // os dois provedores rodam Whisper, então os dois assinam legenda
    stt_provider(wav, shared, s).map(|t| strip_credit(&t).to_string())
}

fn stt_provider(wav: Vec<u8>, shared: &Shared, s: &Settings) -> Result<String, String> {
    if s.stt_provider == "groq" && s.groq_ready() {
        match transcribe_groq(wav.clone(), s) {
            Ok(t) => {
                println!("[stt] transcrito via Groq (nuvem)");
                return Ok(t);
            }
            Err(e) => eprintln!("[stt] Groq falhou ({e}); caindo para o local"),
        }
    }
    match ensure_sidecar(shared, s) {
        Ok(()) => transcribe(wav, s.stt_port, &s.speech_lang),
        Err(e) if s.stt_provider == "local" && s.groq_ready() => {
            eprintln!("[stt] local indisponível ({e}); caindo para o Groq");
            transcribe_groq(wav, s)
        }
        Err(e) => Err(e),
    }
}

fn transcribe(wav: Vec<u8>, port: u16, lang: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = client
        .post(format!("http://127.0.0.1:{port}/stt?lang={lang}"))
        .body(wav)
        .send()
        .map_err(|e| format!("sidecar STT: {e}"))?
        .json()
        .map_err(|e| e.to_string())?;
    Ok(v["text"].as_str().unwrap_or("").trim().to_string())
}

fn build_prompt(s: &Settings) -> String {
    let style = s
        .profiles
        .iter()
        .find(|p| p.name == s.active_profile)
        .map(|p| p.style.clone())
        .unwrap_or_else(|| "texto natural corrigido, mantendo o tom do falante.".into());
    let mut p = RULES.to_string();
    let dict: Vec<&str> = s.dictionary.iter().map(|t| t.trim()).filter(|t| !t.is_empty()).collect();
    if !dict.is_empty() {
        p.push_str(&format!("6. Grafias obrigatórias quando essas palavras aparecerem: {}.\n", dict.join(", ")));
    }
    p.push_str("Responda SOMENTE com o texto final, sem comentários.\n");
    if s.output_lang != "same" && s.output_lang != s.speech_lang {
        let idioma = match s.output_lang.as_str() {
            "pt" => "português do Brasil",
            "en" => "inglês",
            "es" => "espanhol",
            other => other,
        };
        p.push_str(&format!(
            "Escreva o texto final em {idioma}, mantendo URLs, e-mails e nomes próprios intactos.\n"
        ));
    }
    p.push_str(&format!("Estilo: {style}\n\nTranscrição:\n"));
    p
}

/// Modelo de reserva: o acesso a modelos varia por conta/projeto, então a chave
/// de outro usuário pode não ter o modelo configurado (404) ou vê-lo saturado
/// (503/429). Cair aqui é melhor que devolver a transcrição crua — mas SÓ se o
/// reserva for confiável, e o antigo (`gemini-3.1-flash-lite`) não era: medido,
/// ele pendura 25% das chamadas até o timeout (3 de 12), o que transformou o
/// rebaixamento numa sessão inteira de ditados travando. O `-latest` fez 12/12
/// a 1,39s de mediana. Ele é um alias, então o modelo por baixo pode mudar sem
/// aviso — para um RESERVA isso é troca boa: nunca vira 404 quando o Google
/// aposenta uma versão, que é justamente o caso que traz alguém até aqui.
const FALLBACK_MODEL: &str = "gemini-flash-lite-latest";
/// Travado só depois de um 404: sem isso, uma conta sem o modelo preferido
/// pagaria uma chamada perdida em TODO ditado. 429 e 503 NÃO travam — são
/// congestionamento do minuto, e rebaixar a sessão inteira por causa de um pico
/// custaria caro (o 3.1-flash-lite mede 5,5s contra 1,4s do 3.5).
static USE_FALLBACK: AtomicBool = AtomicBool::new(false);

/// Só o 404 é veredito sobre o modelo ("essa chave não tem esse modelo"). 429 e
/// 503 falam do minuto, não do modelo — travar a sessão por causa deles trocaria
/// um pico de segundos por uma sessão inteira no modelo lento.
fn erro_e_definitivo(code: u16) -> bool {
    code == 404
}

fn modelo_efetivo(configurado: &str) -> &str {
    if USE_FALLBACK.load(Ordering::Relaxed) {
        FALLBACK_MODEL
    } else {
        configurado
    }
}

/// Teto por tentativa. O free tier trava requisições isoladas sem relação com o
/// tamanho da entrada — medido em ditados reais do history: 18,1s para 43
/// caracteres no mesmo lote em que 1729 caracteres saíram em 1,8s. E a travada
/// não gruda: refeita na hora, a chamada volta ao tempo normal (24/24 num teste
/// que cortava tudo acima de 2s). Por isso cortar cedo ganha do teto largo —
/// medido, o corte em 2s deu máximo de 4,1s contra 10,0s do corte em 8s.
/// 4s é o meio: acima do p95 real (~3,3s), então quase nenhuma chamada boa
/// morre, e ainda mata o estol antes de o ditado virar prejuízo.
const REWRITE_TIMEOUT: Duration = Duration::from_secs(4);

/// Err traz o status HTTP (0 = falha de rede, 408 = estourou o teto) para separar
/// "esse modelo não serve para esta chave" de "a internet caiu".
fn call_gemini(model: &str, key: &str, prompt: &str) -> Result<String, (u16, String)> {
    let client = reqwest::blocking::Client::builder()
        .timeout(REWRITE_TIMEOUT)
        .build()
        .map_err(|e| (0, e.to_string()))?;
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={key}"
    );
    let body = serde_json::json!({
        "contents": [{"parts": [{"text": prompt}]}],
        "generationConfig": {"temperature": 0.2}
    });
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .map_err(|e| (if e.is_timeout() { 408 } else { 0 }, format!("Gemini: {e}")))?;
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        return Err((status, format!("Gemini {model}: HTTP {status}")));
    }
    let v: serde_json::Value = resp.json().map_err(|e| (status, e.to_string()))?;
    let parts = &v["candidates"][0]["content"]["parts"];
    let text = parts
        .as_array()
        .map(|a| a.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join(""))
        .unwrap_or_default();
    if text.trim().is_empty() {
        return Err((status, format!("resposta vazia do Gemini: {v}")));
    }
    Ok(text.trim().to_string())
}

fn rewrite(raw: &str, s: &Settings) -> Result<String, String> {
    let key = s
        .gemini_api_key
        .clone()
        .filter(|k| !k.is_empty())
        .or_else(|| std::env::var("GEMINI_API_KEY").ok())
        .ok_or("chave do Gemini ausente (settings.json ou GEMINI_API_KEY)")?;
    let prompt = format!("{}{}", build_prompt(s), raw);
    let escolhido = modelo_efetivo(&s.gemini_model);
    match call_gemini(escolhido, &key, &prompt) {
        Ok(t) => Ok(t),
        // Travada isolada do free tier: a segunda tentativa pega outra fila e
        // costuma responder no tempo normal. Só cabe porque o teto caiu para 8s
        // — com os 30s antigos a segunda chamada chegaria depois de o usuário já
        // ter desistido. Uma tentativa só: duas somariam o dobro do teto.
        Err((408, e)) => {
            eprintln!("[gemini] estourou {}s ({e}); segunda tentativa", REWRITE_TIMEOUT.as_secs());
            call_gemini(escolhido, &key, &prompt).map_err(|(_, e2)| e2)
        }
        // Modelo fora do ar (404/503) ou cota do minuto estourada (429): o
        // fallback é outro modelo, logo outro balde de cota, então responde na
        // hora. Erro de rede não entra: erraria de novo.
        Err((code, e)) if (code == 404 || code == 429 || code == 503) && escolhido != FALLBACK_MODEL => {
            eprintln!("[gemini] {escolhido} indisponível ({e}); usando {FALLBACK_MODEL}");
            let t = call_gemini(FALLBACK_MODEL, &key, &prompt).map_err(|(_, e2)| e2)?;
            if erro_e_definitivo(code) {
                USE_FALLBACK.store(true, Ordering::Relaxed);
            }
            Ok(t)
        }
        Err((_, e)) => Err(e),
    }
}

fn apply_snippets(text: &str, snippets: &[Snippet]) -> String {
    let mut out = text.to_string();
    for sn in snippets {
        if sn.trigger.is_empty() {
            continue;
        }
        let lower_out = out.to_lowercase();
        let lower_trig = sn.trigger.to_lowercase();
        if let Some(pos) = lower_out.find(&lower_trig) {
            let mut end = pos + lower_trig.len();
            // consome pontuação final colada ao gatilho ("meu instagram." -> link)
            if out[end..].starts_with('.') || out[end..].starts_with(',') {
                end += 1;
            }
            out = format!("{}{}{}", &out[..pos], sn.text, &out[end..]);
        }
    }
    out
}

fn insert_text(text: &str) -> Result<(), String> {
    let mut clip = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let backup = clip.get_text().ok();
    clip.set_text(text).map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_millis(60));
    // modificador físico ainda pressionado + V sintético formaria combos do sistema
    platform::wait_modifiers_released(Duration::from_secs(1));
    platform::paste_shortcut()?;
    std::thread::sleep(Duration::from_millis(300));
    if let Some(b) = backup {
        let _ = clip.set_text(b);
    }
    Ok(())
}

#[derive(Serialize, Deserialize, Clone)]
struct HistoryEntry {
    ts: u64,
    profile: String,
    raw: String,
    r#final: String,
    audio_secs: f32,
    stt_secs: f32,
    total_secs: f32,
}

fn append_history(entry: &HistoryEntry) {
    let _ = std::fs::create_dir_all(config_dir());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(history_path()) {
        let _ = writeln!(f, "{}", serde_json::to_string(entry).unwrap());
    }
}

fn overlay_log(msg: &str) {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let _ = std::fs::create_dir_all(config_dir());
    if let Ok(mut f) =
        std::fs::OpenOptions::new().create(true).append(true).open(config_dir().join("overlay.log"))
    {
        let _ = writeln!(f, "{ts} {msg}");
    }
}

/// mostra o overlay e devolve o número desta exibição
fn overlay_show(app: &AppHandle) -> u64 {
    let shared = app.state::<Arc<Shared>>();
    let gen = shared.overlay_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let style = shared.settings.lock().unwrap().wave_style.clone();
    if let Some(o) = app.get_webview_window("overlay") {
        // monitor onde está o cursor (foco de digitação), não o primário
        let mon = o
            .cursor_position()
            .ok()
            .and_then(|p| o.monitor_from_point(p.x, p.y).ok().flatten())
            .or_else(|| o.primary_monitor().ok().flatten());
        if let (Some(mon), Ok(sz)) = (mon, o.outer_size()) {
            let m = mon.size();
            let x = mon.position().x + (m.width as i32 - sz.width as i32) / 2;
            // 72px fixos + 2% da altura do monitor: só o valor fixo encostava no
            // dock em telas grandes
            let margem = 72 + m.height as i32 * 2 / 100;
            let y = mon.position().y + m.height as i32 - sz.height as i32 - margem;
            if let Err(e) = o.set_position(PhysicalPosition::new(x, y)) {
                overlay_log(&format!("set_position falhou: {e}"));
            }
        }
        let _ = o.set_always_on_top(true);
        let _ = app.emit_to("overlay", "overlay_show", style);
    } else {
        overlay_log("janela overlay inexistente");
    }
    gen
}

/// só apaga se esta exibição ainda for a atual
///
/// A janela do overlay nasce visível e nunca é escondida: no Windows, esconder e
/// reexibir a janela deixa a webview sem pintar — a janela reaparecia no lugar
/// certo, no topo, do tamanho certo, e mesmo assim invisível, porque o conteúdo
/// nunca era apresentado. Quem "some" e "aparece" agora é só o conteúdo, do lado
/// do React; a janela em si fica parada, transparente e sem capturar cliques.
fn overlay_hide(app: &AppHandle, gen: u64) {
    if app.state::<Arc<Shared>>().overlay_gen.load(Ordering::SeqCst) != gen {
        return;
    }
    let _ = app.emit_to("overlay", "overlay_hide", ());
}

fn overlay_status(app: &AppHandle, state: &str, message: &str) {
    let _ = app.emit_to(
        "overlay",
        "overlay_status",
        serde_json::json!({ "state": state, "message": message }),
    );
}

/// mensagem técnica -> frase curta que cabe no overlay
fn friendly_error(e: &str) -> String {
    let low = e.to_lowercase();
    let msg = if low.contains("microfone") || low.contains("sem microfone") {
        "Microfone não encontrado"
    } else if low.contains("429") || low.contains("rate limit") || low.contains("quota") {
        "Limite da API atingido"
    } else if low.contains("401") || low.contains("403") || low.contains("chave") {
        "Chave de API inválida ou ausente"
    } else if low.contains("timed out")
        || low.contains("timeout")
        || low.contains("dns")
        || low.contains("connect")
    {
        "Sem conexão com o serviço"
    } else if low.contains("transcrição local") {
        "Transcrição local indisponível"
    } else if low.contains("alucinação") || low.contains("transcrição vazia") {
        "Nada foi dito"
    } else if low.contains("clipboard") || low.contains("área de transferência") {
        "Não consegui colar o texto"
    } else {
        return e.chars().take(70).collect();
    };
    msg.to_string()
}

/// mostra o motivo da falha por alguns segundos — sem isso o ditado some em silêncio
fn overlay_fail(app: &AppHandle, gen: u64, err: &str) {
    if app.state::<Arc<Shared>>().overlay_gen.load(Ordering::SeqCst) != gen {
        return;
    }
    overlay_status(app, "error", &friendly_error(err));
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(3200));
        overlay_hide(&app, gen);
    });
}

enum Cmd {
    Start,
    Stop,
    Cancel,
}

fn spawn_pipeline(shared: Arc<Shared>, app: AppHandle) -> Sender<Cmd> {
    let (tx, rx) = channel::<Cmd>();
    std::thread::spawn(move || {
        let mut rec = Recorder::new();
        let mut gen = 0u64;
        loop {
            match rx.recv() {
                Ok(Cmd::Start) => {
                    gen = overlay_show(&app);
                    let mic = shared.settings.lock().unwrap().mic.clone();
                    let app_lvl = app.clone();
                    let mut last = Instant::now() - Duration::from_secs(1);
                    let on_level: LevelFn = Box::new(move |rms| {
                        if last.elapsed() >= Duration::from_millis(40) {
                            last = Instant::now();
                            let _ = app_lvl.emit("level", rms);
                        }
                    });
                    if let Err(e) = rec.start(mic.as_deref(), on_level) {
                        eprintln!("[pipeline] gravação falhou: {e}");
                        overlay_fail(&app, gen, &e);
                    } else {
                        println!("[pipeline] gravando...");
                    }
                }
                Ok(Cmd::Cancel) => {
                    let _ = rec.stop();
                    overlay_hide(&app, gen);
                    println!("[pipeline] ditado cancelado");
                }
                Ok(Cmd::Stop) => {
                    let (samples, rate) = rec.stop();
                    let secs = samples.len() as f32 / rate as f32;
                    if secs < 0.4 {
                        println!("[pipeline] áudio curto demais ({secs:.1}s), ignorado");
                        overlay_hide(&app, gen);
                        continue;
                    }
                    let peak = peak_rms(&samples, rate);
                    if peak < 0.006 {
                        println!("[pipeline] sem fala detectada (pico rms {peak:.4}), ignorado");
                        overlay_hide(&app, gen);
                        continue;
                    }
                    // overlay segue no ar em "processando": soltar o atalho e ver a
                    // janela sumir por 2-3s é indistinguível de ter falhado
                    overlay_status(&app, "processing", "");
                    // processamento em thread própria: a thread do atalho fica livre e o
                    // próximo Start é imediato (overlay e gravação sem atraso de fila)
                    let s = shared.settings.lock().unwrap().clone();
                    let shared_w = shared.clone();
                    let app_w = app.clone();
                    let gen_w = gen;
                    std::thread::spawn(move || {
                        let t0 = Instant::now();
                        let result = run_stt(wav_bytes(&samples, rate), &shared_w, &s)
                            .and_then(|raw| {
                                if raw.is_empty() {
                                    return Err("transcrição vazia".into());
                                }
                                if is_hallucination(&raw, secs) {
                                    return Err(format!("alucinação de silêncio descartada: {raw:?}"));
                                }
                                let t_stt = t0.elapsed().as_secs_f32();
                                let final_text = match rewrite(&raw, &s) {
                                    Ok(t) => t,
                                    Err(e) => {
                                        eprintln!("[pipeline] reescrita falhou ({e}); usando texto bruto");
                                        raw.clone()
                                    }
                                };
                                let final_text = apply_snippets(&final_text, &s.snippets);
                                let total = t0.elapsed().as_secs_f32();
                                println!("[pipeline] áudio {secs:.1}s | stt {t_stt:.2}s | total {total:.2}s");
                                if s.history_enabled {
                                    append_history(&HistoryEntry {
                                        ts: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
                                        profile: s.active_profile.clone(),
                                        raw,
                                        r#final: final_text.clone(),
                                        audio_secs: secs,
                                        stt_secs: t_stt,
                                        total_secs: total,
                                    });
                                }
                                *shared_w.last_text.lock().unwrap() = final_text.clone();
                                let _serial = shared_w.insert_lock.lock().unwrap();
                                insert_text(&final_text)
                            });
                        match result {
                            Ok(()) => overlay_hide(&app_w, gen_w),
                            Err(e) => {
                                eprintln!("[pipeline] erro: {e}");
                                overlay_fail(&app_w, gen_w, &e);
                            }
                        }
                    });
                }
                Err(_) => break,
            }
        }
    });
    tx
}

// Estado compartilhado do rastreador de atalho: alimentado pelo hook LL no
// Windows e pelo CGEventTap no macOS (ambos em platform::).
struct HookState {
    shared: Arc<Shared>,
    tx: Sender<Cmd>,
    pressed: HashSet<u32>,
    active: bool,
    recording: bool,
}

fn handle_key(st: &mut HookState, vk: u32, down: bool) {
    if down {
        st.pressed.insert(vk);
    } else {
        st.pressed.remove(&vk);
    }
    // aborta o ditado em andamento sem inserir nada; no modo hold o atalho segue
    // pressionado e o soltar seguinte não dispara Stop (recording já é false)
    if down && st.recording && *st.shared.cancel.lock().unwrap() == Some(vk) {
        st.recording = false;
        let _ = st.tx.send(Cmd::Cancel);
        return;
    }
    let groups = st.shared.groups.lock().unwrap();
    let satisfied =
        !groups.is_empty() && groups.iter().all(|g| g.iter().any(|k| st.pressed.contains(k)));
    drop(groups);
    let mode = st.shared.settings.lock().unwrap().mode.clone();
    if satisfied && !st.active {
        st.active = true;
        println!("[hotkey] atalho ativado");
        if mode == "toggle" {
            st.recording = !st.recording;
            let _ = st.tx.send(if st.recording { Cmd::Start } else { Cmd::Stop });
        } else {
            st.recording = true;
            let _ = st.tx.send(Cmd::Start);
        }
    } else if !satisfied && st.active {
        st.active = false;
        if mode != "toggle" && st.recording {
            st.recording = false;
            let _ = st.tx.send(Cmd::Stop);
        }
    }
}

#[tauri::command]
fn get_settings(state: tauri::State<Arc<Shared>>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let m = app.autolaunch();
    if enabled {
        m.enable().map_err(|e| e.to_string())
    } else {
        m.disable().map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    state: tauri::State<Arc<Shared>>,
    settings: Settings,
) -> Result<(), String> {
    persist_settings(&settings)?;
    *state.groups.lock().unwrap() = parse_hotkey(&settings.hotkey);
    *state.cancel.lock().unwrap() = parse_cancel_key(&settings.cancel_key);
    let warm_local = settings.stt_provider == "local";
    rebuild_tray_menu(&app, &settings);
    *state.settings.lock().unwrap() = settings;
    if warm_local {
        let shared = state.inner().clone();
        std::thread::spawn(move || {
            let s = shared.settings.lock().unwrap().clone();
            if let Err(e) = ensure_sidecar(&shared, &s) {
                eprintln!("[stt] falha ao aquecer local: {e}");
            }
        });
    }
    Ok(())
}

#[tauri::command]
fn list_mics() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

#[tauri::command]
fn get_history(query: Option<String>, limit: Option<usize>) -> Vec<HistoryEntry> {
    let q = query.unwrap_or_default().to_lowercase();
    let limit = limit.unwrap_or(50);
    let Ok(txt) = std::fs::read_to_string(history_path()) else {
        return Vec::new();
    };
    txt.lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<HistoryEntry>(l).ok())
        .filter(|e| {
            q.is_empty()
                || e.r#final.to_lowercase().contains(&q)
                || e.raw.to_lowercase().contains(&q)
        })
        .take(limit)
        .collect()
}

#[tauri::command]
fn clear_history() -> Result<(), String> {
    match std::fs::remove_file(history_path()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[derive(Serialize)]
struct Diagnostics {
    stt_provider: String,
    groq_key_set: bool,
    sidecar_ok: bool,
    sidecar_configured: bool,
    sidecar_device: String,
    default_mic: String,
    gemini_key_set: bool,
    gemini_model: String,
    recent: Vec<HistoryEntry>,
}

#[tauri::command]
fn get_diagnostics(state: tauri::State<Arc<Shared>>) -> Diagnostics {
    let s = state.settings.lock().unwrap().clone();
    let (sidecar_ok, sidecar_device) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()
        .and_then(|c| c.get(format!("http://127.0.0.1:{}/health", s.stt_port)).send().ok())
        .and_then(|r| r.json::<serde_json::Value>().ok())
        .map(|v| (true, v["device"].as_str().unwrap_or("?").to_string()))
        .unwrap_or((false, "offline".into()));
    let default_mic = cpal::default_host()
        .default_input_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_else(|| "nenhum".into());
    // sem os dois caminhos no disco o modo local nunca sobe — e "descarregado"
    // parece normal, escondendo que ele simplesmente não existe nesta máquina
    let sidecar_configured =
        std::path::Path::new(&s.python).exists() && std::path::Path::new(&s.sidecar).exists();
    Diagnostics {
        stt_provider: s.stt_provider.clone(),
        groq_key_set: s.groq_ready(),
        sidecar_ok,
        sidecar_configured,
        sidecar_device,
        default_mic,
        gemini_key_set: s.gemini_api_key.as_deref().map(|k| !k.is_empty()).unwrap_or(false)
            || std::env::var("GEMINI_API_KEY").is_ok(),
        gemini_model: s.gemini_model.clone(),
        recent: get_history(None, Some(10)),
    }
}

fn as_menu_refs(v: &[CheckMenuItem<tauri::Wry>]) -> Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> {
    v.iter().map(|i| i as &dyn tauri::menu::IsMenuItem<tauri::Wry>).collect()
}

fn build_tray_menu(app: &AppHandle, s: &Settings) -> tauri::Result<Menu<tauri::Wry>> {
    let items: Vec<CheckMenuItem<tauri::Wry>> = s
        .profiles
        .iter()
        .map(|p| {
            CheckMenuItem::with_id(
                app,
                format!("profile:{}", p.name),
                &p.name,
                true,
                p.name == s.active_profile,
                None::<&str>,
            )
        })
        .collect::<Result<_, _>>()?;
    let refs = as_menu_refs(&items);
    let perfil = Submenu::with_items(app, "Perfil", true, &refs)?;

    let langs = |prefix: &str, atual: &str, opts: &[(&str, &str)]| -> tauri::Result<Vec<CheckMenuItem<tauri::Wry>>> {
        opts.iter()
            .map(|(code, label)| {
                CheckMenuItem::with_id(
                    app,
                    format!("{prefix}:{code}"),
                    *label,
                    true,
                    *code == atual,
                    None::<&str>,
                )
            })
            .collect()
    };
    let fala_items = langs(
        "speech",
        &s.speech_lang,
        &[
            ("pt", "Português (Brasil)"),
            ("en", "English"),
            ("es", "Español"),
            ("auto", "Detectar automaticamente"),
        ],
    )?;
    let saida_items = langs(
        "out",
        &s.output_lang,
        &[
            ("same", "Mesmo da fala"),
            ("pt", "Português (Brasil)"),
            ("en", "English"),
            ("es", "Español"),
        ],
    )?;
    let fala_refs = as_menu_refs(&fala_items);
    let saida_refs = as_menu_refs(&saida_items);
    let fala = Submenu::with_items(app, "Fala", true, &fala_refs)?;
    let saida = Submenu::with_items(app, "Saída", true, &saida_refs)?;
    let idioma = Submenu::with_items(app, "Idioma", true, &[&fala, &saida])?;

    // atalho de um clique para o par mais usado; desmarcar volta a saída para "mesmo da fala"
    let pt_en = CheckMenuItem::with_id(
        app,
        "lang_pt_en",
        "PT falado → EN escrito",
        true,
        s.speech_lang == "pt" && s.output_lang == "en",
        None::<&str>,
    )?;

    let copy_last =
        MenuItem::with_id(app, "copy_last", "Copiar último ditado", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "Configurações", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
    Menu::with_items(app, &[&perfil, &pt_en, &idioma, &copy_last, &show, &quit])
}

/// altera as configurações no estado, grava em disco e avisa tray e janela
fn update_settings(app: &AppHandle, f: impl FnOnce(&mut Settings)) {
    let s = {
        let shared = app.state::<Arc<Shared>>();
        let mut g = shared.settings.lock().unwrap();
        f(&mut g);
        g.clone()
    };
    let _ = persist_settings(&s);
    rebuild_tray_menu(app, &s);
    let _ = app.emit("settings_changed", ());
}

fn rebuild_tray_menu(app: &AppHandle, s: &Settings) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id("tray"), build_tray_menu(app, s)) {
        let _ = tray.set_menu(Some(menu));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// casos reais do history.jsonl: o Whisper assina o fim da transcrição
    #[test]
    fn assinatura_de_legenda_nao_chega_ao_texto_final() {
        for (entrada, esperado) in [
            (
                "Mas isso será em um segundo momento. Legenda Adriana Zanotto",
                "Mas isso será em um segundo momento.",
            ),
            (
                "Fica bom para você? Legenda Adriana Zanotto",
                "Fica bom para você?",
            ),
            (
                "Tem texto. Legenda: Adriana Zanotto.",
                "Tem texto.",
            ),
            (
                "Foi ótimo. Legendas pela comunidade Amara.org",
                "Foi ótimo.",
            ),
            // "legenda" dita pelo falante no meio da frase: preservar
            (
                "Quero as tabelas com título, legenda, nome das variáveis.",
                "Quero as tabelas com título, legenda, nome das variáveis.",
            ),
            // frase final curta que só começa parecido: preservar
            (
                "Terminei o vídeo. Legendei tudo ontem.",
                "Terminei o vídeo. Legendei tudo ontem.",
            ),
            ("Sem assinatura nenhuma aqui.", "Sem assinatura nenhuma aqui."),
        ] {
            assert_eq!(strip_credit(entrada), esperado, "entrada: {entrada:?}");
        }
        // sozinha, sem frase anterior, cai no guarda de alucinação
        assert!(is_hallucination("Legenda Adriana Zanotto", 0.8));
        assert!(!is_hallucination("Legenda Adriana Zanotto", 30.0));
    }

    #[test]
    fn fallback_so_troca_o_modelo_depois_de_travado() {
        USE_FALLBACK.store(false, Ordering::Relaxed);
        assert_eq!(modelo_efetivo("gemini-3.5-flash-lite"), "gemini-3.5-flash-lite");
        USE_FALLBACK.store(true, Ordering::Relaxed);
        assert_eq!(modelo_efetivo("gemini-3.5-flash-lite"), FALLBACK_MODEL);
        USE_FALLBACK.store(false, Ordering::Relaxed);
    }

    #[test]
    fn so_404_rebaixa_a_sessao_inteira() {
        assert!(erro_e_definitivo(404), "404 = a chave não tem o modelo, é definitivo");
        // 408 (teto estourado), 429 (cota do minuto), 503 (saturado) e 0 (rede)
        // passam; travar em qualquer um deles rebaixaria a sessão por um pico
        for transitorio in [0u16, 408, 429, 503, 500] {
            assert!(!erro_e_definitivo(transitorio), "{transitorio} não pode travar a sessão");
        }
    }

    fn hook(mode: &str, cancel: &str) -> (HookState, std::sync::mpsc::Receiver<Cmd>) {
        let mut s = Settings::default();
        s.mode = mode.into();
        let shared = Arc::new(Shared {
            groups: Mutex::new(parse_hotkey(&s.hotkey)),
            cancel: Mutex::new(parse_cancel_key(cancel)),
            sidecar: Mutex::new(None),
            settings: Mutex::new(s),
            insert_lock: Mutex::new(()),
            last_text: Mutex::new(String::new()),
            overlay_gen: AtomicU64::new(0),
        });
        let (tx, rx) = channel::<Cmd>();
        (HookState { shared, tx, pressed: HashSet::new(), active: false, recording: false }, rx)
    }

    /// primeiro grupo do atalho padrão (Ctrl) e segundo (Win/Cmd)
    fn hotkey_codes(st: &HookState) -> (u32, u32) {
        let g = st.shared.groups.lock().unwrap();
        (g[0][0], g[1][0])
    }

    #[test]
    fn cancelar_no_hold_nao_dispara_stop_ao_soltar() {
        let (mut st, rx) = hook("hold", "SPACE");
        let (ctrl, win) = hotkey_codes(&st);
        let space = platform::key_from_name("SPACE");

        handle_key(&mut st, ctrl, true);
        handle_key(&mut st, win, true);
        assert!(matches!(rx.try_recv(), Ok(Cmd::Start)));

        handle_key(&mut st, space, true);
        assert!(matches!(rx.try_recv(), Ok(Cmd::Cancel)));

        // soltar o atalho depois do cancelamento não pode mandar o áudio para o STT
        handle_key(&mut st, space, false);
        handle_key(&mut st, win, false);
        handle_key(&mut st, ctrl, false);
        assert!(rx.try_recv().is_err(), "Stop fantasma depois de cancelar");

        // e o próximo ditado continua funcionando
        handle_key(&mut st, ctrl, true);
        handle_key(&mut st, win, true);
        assert!(matches!(rx.try_recv(), Ok(Cmd::Start)));
    }

    #[test]
    fn cancelar_no_toggle_permite_novo_ditado() {
        let (mut st, rx) = hook("toggle", "ESC");
        let (ctrl, win) = hotkey_codes(&st);
        let esc = platform::key_from_name("ESC");

        handle_key(&mut st, ctrl, true);
        handle_key(&mut st, win, true);
        handle_key(&mut st, win, false);
        handle_key(&mut st, ctrl, false);
        assert!(matches!(rx.try_recv(), Ok(Cmd::Start)));

        handle_key(&mut st, esc, true);
        handle_key(&mut st, esc, false);
        assert!(matches!(rx.try_recv(), Ok(Cmd::Cancel)));

        handle_key(&mut st, ctrl, true);
        handle_key(&mut st, win, true);
        assert!(matches!(rx.try_recv(), Ok(Cmd::Start)), "toggle travado depois do cancelamento");
    }

    #[test]
    fn cancelamento_desativado_ignora_a_tecla() {
        let (mut st, rx) = hook("hold", "");
        let (ctrl, win) = hotkey_codes(&st);
        handle_key(&mut st, ctrl, true);
        handle_key(&mut st, win, true);
        assert!(matches!(rx.try_recv(), Ok(Cmd::Start)));

        handle_key(&mut st, platform::key_from_name("SPACE"), true);
        assert!(rx.try_recv().is_err());

        handle_key(&mut st, win, false);
        assert!(matches!(rx.try_recv(), Ok(Cmd::Stop)));
    }

    #[test]
    fn erros_viram_frases_curtas() {
        assert_eq!(friendly_error("Groq HTTP 401: {\"error\":...}"), "Chave de API inválida ou ausente");
        assert_eq!(friendly_error("Groq HTTP 429: rate limit"), "Limite da API atingido");
        assert_eq!(friendly_error("microfone 'Yeti' não encontrado"), "Microfone não encontrado");
        assert_eq!(friendly_error("transcrição vazia"), "Nada foi dito");
        // sem regra conhecida: mostra o original, curto o bastante para caber
        assert!(friendly_error(&"x".repeat(200)).chars().count() <= 70);
    }
}

pub fn run() {
    let (settings, first_run) = load_settings();
    // migra chaves em texto plano para DPAPI na primeira oportunidade
    let _ = persist_settings(&settings);
    // com Groq configurado como provedor, a GPU fica livre: o local só aquece sob demanda
    let start_local = settings.stt_provider != "groq" || !settings.groq_ready();
    let shared = Arc::new(Shared {
        groups: Mutex::new(parse_hotkey(&settings.hotkey)),
        cancel: Mutex::new(parse_cancel_key(&settings.cancel_key)),
        sidecar: Mutex::new(if start_local { platform::spawn_sidecar(&settings) } else { None }),
        settings: Mutex::new(settings.clone()),
        insert_lock: Mutex::new(()),
        last_text: Mutex::new(String::new()),
        overlay_gen: AtomicU64::new(0),
    });

    let shared_exit = shared.clone();
    let shared_setup = shared.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // no boot o app pode subir duas vezes (autostart do registro + atalho em
            // shell:startup, ou o "reabrir apps ao entrar" do Windows). A segunda cai
            // aqui e abria as configurações na cara do usuário — quem vem do boot só sai.
            if args.iter().any(|a| a == "--autostart") {
                return;
            }
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .manage(shared)
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            list_mics,
            get_history,
            clear_history,
            get_diagnostics,
            get_autostart,
            set_autostart
        ])
        .setup(move |app| {
            // app de bandeja: no mac o ícone vive só na menu bar, sem Dock
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // impede o App Nap: sem janela visível o macOS suspende o processo e o
            // event tap do atalho para de responder até o app "acordar"
            #[cfg(target_os = "macos")]
            unsafe {
                use objc2::runtime::AnyObject;
                use objc2::{class, msg_send};
                let pi: *mut AnyObject = msg_send![class!(NSProcessInfo), processInfo];
                let reason: *mut AnyObject = msg_send![
                    class!(NSString),
                    stringWithUTF8String: c"hotkey e ditado globais".as_ptr()
                ];
                // NSActivityUserInitiatedAllowingIdleSystemSleep
                let opts: usize = 0x00FF_FFFF & !(1 << 20);
                let token: *mut AnyObject =
                    msg_send![pi, beginActivityWithOptions: opts, reason: reason];
                let _: *mut AnyObject = msg_send![token, retain]; // token vive para sempre
            }

            {
                use tauri_plugin_autostart::ManagerExt;
                let al = app.autolaunch();
                // re-habilitar reescreve a chave do registro com os args atuais
                // (--autostart), inclusive em quem já tinha o autostart de versões antigas
                if first_run || al.is_enabled().unwrap_or(false) {
                    let _ = al.enable();
                }
            }
            let tx = spawn_pipeline(shared_setup.clone(), app.handle().clone());
            platform::spawn_hotkey_listener(shared_setup.clone(), tx);

            if let Some(o) = app.get_webview_window("overlay") {
                let _ = o.set_ignore_cursor_events(true);
                // Spaces: sem isto o overlay só existe na mesa onde nasceu (a das
                // configurações) e "some" quando se dita em outra mesa/tela cheia.
                // CanJoinAllSpaces (1<<0) | Stationary (1<<4) | FullScreenAuxiliary (1<<8)
                #[cfg(target_os = "macos")]
                if let Ok(nsw) = o.ns_window() {
                    let nsw = nsw as *mut objc2::runtime::AnyObject;
                    let behavior: usize = (1 << 0) | (1 << 4) | (1 << 8);
                    unsafe {
                        let _: () = objc2::msg_send![nsw, setCollectionBehavior: behavior];
                    }
                }
            }

            let menu_settings = shared_setup.settings.lock().unwrap().clone();
            let menu = build_tray_menu(app.handle(), &menu_settings)?;
            let tray = TrayIconBuilder::with_id("tray");
            // menu bar do mac usa ícone template: só o glifo da onda, monocromático,
            // e o sistema o adapta ao estilo/tema da barra (como os ícones nativos)
            #[cfg(target_os = "macos")]
            let tray = tray
                .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))?)
                .icon_as_template(true);
            #[cfg(not(target_os = "macos"))]
            let tray = tray.icon(app.default_window_icon().unwrap().clone());
            tray.menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(w) = tray.app_handle().get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                })
                .tooltip("Open Flow — segure o atalho para ditar")
                .on_menu_event(move |app, event| {
                    let id = event.id.as_ref();
                    if let Some(name) = id.strip_prefix("profile:") {
                        let name = name.to_string();
                        update_settings(app, |g| g.active_profile = name);
                        return;
                    }
                    if let Some(code) = id.strip_prefix("speech:") {
                        let code = code.to_string();
                        update_settings(app, |g| g.speech_lang = code);
                        return;
                    }
                    if let Some(code) = id.strip_prefix("out:") {
                        let code = code.to_string();
                        update_settings(app, |g| g.output_lang = code);
                        return;
                    }
                    match id {
                        "lang_pt_en" => update_settings(app, |g| {
                            let ligado = g.speech_lang == "pt" && g.output_lang == "en";
                            g.speech_lang = "pt".into();
                            g.output_lang = if ligado { "same".into() } else { "en".into() };
                        }),
                        "copy_last" => {
                            let text = app.state::<Arc<Shared>>().last_text.lock().unwrap().clone();
                            if !text.is_empty() {
                                if let Ok(mut c) = arboard::Clipboard::new() {
                                    let _ = c.set_text(text);
                                }
                            }
                        }
                        "show" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("erro ao iniciar o Open Flow")
        .run(move |_app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(child) = shared_exit.sidecar.lock().unwrap().as_mut() {
                    let _ = child.kill();
                }
            }
        });
}
