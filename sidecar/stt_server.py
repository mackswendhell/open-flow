"""Sidecar STT do Open Flow: servidor HTTP local com faster-whisper residente na GPU.

POST /stt (corpo = WAV bytes) -> {"text": "..."}
GET  /health -> 200 quando o modelo está quente.
"""

import ctypes
import io
import json
import os
import struct
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path


def _watch_parent():
    """Encerra este processo quando o app (pai) morrer — evita sidecars órfãos."""
    ppid = int(os.environ.get("OPENFLOW_PARENT_PID", "0"))
    if not ppid:
        return
    if sys.platform == "win32":
        SYNCHRONIZE = 0x00100000
        h = ctypes.windll.kernel32.OpenProcess(SYNCHRONIZE, False, ppid)
        if not h:
            os._exit(0)
        ctypes.windll.kernel32.WaitForSingleObject(h, 0xFFFFFFFF)
        os._exit(0)
    # POSIX: sinal 0 só checa existência; reparentado ao launchd (ppid 1) = pai morreu
    while True:
        if os.getppid() == 1:
            os._exit(0)
        try:
            os.kill(ppid, 0)
        except OSError:
            os._exit(0)
        time.sleep(2)


threading.Thread(target=_watch_parent, daemon=True).start()

VENV_NVIDIA = Path(sys.prefix) / "Lib" / "site-packages" / "nvidia"
for sub in ("cublas", "cudnn"):
    d = VENV_NVIDIA / sub / "bin"
    if d.is_dir():
        os.add_dll_directory(str(d))
        os.environ["PATH"] = str(d) + os.pathsep + os.environ["PATH"]

from faster_whisper import WhisperModel

PORT = int(os.environ.get("OPENFLOW_STT_PORT", "17765"))

print("[stt] carregando large-v3-turbo...", flush=True)
t0 = time.perf_counter()
try:
    MODEL = WhisperModel("large-v3-turbo", device="cuda", compute_type="float16")
    DEVICE = "cuda/float16"
except Exception as e:
    print(f"[stt] CUDA indisponível ({e}); usando CPU int8", flush=True)
    MODEL = WhisperModel("large-v3-turbo", device="cpu", compute_type="int8")
    DEVICE = "cpu/int8"


def _silence_wav(seconds: float = 1.0, rate: int = 16000) -> bytes:
    n = int(seconds * rate)
    data = b"\x00\x00" * n
    hdr = struct.pack("<4sI4s4sIHHIIHH4sI", b"RIFF", 36 + len(data), b"WAVE", b"fmt ",
                      16, 1, 1, rate, rate * 2, 2, 16, b"data", len(data))
    return hdr + data


list(MODEL.transcribe(io.BytesIO(_silence_wav(30.0)), language="pt", beam_size=1)[0])
print(f"[stt] pronto em {time.perf_counter() - t0:.1f}s ({DEVICE}) na porta {PORT}", flush=True)


class Handler(BaseHTTPRequestHandler):
    def _reply(self, code: int, body: bytes) -> None:
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        self._reply(200, json.dumps({"status": "ok", "device": DEVICE}).encode())

    def do_POST(self):
        from urllib.parse import parse_qs, urlparse

        wav = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        lang = parse_qs(urlparse(self.path).query).get("lang", ["pt"])[0]
        kwargs = {} if lang == "auto" else {"language": lang}
        t = time.perf_counter()
        segments, info = MODEL.transcribe(io.BytesIO(wav), beam_size=1, vad_filter=True, **kwargs)
        text = " ".join(s.text.strip() for s in segments)
        dt = time.perf_counter() - t
        print(f"[stt] {info.duration:.1f}s de áudio em {dt:.2f}s: {text[:80]}", flush=True)
        self._reply(200, json.dumps({"text": text, "stt_seconds": round(dt, 2)}).encode())

    def log_message(self, *a):
        pass


class Server(HTTPServer):
    allow_reuse_address = False  # porta ocupada = outro sidecar vivo; falha em vez de duplicar


Server(("127.0.0.1", PORT), Handler).serve_forever()
