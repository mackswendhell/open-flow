import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const W = 119;
const H = 31;
const HIST = 48;

export default function Overlay() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const level = useRef(0);
  const style = useRef("tech");

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
    const unlisten = listen<number>("level", (e) => {
      level.current = e.payload;
    });
    const unShow = listen<string>("overlay_show", (e) => {
      style.current = e.payload;
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
    let peak = 0.004;
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

    const draw = () => {
      peak = Math.max(peak * 0.997, level.current, 0.004);
      const target =
        level.current < 0.0015
          ? 0
          : Math.min(1, Math.pow(level.current / peak, 0.9));
      env = target > env ? env + (target - env) * 0.5 : env * 0.9;
      hist.push(env);
      hist.shift();
      phase += 0.22 + env * 0.35;

      ctx.clearRect(0, 0, W, H);
      ctx.shadowColor = "rgba(0, 0, 0, 0.9)";
      ctx.shadowBlur = 5;
      ctx.shadowOffsetY = 1.5;
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
    };
  }, []);

  return (
    <canvas
      ref={canvasRef}
      style={{ display: "block", width: `${W}px`, height: `${H}px` }}
    />
  );
}
