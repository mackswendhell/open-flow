import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import Overlay from "./Overlay";
import canalImg from "./assets/canal.png";
import pixQrImg from "./assets/pix-qr.png";
import "./App.css";

const isMac = navigator.userAgent.includes("Mac");

export interface Profile {
  name: string;
  style: string;
}

export interface Snippet {
  trigger: string;
  text: string;
}

export interface Settings {
  hotkey: string;
  cancel_key: string;
  mode: string;
  mic: string | null;
  stt_provider: string;
  groq_api_key: string | null;
  groq_model: string;
  speech_lang: string;
  output_lang: string;
  gemini_model: string;
  gemini_api_key: string | null;
  wave_style: string;
  theme: string;
  profiles: Profile[];
  active_profile: string;
  dictionary: string[];
  snippets: Snippet[];
  history_enabled: boolean;
  python: string;
  sidecar: string;
  stt_port: number;
}

interface HistoryEntry {
  ts: number;
  profile: string;
  raw: string;
  final: string;
  audio_secs: number;
  stt_secs: number;
  total_secs: number;
}

interface Diagnostics {
  stt_provider: string;
  groq_key_set: boolean;
  sidecar_ok: boolean;
  sidecar_configured: boolean;
  sidecar_device: string;
  default_mic: string;
  gemini_key_set: boolean;
  gemini_model: string;
  recent: HistoryEntry[];
}

const TABS = ["Geral", "Insights", "Perfis", "Dicionário", "Snippets", "Histórico", "Diagnóstico", "Apoie"] as const;
type Tab = (typeof TABS)[number];

function App() {
  const isOverlay = new URLSearchParams(window.location.search).has("overlay");
  if (isOverlay) {
    return <Overlay />;
  }
  return <SettingsApp />;
}

function SettingsApp() {
  const [tab, setTab] = useState<Tab>("Geral");
  const [settings, setSettings] = useState<Settings | null>(null);
  const [saved, setSaved] = useState(true);
  const timer = useRef<number | undefined>(undefined);
  const first = useRef(true);

  useEffect(() => {
    invoke<Settings>("get_settings").then(setSettings);
    // troca de perfil pela bandeja: recarrega para a UI não sobrescrever com estado velho
    const un = listen("settings_changed", () => {
      invoke<Settings>("get_settings").then(setSettings);
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = settings?.theme === "light" ? "light" : "dark";
  }, [settings?.theme]);

  useEffect(() => {
    if (!settings) {
      return;
    }
    if (first.current) {
      first.current = false;
      return;
    }
    setSaved(false);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(async () => {
      await invoke("save_settings", { settings });
      setSaved(true);
    }, 600);
  }, [settings]);

  const patch = useCallback((p: Partial<Settings>) => {
    setSettings((s) => (s ? { ...s, ...p } : s));
  }, []);

  if (!settings) {
    return <div className="loading">Carregando...</div>;
  }

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand">
          <svg className="brand-mark" width="22" height="13" viewBox="0 0 28 16" fill="currentColor" aria-hidden>
            <rect x="1" y="7.4" width="26" height="1.2" rx="0.6" />
            <rect x="4" y="6" width="2.4" height="4" rx="1.2" />
            <rect x="6.93" y="4.75" width="2.4" height="6.5" rx="1.2" />
            <rect x="9.87" y="3" width="2.4" height="10" rx="1.2" />
            <rect x="12.8" y="1.5" width="2.4" height="13" rx="1.2" />
            <rect x="15.73" y="3" width="2.4" height="10" rx="1.2" />
            <rect x="18.67" y="4.75" width="2.4" height="6.5" rx="1.2" />
            <rect x="21.6" y="6" width="2.4" height="4" rx="1.2" />
          </svg>{" "}
          Open Flow
        </div>
        <nav>
          {TABS.map((t) => (
            <button key={t} className={t === tab ? "active" : ""} onClick={() => setTab(t)}>
              {t}
            </button>
          ))}
        </nav>
        <div className="save-state">{saved ? "✓ salvo" : "salvando..."}</div>
      </aside>
      <main className="content">
        {tab === "Geral" && <GeneralTab s={settings} patch={patch} />}
        {tab === "Insights" && <InsightsTab historyEnabled={settings.history_enabled} />}
        {tab === "Perfis" && <ProfilesTab s={settings} patch={patch} />}
        {tab === "Dicionário" && <DictTab s={settings} patch={patch} />}
        {tab === "Snippets" && <SnippetsTab s={settings} patch={patch} />}
        {tab === "Histórico" && <HistoryTab s={settings} patch={patch} />}
        {tab === "Diagnóstico" && <DiagTab />}
        {tab === "Apoie" && <ApoieTab />}
      </main>
    </div>
  );
}

interface TabProps {
  s: Settings;
  patch: (p: Partial<Settings>) => void;
}

function SecretInput({
  value,
  onChange,
}: {
  value: string;
  onChange: (v: string) => void;
}) {
  const [show, setShow] = useState(false);
  return (
    <div className="secret-input">
      <input
        type={show ? "text" : "password"}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
      <button
        type="button"
        className="eye"
        title={show ? "Ocultar" : "Revelar"}
        onClick={() => setShow(!show)}
      >
        {show ? (
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94" />
            <path d="M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19" />
            <path d="M14.12 14.12a3 3 0 1 1-4.24-4.24" />
            <line x1="1" y1="1" x2="23" y2="23" />
          </svg>
        ) : (
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
            <circle cx="12" cy="12" r="3" />
          </svg>
        )}
      </button>
    </div>
  );
}

function GeneralTab({ s, patch }: TabProps) {
  const [mics, setMics] = useState<string[]>([]);
  const [autostart, setAutostart] = useState(false);
  useEffect(() => {
    invoke<string[]>("list_mics").then(setMics);
    invoke<boolean>("get_autostart").then(setAutostart);
  }, []);

  return (
    <section>
      <h1>Geral</h1>
      <label className="field">
        <span>Atalho global</span>
        <input
          list="hotkeys"
          value={s.hotkey}
          onChange={(e) => patch({ hotkey: e.target.value })}
        />
        <datalist id="hotkeys">
          {(isMac
            ? ["Ctrl+Option", "Ctrl+Shift", "Option+Shift", "Ctrl+Cmd", "F9", "F8"]
            : ["Ctrl+Win", "Ctrl+Alt", "Win+Shift", "Ctrl+Shift", "F9", "F8", "SCROLLLOCK", "PAUSE"]
          ).map((h) => (
            <option key={h} value={h} />
          ))}
        </datalist>
        <small>
          {isMac
            ? "Combos de Ctrl, Cmd, Option, Shift (ex.: Ctrl+Option) ou tecla única (F9)."
            : "Combos de Ctrl, Win, Alt, Shift (ex.: Ctrl+Win) ou tecla única (F9, PAUSE)."}
        </small>
      </label>

      <label className="field">
        <span>Cancelar ditado</span>
        <select
          value={s.cancel_key}
          onChange={(e) => patch({ cancel_key: e.target.value })}
        >
          <option value="">Desativado</option>
          <option value="ESC">Esc</option>
        </select>
        <small>Durante a gravação, essa tecla descarta o ditado sem inserir nada.</small>
      </label>

      <div className="field">
        <span>Tema</span>
        <label className="switch-row">
          <span className="switch">
            <input
              type="checkbox"
              checked={s.theme === "light"}
              onChange={(e) => patch({ theme: e.target.checked ? "light" : "dark" })}
            />
            <span className="slider" />
          </span>
          {s.theme === "light" ? "Claro" : "Escuro"}
        </label>
      </div>

      <div className="field">
        <span>Iniciar com o Windows</span>
        <label className="switch-row">
          <span className="switch">
            <input
              type="checkbox"
              checked={autostart}
              onChange={async (e) => {
                const enabled = e.target.checked;
                setAutostart(enabled);
                await invoke("set_autostart", { enabled });
              }}
            />
            <span className="slider" />
          </span>
          {autostart ? "Ativado" : "Desativado"}
        </label>
      </div>

      <div className="field">
        <span>Modo de ditado</span>
        <div className="radio-row">
          <label>
            <input
              type="radio"
              checked={s.mode !== "toggle"}
              onChange={() => patch({ mode: "hold" })}
            />
            Pressionar e segurar
          </label>
          <label>
            <input
              type="radio"
              checked={s.mode === "toggle"}
              onChange={() => patch({ mode: "toggle" })}
            />
            Apertar para iniciar/parar
          </label>
        </div>
      </div>

      <label className="field">
        <span>Microfone</span>
        <select
          value={s.mic ?? ""}
          onChange={(e) => patch({ mic: e.target.value === "" ? null : e.target.value })}
        >
          <option value="">Padrão do sistema</option>
          {mics.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
      </label>

      <label className="field">
        <span>Estilo da onda no overlay</span>
        <select value={s.wave_style} onChange={(e) => patch({ wave_style: e.target.value })}>
          <option value="tech">Tech (curvas fluidas)</option>
          <option value="classic">Clássica (barras)</option>
        </select>
        <small>Aplicado na próxima gravação.</small>
      </label>

      <h2>Transcrição — voz → texto</h2>
      <p className="hint">
        Quem converte sua fala em texto bruto. Se o provedor escolhido falhar (limite, sem
        chave, GPU ocupada), o outro assume automaticamente.
      </p>
      <label className="field">
        <span>Provedor</span>
        <select
          value={s.stt_provider}
          onChange={(e) => patch({ stt_provider: e.target.value })}
        >
          <option value="groq">Groq (nuvem — rápido, libera a GPU)</option>
          <option value="local">Local (GPU — privado, funciona offline)</option>
        </select>
        <small>
          Com Groq como provedor, o modelo local não ocupa a GPU; ele só é carregado se o
          fallback for necessário.
        </small>
      </label>
      <label className="field">
        <span>Chave da API Groq</span>
        <SecretInput
          value={s.groq_api_key ?? ""}
          onChange={(v) => patch({ groq_api_key: v || null })}
        />
        <small>Grátis (~2.000 transcrições/dia): console.groq.com/keys</small>
      </label>
      <label className="field">
        <span>Modelo de transcrição (Groq)</span>
        <input value={s.groq_model} onChange={(e) => patch({ groq_model: e.target.value })} />
      </label>
      <label className="field">
        <span>Python do modo local</span>
        <input value={s.python} onChange={(e) => patch({ python: e.target.value })} />
        <small>
          Caminho do executável Python com o faster-whisper instalado. Sem ele (e sem o arquivo
          abaixo), o modo local não funciona nem como fallback da nuvem.
        </small>
      </label>
      <label className="field">
        <span>Script do sidecar</span>
        <input value={s.sidecar} onChange={(e) => patch({ sidecar: e.target.value })} />
        <small>Caminho do stt_server.py. Confira o estado dos dois na aba Diagnóstico.</small>
      </label>
      <label className="field">
        <span>Idioma da fala</span>
        <select
          value={s.speech_lang}
          onChange={(e) => patch({ speech_lang: e.target.value })}
        >
          <option value="pt">Português (Brasil)</option>
          <option value="en">English</option>
          <option value="es">Español</option>
          <option value="auto">Detectar automaticamente</option>
        </select>
        <small>Idioma fixo transcreve melhor que a detecção automática em frases curtas.</small>
      </label>

      <h2>Reescrita — texto → texto final</h2>
      <p className="hint">
        Quem limpa a narração (hesitações, autocorreções) e aplica o perfil de escrita ativo.
      </p>
      <label className="field">
        <span>Idioma de saída</span>
        <select
          value={s.output_lang}
          onChange={(e) => patch({ output_lang: e.target.value })}
        >
          <option value="same">Mesmo da fala</option>
          <option value="pt">Português (Brasil)</option>
          <option value="en">English</option>
          <option value="es">Español</option>
        </select>
        <small>
          Diferente do idioma da fala = o texto final sai traduzido (requer a chave Gemini).
        </small>
      </label>
      <label className="field">
        <span>Modelo de reescrita (Gemini)</span>
        <input
          value={s.gemini_model}
          onChange={(e) => patch({ gemini_model: e.target.value })}
        />
      </label>
      <label className="field">
        <span>Chave da API Gemini</span>
        <SecretInput
          value={s.gemini_api_key ?? ""}
          onChange={(v) => patch({ gemini_api_key: v || null })}
        />
        <small>Grátis: aistudio.google.com/apikey</small>
      </label>
    </section>
  );
}

function ProfilesTab({ s, patch }: TabProps) {
  const update = (i: number, p: Partial<Profile>) => {
    const profiles = s.profiles.map((pr, j) => (j === i ? { ...pr, ...p } : pr));
    const renamed = p.name !== undefined && s.profiles[i].name === s.active_profile;
    patch({ profiles, ...(renamed ? { active_profile: p.name! } : {}) });
  };
  const remove = (i: number) => {
    const profiles = s.profiles.filter((_, j) => j !== i);
    patch({
      profiles,
      active_profile:
        s.profiles[i].name === s.active_profile
          ? (profiles[0]?.name ?? "")
          : s.active_profile,
    });
  };

  return (
    <section>
      <h1>Perfis de escrita</h1>
      <p className="hint">
        O perfil ativo define o estilo que a IA aplica ao texto ditado. As regras de limpeza
        (remover hesitações e autocorreções) valem para todos. O campo de texto de cada perfil
        é a instrução dada à IA — descreva ali a personalidade, o tom e o formato que o texto
        final deve ter.
      </p>
      {s.profiles.map((p, i) => (
        <div key={i} className={`card ${p.name === s.active_profile ? "card-active" : ""}`}>
          <div className="card-head">
            <label className="radio-active">
              <input
                type="radio"
                checked={p.name === s.active_profile}
                onChange={() => patch({ active_profile: p.name })}
              />
              ativo
            </label>
            <input
              className="card-title"
              value={p.name}
              onChange={(e) => update(i, { name: e.target.value })}
            />
            <button className="danger" onClick={() => remove(i)}>
              remover
            </button>
          </div>
          <textarea
            rows={2}
            value={p.style}
            placeholder='Instrução de estilo para a IA — personalidade, tom e formato do texto. Ex.: "e-mail curto e cordial, com saudação e fecho"'
            onChange={(e) => update(i, { style: e.target.value })}
          />
        </div>
      ))}
      <button
        onClick={() =>
          patch({ profiles: [...s.profiles, { name: "Novo perfil", style: "" }] })
        }
      >
        + Adicionar perfil
      </button>
    </section>
  );
}

function DictTab({ s, patch }: TabProps) {
  const update = (i: number, v: string) =>
    patch({ dictionary: s.dictionary.map((d, j) => (j === i ? v : d)) });

  return (
    <section>
      <h1>Dicionário pessoal</h1>
      <p className="hint">
        Nomes próprios e termos que devem ser grafados exatamente assim — com espaços, acentos e
        maiúsculas. Ex.: Macks Wendhell, Wispr Flow.
      </p>
      {s.dictionary.map((term, i) => (
        <div key={i} className="snippet-row">
          <input
            placeholder="termo ou nome próprio"
            value={term}
            onChange={(e) => update(i, e.target.value)}
          />
          <button
            className="danger"
            onClick={() => patch({ dictionary: s.dictionary.filter((_, j) => j !== i) })}
          >
            ×
          </button>
        </div>
      ))}
      <button onClick={() => patch({ dictionary: [...s.dictionary, ""] })}>
        + Adicionar termo
      </button>
    </section>
  );
}

function SnippetsTab({ s, patch }: TabProps) {
  const update = (i: number, p: Partial<Snippet>) =>
    patch({ snippets: s.snippets.map((sn, j) => (j === i ? { ...sn, ...p } : sn)) });

  return (
    <section>
      <h1>Snippets</h1>
      <p className="hint">
        Diga o gatilho durante o ditado e ele vira o texto completo. Ex.: "link do meu canal" →
        https://www.youtube.com/@mackswendhell
      </p>
      {s.snippets.map((sn, i) => (
        <div key={i} className="snippet-row">
          <input
            placeholder="gatilho falado"
            value={sn.trigger}
            onChange={(e) => update(i, { trigger: e.target.value })}
          />
          <span>→</span>
          <input
            placeholder="texto inserido"
            value={sn.text}
            onChange={(e) => update(i, { text: e.target.value })}
          />
          <button
            className="danger"
            onClick={() => patch({ snippets: s.snippets.filter((_, j) => j !== i) })}
          >
            ×
          </button>
        </div>
      ))}
      <button onClick={() => patch({ snippets: [...s.snippets, { trigger: "", text: "" }] })}>
        + Adicionar snippet
      </button>
    </section>
  );
}

function HistoryTab({ s, patch }: TabProps) {
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [query, setQuery] = useState("");

  const load = useCallback(
    (q: string) => invoke<HistoryEntry[]>("get_history", { query: q, limit: 50 }).then(setEntries),
    [],
  );
  // sem espera, cada tecla relê e reparseia o history.jsonl inteiro do disco
  useEffect(() => {
    const t = window.setTimeout(() => load(query), 250);
    return () => window.clearTimeout(t);
  }, [query, load]);

  return (
    <section>
      <h1>Histórico</h1>
      <div className="history-controls">
        <label className="check">
          <input
            type="checkbox"
            checked={s.history_enabled}
            onChange={(e) => patch({ history_enabled: e.target.checked })}
          />
          Salvar histórico localmente
        </label>
        <button
          className="danger"
          onClick={async () => {
            await invoke("clear_history");
            load(query);
          }}
        >
          Apagar tudo
        </button>
      </div>
      <input
        className="search"
        placeholder="Buscar nas transcrições..."
        value={query}
        onChange={(e) => setQuery(e.target.value)}
      />
      {entries.length === 0 && <p className="hint">Nenhuma transcrição.</p>}
      {entries.map((e, i) => (
        <div key={i} className="card">
          <div className="entry-meta">
            {new Date(e.ts * 1000).toLocaleString("pt-BR")} · {e.profile} ·{" "}
            {e.total_secs.toFixed(1)}s
          </div>
          <div className="entry-final">{e.final}</div>
          <details>
            <summary>transcrição bruta</summary>
            <div className="entry-raw">{e.raw}</div>
          </details>
        </div>
      ))}
    </section>
  );
}

function countWords(text: string): number {
  return text.trim().split(/\s+/).filter(Boolean).length;
}

function dayKey(d: Date): string {
  return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
}

function InsightsTab({ historyEnabled }: { historyEnabled: boolean }) {
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  useEffect(() => {
    invoke<HistoryEntry[]>("get_history", { limit: 5000 }).then(setEntries);
  }, []);

  const totalWords = entries.reduce((a, e) => a + countWords(e.final), 0);
  const audioSecs = entries.reduce((a, e) => a + e.audio_secs, 0);
  const wpm = audioSecs > 0 ? totalWords / (audioSecs / 60) : 0;
  const avgLatency =
    entries.length > 0 ? entries.reduce((a, e) => a + e.total_secs, 0) / entries.length : 0;

  const byDay = new Map<string, number>();
  for (const e of entries) {
    const k = dayKey(new Date(e.ts * 1000));
    byDay.set(k, (byDay.get(k) ?? 0) + 1);
  }

  let streak = 0;
  const cursor = new Date();
  if (!byDay.has(dayKey(cursor))) {
    cursor.setDate(cursor.getDate() - 1);
  }
  while (byDay.has(dayKey(cursor))) {
    streak++;
    cursor.setDate(cursor.getDate() - 1);
  }

  const WEEKS = 20;
  const today = new Date();
  const start = new Date(today);
  start.setDate(start.getDate() - start.getDay() - (WEEKS - 1) * 7);
  const cells: { count: number; future: boolean }[] = [];
  for (let w = 0; w < WEEKS; w++) {
    for (let d = 0; d < 7; d++) {
      const day = new Date(start);
      day.setDate(start.getDate() + w * 7 + d);
      cells.push({ count: byDay.get(dayKey(day)) ?? 0, future: day > today });
    }
  }
  const lvl = (c: number) => (c === 0 ? 0 : c < 3 ? 1 : c < 8 ? 2 : 3);

  const byProfile = new Map<string, { words: number; count: number }>();
  for (const e of entries) {
    const p = byProfile.get(e.profile) ?? { words: 0, count: 0 };
    p.words += countWords(e.final);
    p.count += 1;
    byProfile.set(e.profile, p);
  }

  return (
    <section>
      <h1>Insights</h1>
      {!historyEnabled && (
        <p className="hint">O histórico está desativado — os insights só contam ditados gravados nele.</p>
      )}
      <div className="diag-grid">
        <div className="card stat">
          <span className="stat-big">{Math.round(wpm)}</span>
          <span className="stat-label">palavras por minuto</span>
        </div>
        <div className="card stat">
          <span className="stat-big">{totalWords}</span>
          <span className="stat-label">palavras ditadas</span>
        </div>
        <div className="card stat">
          <span className="stat-big">{entries.length}</span>
          <span className="stat-label">ditados</span>
        </div>
        <div className="card stat">
          <span className="stat-big">{avgLatency.toFixed(1)}s</span>
          <span className="stat-label">latência média</span>
        </div>
      </div>
      <div className="card streak-card">
        <b>
          {streak > 0 ? `${streak} ${streak === 1 ? "dia seguido" : "dias seguidos"}` : "sem sequência ativa"}
        </b>
        <div className="heatmap">
          {cells.map((c, i) => (
            <span
              key={i}
              className={`hm hm-${lvl(c.count)} ${c.future ? "hm-future" : ""}`}
              title={`${c.count} ditado(s)`}
            />
          ))}
        </div>
      </div>
      {byProfile.size > 0 && (
        <div className="card">
          <b>Por perfil</b>
          {[...byProfile.entries()]
            .sort((a, b) => b[1].words - a[1].words)
            .map(([name, p]) => (
              <div key={name} className="profile-stat">
                {name}: {p.words} palavras em {p.count} ditado(s)
              </div>
            ))}
        </div>
      )}
    </section>
  );
}

function DiagTab() {
  const [d, setD] = useState<Diagnostics | null>(null);
  useEffect(() => {
    invoke<Diagnostics>("get_diagnostics").then(setD);
  }, []);

  if (!d) {
    return (
      <section>
        <h1>Diagnóstico</h1>
        <p className="hint">Verificando...</p>
      </section>
    );
  }
  const avg =
    d.recent.length > 0
      ? d.recent.reduce((a, e) => a + e.total_secs, 0) / d.recent.length
      : null;

  return (
    <section>
      <h1>Diagnóstico</h1>
      <div className="diag-grid">
        <div className="card">
          <b>Transcrição (provedor: {d.stt_provider === "groq" ? "Groq nuvem" : "Local GPU"})</b>
          <div>Groq: {d.groq_key_set ? "● chave configurada" : "○ sem chave"}</div>
          <div>
            Local:{" "}
            {d.sidecar_ok
              ? `● quente (${d.sidecar_device})`
              : d.sidecar_configured
                ? "○ descarregado (sob demanda)"
                : "○ não configurado"}
          </div>
          {!d.sidecar_configured && (
            <div className="hint">
              Sem Python e sidecar válidos o modo local nunca sobe — nem como fallback. Configure
              os caminhos em Geral.
            </div>
          )}
        </div>
        <div className="card">
          <b>Microfone padrão</b>
          <div>{d.default_mic}</div>
        </div>
        <div className="card">
          <b>Gemini</b>
          <div>
            {d.gemini_key_set ? "● chave configurada" : "○ sem chave"} · {d.gemini_model}
          </div>
        </div>
        <div className="card">
          <b>Latência média (últimos {d.recent.length})</b>
          <div>{avg !== null ? `${avg.toFixed(2)}s do soltar ao texto` : "sem dados"}</div>
        </div>
      </div>
    </section>
  );
}

const PIX_KEY = "mackspls@hotmail.com";

function ApoieTab() {
  const [copied, setCopied] = useState(false);
  return (
    <section>
      <h1>Apoie o Open Flow</h1>
      <p className="hint">
        O Open Flow é gratuito e open source, criado para tornar a escrita por voz mais
        acessível, prática e inteligente.
      </p>
      <div className="diag-grid apoie">
        <div className="card">
          <b>Quem faz</b>
          <img src={canalImg} alt="Canal Macks Wendhell — Inteligência Aplicada" />
          <p>
            Conteúdos práticos sobre IA, trabalho e produtividade no canal do criador do app.
          </p>
          <button onClick={() => openUrl("https://www.youtube.com/@mackswendhell")}>
            Conhecer o canal
          </button>
        </div>
        <div className="card">
          <b>Apoio voluntário</b>
          <img src={pixQrImg} alt="QR code Pix" className="apoie-qr" />
          <p>
            Se o app facilita seu trabalho, você pode apoiar o desenvolvimento com uma
            contribuição via Pix — qualquer valor ajuda a manter o projeto ativo. O apoio é
            totalmente opcional: o Open Flow continuará gratuito e aberto para todos.
          </p>
          <button
            onClick={() =>
              navigator.clipboard.writeText(PIX_KEY).then(() => {
                setCopied(true);
                window.setTimeout(() => setCopied(false), 2000);
              })
            }
          >
            {copied ? "✓ chave copiada" : `Copiar chave Pix (${PIX_KEY})`}
          </button>
        </div>
      </div>
    </section>
  );
}

export default App;
