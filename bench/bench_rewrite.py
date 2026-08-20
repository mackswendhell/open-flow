#!/usr/bin/env python3
"""Replica ditados reais do history.jsonl contra a API de reescrita e mede
latencia, timeout e taxa de sucesso. Sem dependencias: so stdlib.

Objetivo: descobrir se o hang de 30s do Gemini e travamento (retry resolve)
ou lentidao real (retry so dobra a espera).
"""
import json, os, re, ssl, sys, time, random, argparse, urllib.request, urllib.error
from statistics import median

APP = os.path.expanduser('~/Library/Application Support/OpenFlow')
LIB = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                   'app', 'src-tauri', 'src', 'lib.rs')


def carrega_regras():
    """Extrai o const RULES do lib.rs para o prompt bater com o do app."""
    src = open(LIB, encoding='utf-8').read()
    m = re.search(r'const RULES: &str = "(.*?)";', src, re.S)
    if not m:
        sys.exit('RULES nao encontrado em lib.rs')
    # no Rust, `\` no fim da linha engole a quebra e a indentacao seguinte
    s = re.sub(r'\\\n\s*', '', m.group(1))
    return s.replace('\\n', '\n').replace('\\"', '"').replace('\\\\', '\\')


def build_prompt(cfg, regras, perfil):
    """Espelha build_prompt() do lib.rs."""
    estilo = next((p['style'] for p in cfg.get('profiles', []) if p['name'] == perfil),
                  'texto natural corrigido, mantendo o tom do falante.')
    p = regras
    dic = [t.strip() for t in cfg.get('dictionary', []) if t.strip()]
    if dic:
        p += f"6. Grafias obrigatorias quando essas palavras aparecerem: {', '.join(dic)}.\n"
    p += "Responda SOMENTE com o texto final, sem comentarios.\n"
    p += f"Estilo: {estilo}\n\nTranscricao:\n"
    return p


def chama_gemini(model, key, prompt, timeout):
    """Devolve (status, texto, segundos). status: ok | timeout | http_NNN | rede."""
    url = f"https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={key}"
    body = json.dumps({"contents": [{"parts": [{"text": prompt}]}],
                       "generationConfig": {"temperature": 0.2}}).encode()
    req = urllib.request.Request(url, data=body, headers={'Content-Type': 'application/json'})
    t0 = time.time()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            v = json.loads(r.read())
        partes = v.get('candidates', [{}])[0].get('content', {}).get('parts', [])
        txt = ''.join(x.get('text', '') for x in partes).strip()
        return ('ok' if txt else 'vazio'), txt, time.time() - t0
    except urllib.error.HTTPError as e:
        return f'http_{e.code}', e.read()[:200].decode('utf-8', 'replace'), time.time() - t0
    except TimeoutError:
        return 'timeout', '', time.time() - t0
    except Exception as e:
        cls = 'timeout' if 'timed out' in str(e).lower() else 'rede'
        return cls, str(e)[:200], time.time() - t0


def amostra(n, seed=7):
    """Amostra estratificada por tamanho da fala, para nao viciar em ditado curto."""
    rs = []
    for l in open(os.path.join(APP, 'history.jsonl'), encoding='utf-8'):
        l = l.strip()
        if not l:
            continue
        try:
            d = json.loads(l)
        except Exception:
            continue
        if d.get('raw', '').strip():
            rs.append(d)
    faixas = [(0, 200), (200, 600), (600, 1500), (1500, 10**9)]
    rnd = random.Random(seed)
    out = []
    for lo, hi in faixas:
        g = [x for x in rs if lo <= len(x['raw']) < hi]
        if g:
            out += rnd.sample(g, min(len(g), max(1, n // len(faixas))))
    rnd.shuffle(out)
    return out[:n]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--n', type=int, default=40)
    ap.add_argument('--timeout', type=float, default=30.0)
    ap.add_argument('--retry-apos', type=float, default=0,
                    help='se >0, corta em N s e tenta de novo (mede a estrategia do fix)')
    ap.add_argument('--pausa', type=float, default=1.5, help='pausa entre chamadas (poupa cota)')
    ap.add_argument('--modelo', default=None)
    ap.add_argument('--saida', default=None)
    a = ap.parse_args()

    cfg = json.load(open(os.path.join(APP, 'settings.json'), encoding='utf-8'))
    key = cfg.get('gemini_api_key') or os.environ.get('GEMINI_API_KEY')
    if not key:
        sys.exit('sem chave do Gemini')
    modelo = a.modelo or cfg.get('gemini_model', 'gemini-3.5-flash-lite')
    regras = carrega_regras()

    casos = amostra(a.n)
    print(f"modelo={modelo}  n={len(casos)}  timeout={a.timeout}s  "
          f"retry_apos={a.retry_apos or 'nao'}  pausa={a.pausa}s", flush=True)

    linhas = []
    for i, c in enumerate(casos, 1):
        perfil = c.get('profile', cfg.get('active_profile'))
        prompt = build_prompt(cfg, regras, perfil) + c['raw']
        if a.retry_apos:
            st, txt, dt = chama_gemini(modelo, key, prompt, a.retry_apos)
            tentativas = 1
            if st in ('timeout', 'rede'):
                st2, txt2, dt2 = chama_gemini(modelo, key, prompt, a.retry_apos)
                tentativas = 2
                st, txt, dt = st2, txt2, dt + dt2
        else:
            st, txt, dt = chama_gemini(modelo, key, prompt, a.timeout)
            tentativas = 1
        linhas.append({'i': i, 'status': st, 'secs': round(dt, 3), 'tentativas': tentativas,
                       'raw_ch': len(c['raw']), 'out_ch': len(txt), 'perfil': perfil,
                       'ts_orig': c.get('ts')})
        print(f"  [{i:>3}/{len(casos)}] {st:<9} {dt:6.2f}s  entrada={len(c['raw']):>5}ch  "
              f"saida={len(txt):>5}ch  tent={tentativas}", flush=True)
        time.sleep(a.pausa)

    oks = [x['secs'] for x in linhas if x['status'] == 'ok']
    falhas = [x for x in linhas if x['status'] != 'ok']
    print("\n===== RESUMO =====")
    print(f"sucesso: {len(oks)}/{len(linhas)}  ({100*len(oks)/len(linhas):.1f}%)")
    if oks:
        v = sorted(oks)
        print(f"latencia ok -> mediana {median(v):.2f}s  p90 {v[int(len(v)*.9)]:.2f}s  max {v[-1]:.2f}s")
    from collections import Counter
    for st, qtd in Counter(x['status'] for x in linhas).most_common():
        print(f"  {st}: {qtd}")
    if falhas:
        print("falhas:", [(x['i'], x['status'], x['secs']) for x in falhas])

    if a.saida:
        json.dump(linhas, open(a.saida, 'w'), indent=1)
        print(f"\ndetalhe salvo em {a.saida}")


if __name__ == '__main__':
    main()
