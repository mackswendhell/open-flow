import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const W = 119;
const H = 31;
const HIST = 48;
// abaixo disto é ruído de fundo, não fala — mesmo limiar que o Rust usa para
// descartar gravações sem voz (peak_rms < 0.006)
const SILENCE = 0.006;
// piso do normalizador: se ele puder cair até o nível do ruído, o silêncio vira
// onda cheia porque o próprio ruído passa a ser o "máximo" da escala
const PEAK_FLOOR = 0.02;

type State = "recording" | "processing" | "error";

export default function Overlay() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const level = useRef(0);
  const style = useRef("tech");
  const stateRef = useRef<State>("recording");
  const reset = useRef(false);
  const [state, setState] = useState<State>("recording");
  const [message, setMessage] = useState("");

  useEffect(() => {
    for (const el of [document.documentElement, document.body]) {
      el.style.background = "transparent";
      el.style.margin = "0";
      el.style.padding = "0";
      el.style.overflow = "hidden";
    }
    invoke<{ wave_style: string }>("get_settings").then((s) => {
      style.current = s.wave_style;
    });
    const apply = (s: State, msg = "") => {
      stateRef.current = s;
      setState(s);
      setMessage(msg);
    };
    const unlisten = listen<number>("level", (e) => {
      level.current = e.payload;
    });
    const unShow = listen<string>("overlay_show", (e) => {
      style.current = e.payload;
      level.current = 0;
      reset.current = true;
      apply("recording");
    });
    const unStatus = listen<{ state: State; message: string }>("overlay_status", (e) => {
      apply(e.payload.state, e.payload.message);
    });

    const canvas = canvasRef.current!;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = W * dpr;
    canvas.height = H * dpr;
    const ctx = canvas.getContext("2d")!;
    ctx.scale(dpr, dpr);

    const hist = new Array<number>(HIST).fill(0);
    let env = 0;
    let phase = 0;
    let raf = 0;
    let peak = PEAK_FLOOR;
    const mid = H / 2;

    const drawTech = () => {
      ctx.lineCap = "round";
      for (let k = 0; k < 4; k++) {
        ctx.beginPath();
        ctx.strokeStyle = `rgba(255, 255, 255, ${0.95 - k * 0.22})`;
        ctx.lineWidth = k === 0 ? 1.5 : 1;
        for (let x = 0; x <= W; x += 2) {
          const t = x / W;
          const envx = hist[Math.floor(t * (HIST - 1))];
          const edge = Math.sin(Math.PI * t);
          const amp = envx * (mid - 3) * edge * (1 - k * 0.18);
          const y = mid + Math.sin(x * 0.11 + phase + k * 1.7) * amp;
          if (x === 0) {
            ctx.moveTo(x, y);
          } else {
            ctx.lineTo(x, y);
          }
        }
        ctx.stroke();
      }
    };

    const drawClassic = () => {
      ctx.fillStyle = "rgba(255, 255, 255, 0.95)";
      ctx.strokeStyle = "rgba(255, 255, 255, 0.9)";
      ctx.lineWidth = 1.2;
      ctx.beginPath();
      ctx.moveTo(0, mid);
      ctx.lineTo(W, mid);
      ctx.stroke();
      const nb = 23;
      const x0 = W * 0.09;
      const step = (W * 0.82) / (nb - 1);
      const bw = 2.6;
      for (let i = 0; i < nb; i++) {
        const v = hist[Math.floor((i / (nb - 1)) * (HIST - 1))];
        const edge = 0.45 + 0.55 * Math.sin(Math.PI * (i / (nb - 1)));
        const h = Math.max(2.5, v * (H - 5) * edge);
        const x = x0 + i * step;
        ctx.beginPath();
        ctx.roundRect(x - bw / 2, mid - h / 2, bw, h, bw / 2);
        ctx.fill();
      }
    };

    // nada de onda aqui: qualquer coisa que oscile como voz faz parecer que o
    // microfone continua aberto depois de soltar o atalho
    const drawProcessing = () => {
      const t = performance.now() / 1000;
      const gap = 11;
      for (let i = 0; i < 3; i++) {
        const wave = 0.5 + 0.5 * Math.sin(t * 3.4 - i * 0.6);
        ctx.fillStyle = `rgba(255, 255, 255, ${0.3 + 0.6 * wave})`;
        ctx.beginPath();
        ctx.arc(W / 2 + (i - 1) * gap, mid, 2.1 + 1.1 * wave, 0, Math.PI * 2);
        ctx.fill();
      }
    };

    const draw = () => {
      if (reset.current) {
        reset.current = false;
        hist.fill(0);
        env = 0;
        // peak NÃO é zerado: ele guarda a calibração da voz entre gravações.
        // Zerá-lo faz o ruído de fundo saturar a escala e a onda abrir sozinha
        // no silêncio, que é o bug que já tinha sido corrigido uma vez.
      }
      ctx.clearRect(0, 0, W, H);
      ctx.shadowColor = "rgba(0, 0, 0, 0.9)";
      ctx.shadowBlur = 5;
      ctx.shadowOffsetY = 1.5;

      if (stateRef.current === "processing") {
        drawProcessing();
        raf = requestAnimationFrame(draw);
        return;
      }

      peak = Math.max(peak * 0.997, level.current, PEAK_FLOOR);
      const target =
        level.current < SILENCE ? 0 : Math.min(1, Math.pow(level.current / peak, 0.9));
      env = target > env ? env + (target - env) * 0.5 : env * 0.9;
      hist.push(env);
      hist.shift();
      phase += 0.22 + env * 0.35;

      if (style.current === "classic") {
        drawClassic();
      } else {
        drawTech();
      }
      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);

    return () => {
      cancelAnimationFrame(raf);
      unlisten.then((f) => f());
      unShow.then((f) => f());
      unStatus.then((f) => f());
    };
  }, []);

  return (
    <div
      style={{
        width: "100vw",
        height: "100vh",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <canvas
        ref={canvasRef}
        style={{
          display: state === "error" ? "none" : "block",
          width: `${W}px`,
          height: `${H}px`,
        }}
      />
      {state === "error" && (
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "7px",
            maxWidth: "100%",
            padding: "6px 13px",
            borderRadius: "999px",
            background: "rgba(20, 20, 22, 0.94)",
            border: "1px solid rgba(255, 110, 110, 0.55)",
            boxShadow: "0 2px 10px rgba(0, 0, 0, 0.55)",
            color: "#ffd9d9",
            font: "500 12.5px system-ui, -apple-system, Segoe UI, sans-serif",
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
          }}
        >
          <span style={{ color: "#ff6b6b", fontSize: "14px", lineHeight: 1 }}>●</span>
          {message}
        </div>
      )}
    </div>
  );
}
