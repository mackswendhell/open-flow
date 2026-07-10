use std::collections::HashSet;
use std::io::Cursor;
use std::io::Write as _;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition};

const RULES: &str = "Você é um editor de ditado. Receberá a transcrição literal de uma fala em pt-BR.\n\
Regras invioláveis:\n\
1. Quando o falante se corrige (\"não, melhor...\", \"deixa eu corrigir...\", \"quer dizer...\"), mantenha APENAS a versão final e descarte a descartada.\n\
2. Remova hesitações, muletas (\"é...\", \"tipo\", \"então...\") e repetições.\n\
3. Corrija concordância e pontuação. NÃO adicione fatos, NÃO resuma, NÃO omita conteúdo.\n";

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
    mode: String,
    mic: Option<String>,
    stt_provider: String,
    groq_api_key: Option<String>,
    groq_model: String,
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
        p("Bruto corrigido", "texto natural corrigido, mantendo o tom do falante."),
        p("Jurídico formal", "certidão ou comunicação de oficial de justiça, linguagem jurídica formal brasileira (ex.: \"Certifico e dou fé que...\", \"genitor\", \"Ante o exposto, devolvo o presente mandado\")."),
        p("E-mail profissional", "e-mail profissional claro e cordial, com saudação e fecho quando fizer sentido."),
        p("WhatsApp curto", "mensagem curta e informal de WhatsApp, direta, sem formalidade excessiva."),
        p("Roteiro de vídeo", "roteiro falado para vídeo do YouTube: frases curtas, ritmo de fala natural, tom de conversa."),
    ]
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            hotkey: "Ctrl+Win".into(),
            mode: "hold".into(),
            mic: None,
            stt_provider: "groq".into(),
            groq_api_key: None,
            groq_model: "whisper-large-v3-turbo".into(),
            gemini_model: "gemini-3.1-flash-lite".into(),
            gemini_api_key: None,
            wave_style: "tech".into(),
            theme: "dark".into(),
            profiles: default_profiles(),
            active_profile: "Bruto corrigido".into(),
            dictionary: Vec::new(),
            snippets: Vec::new(),
            history_enabled: true,
            // ponytail: caminhos desta máquina; viram recurso empacotado na fase 3
            python: r"C:\dev\open-flow\.venv\Scripts\python.exe".into(),
            sidecar: r"C:\dev\open-flow\sidecar\stt_server.py".into(),
            stt_port: 17765,
        }
    }
}

struct Shared {
    settings: Mutex<Settings>,
    groups: Mutex<Vec<Vec<rdev::Key>>>,
    sidecar: Mutex<Option<std::process::Child>>,
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

fn key_from_name(name: &str) -> rdev::Key {
    use rdev::Key::*;
    match name {
        "F1" => F1, "F2" => F2, "F3" => F3, "F4" => F4, "F5" => F5, "F6" => F6,
        "F7" => F7, "F8" => F8, "F10" => F10, "F11" => F11, "F12" => F12,
        "SCROLLLOCK" => ScrollLock, "PAUSE" => Pause, "CAPSLOCK" => CapsLock,
        "INSERT" => Insert, "HOME" => Home, "END" => End,
        _ => F9,
    }
}

fn parse_hotkey(s: &str) -> Vec<Vec<rdev::Key>> {
    use rdev::Key::*;
    s.split('+')
        .map(|t| match t.trim().to_uppercase().as_str() {
            "CTRL" | "CONTROL" => vec![ControlLeft, ControlRight],
            "WIN" | "META" | "SUPER" => vec![MetaLeft, MetaRight],
            "ALT" => vec![Alt, AltGr],
            "SHIFT" => vec![ShiftLeft, ShiftRight],
            other => vec![key_from_name(other)],
        })
        .collect()
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
        if let Some(c) = guard.as_mut() {
            let _ = c.kill();
        }
        *guard = spawn_sidecar(s);
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
    let form = reqwest::blocking::multipart::Form::new()
        .part("file", part)
        .text("model", s.groq_model.clone())
        .text("language", "pt")
        .text("response_format", "json");
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

fn run_stt(wav: Vec<u8>, shared: &Shared, s: &Settings) -> Result<String, String> {
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
        Ok(()) => transcribe(wav, s.stt_port),
        Err(e) if s.stt_provider == "local" && s.groq_ready() => {
            eprintln!("[stt] local indisponível ({e}); caindo para o Groq");
            transcribe_groq(wav, s)
        }
        Err(e) => Err(e),
    }
}

fn transcribe(wav: Vec<u8>, port: u16) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = client
        .post(format!("http://127.0.0.1:{port}/stt"))
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
    if !s.dictionary.is_empty() {
        p.push_str(&format!("4. Grafias obrigatórias quando essas palavras aparecerem: {}.\n", s.dictionary.join(", ")));
    }
    p.push_str("Responda SOMENTE com o texto final, sem comentários.\n");
    p.push_str(&format!("Estilo: {style}\n\nTranscrição:\n"));
    p
}

fn rewrite(raw: &str, s: &Settings) -> Result<String, String> {
    let key = s
        .gemini_api_key
        .clone()
        .filter(|k| !k.is_empty())
        .or_else(|| std::env::var("GEMINI_API_KEY").ok())
        .ok_or("chave do Gemini ausente (settings.json ou GEMINI_API_KEY)")?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        s.gemini_model, key
    );
    let body = serde_json::json!({
        "contents": [{"parts": [{"text": format!("{}{}", build_prompt(s), raw)}]}],
        "generationConfig": {"temperature": 0.2}
    });
    let v: serde_json::Value = client
        .post(url)
        .json(&body)
        .send()
        .map_err(|e| format!("Gemini: {e}"))?
        .json()
        .map_err(|e| e.to_string())?;
    let parts = &v["candidates"][0]["content"]["parts"];
    let text = parts
        .as_array()
        .map(|a| a.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join(""))
        .unwrap_or_default();
    if text.trim().is_empty() {
        return Err(format!("resposta vazia do Gemini: {v}"));
    }
    Ok(text.trim().to_string())
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
    use enigo::{Direction, Enigo, Key, Keyboard};
    let mut clip = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let backup = clip.get_text().ok();
    clip.set_text(text).map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_millis(60));
    let mut enigo = Enigo::new(&enigo::Settings::default()).map_err(|e| e.to_string())?;
    enigo.key(Key::Control, Direction::Press).map_err(|e| e.to_string())?;
    enigo.key(Key::Unicode('v'), Direction::Click).map_err(|e| e.to_string())?;
    enigo.key(Key::Control, Direction::Release).map_err(|e| e.to_string())?;
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

fn overlay_show(app: &AppHandle) {
    if let Some(o) = app.get_webview_window("overlay") {
        if let (Ok(Some(mon)), Ok(sz)) = (o.primary_monitor(), o.outer_size()) {
            let m = mon.size();
            let x = mon.position().x + (m.width as i32 - sz.width as i32) / 2;
            let y = mon.position().y + m.height as i32 - sz.height as i32 - 72;
            let _ = o.set_position(PhysicalPosition::new(x, y));
        }
        let _ = o.show();
    }
}

fn overlay_hide(app: &AppHandle) {
    if let Some(o) = app.get_webview_window("overlay") {
        let _ = o.hide();
    }
}

enum Cmd {
    Start,
    Stop,
}

fn spawn_pipeline(shared: Arc<Shared>, app: AppHandle) -> Sender<Cmd> {
    let (tx, rx) = channel::<Cmd>();
    std::thread::spawn(move || {
        let mut rec = Recorder::new();
        loop {
            match rx.recv() {
                Ok(Cmd::Start) => {
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
                    } else {
                        overlay_show(&app);
                        println!("[pipeline] gravando...");
                    }
                }
                Ok(Cmd::Stop) => {
                    overlay_hide(&app);
                    let (samples, rate) = rec.stop();
                    let secs = samples.len() as f32 / rate as f32;
                    if secs < 0.4 {
                        println!("[pipeline] áudio curto demais ({secs:.1}s), ignorado");
                        continue;
                    }
                    let s = shared.settings.lock().unwrap().clone();
                    let t0 = Instant::now();
                    let result = run_stt(wav_bytes(&samples, rate), &shared, &s)
                        .and_then(|raw| {
                            if raw.is_empty() {
                                return Err("transcrição vazia".into());
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
                            insert_text(&final_text)
                        });
                    if let Err(e) = result {
                        eprintln!("[pipeline] erro: {e}");
                    }
                }
                Err(_) => break,
            }
        }
    });
    tx
}

fn spawn_hotkey_listener(shared: Arc<Shared>, tx: Sender<Cmd>) {
    std::thread::spawn(move || {
        let mut pressed: HashSet<rdev::Key> = HashSet::new();
        let mut active = false;
        let mut recording = false;
        println!("[hotkey] instalando hook global...");
        let result = rdev::listen(move |event| {
            let (key, down) = match event.event_type {
                rdev::EventType::KeyPress(k) => (k, true),
                rdev::EventType::KeyRelease(k) => (k, false),
                _ => return,
            };
            if down {
                pressed.insert(key);
            } else {
                pressed.remove(&key);
            }
            let groups = shared.groups.lock().unwrap();
            let satisfied =
                !groups.is_empty() && groups.iter().all(|g| g.iter().any(|k| pressed.contains(k)));
            drop(groups);
            let mode = shared.settings.lock().unwrap().mode.clone();
            if satisfied && !active {
                active = true;
                println!("[hotkey] atalho ativado");
                if mode == "toggle" {
                    recording = !recording;
                    let _ = tx.send(if recording { Cmd::Start } else { Cmd::Stop });
                } else {
                    recording = true;
                    let _ = tx.send(Cmd::Start);
                }
            } else if !satisfied && active {
                active = false;
                if mode != "toggle" && recording {
                    recording = false;
                    let _ = tx.send(Cmd::Stop);
                }
            }
        });
        if let Err(e) = result {
            eprintln!("[hotkey] hook falhou: {e:?}");
        }
    });
}

fn spawn_sidecar(s: &Settings) -> Option<std::process::Child> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if !std::path::Path::new(&s.python).exists() || !std::path::Path::new(&s.sidecar).exists() {
        println!("[sidecar] transcrição local não configurada — operando só em nuvem (Groq)");
        return None;
    }
    match std::process::Command::new(&s.python)
        .arg(&s.sidecar)
        .env("OPENFLOW_STT_PORT", s.stt_port.to_string())
        .env("OPENFLOW_PARENT_PID", std::process::id().to_string())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("[sidecar] falha ao iniciar ({e}) — inicie manualmente: {} {}", s.python, s.sidecar);
            None
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
fn save_settings(state: tauri::State<Arc<Shared>>, settings: Settings) -> Result<(), String> {
    let path = settings_path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    *state.groups.lock().unwrap() = parse_hotkey(&settings.hotkey);
    let warm_local = settings.stt_provider == "local";
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
    Diagnostics {
        stt_provider: s.stt_provider.clone(),
        groq_key_set: s.groq_ready(),
        sidecar_ok,
        sidecar_device,
        default_mic,
        gemini_key_set: s.gemini_api_key.as_deref().map(|k| !k.is_empty()).unwrap_or(false)
            || std::env::var("GEMINI_API_KEY").is_ok(),
        gemini_model: s.gemini_model.clone(),
        recent: get_history(None, Some(10)),
    }
}

pub fn run() {
    let (settings, first_run) = load_settings();
    // com Groq configurado como provedor, a GPU fica livre: o local só aquece sob demanda
    let start_local = settings.stt_provider != "groq" || !settings.groq_ready();
    let shared = Arc::new(Shared {
        groups: Mutex::new(parse_hotkey(&settings.hotkey)),
        sidecar: Mutex::new(if start_local { spawn_sidecar(&settings) } else { None }),
        settings: Mutex::new(settings.clone()),
    });

    let shared_exit = shared.clone();
    let shared_setup = shared.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
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
            if first_run {
                use tauri_plugin_autostart::ManagerExt;
                let _ = app.autolaunch().enable();
            }
            let tx = spawn_pipeline(shared_setup.clone(), app.handle().clone());
            spawn_hotkey_listener(shared_setup.clone(), tx);

            if let Some(o) = app.get_webview_window("overlay") {
                let _ = o.set_ignore_cursor_events(true);
            }

            let show = MenuItem::with_id(app, "show", "Configurações", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
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
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
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
