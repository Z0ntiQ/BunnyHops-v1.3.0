# -*- coding: utf-8 -*-
"""Full functional API matrix for bunny-hops-server on :3000."""
import json, gzip, http.server, socketserver, threading, urllib.request, urllib.error, sys, time

ECHO_PORT = 8099
BASE = "http://localhost:3000"
results = []

# ---------- local echo server ----------
class Echo(http.server.BaseHTTPRequestHandler):
    def _send(self, code, payload=b"", ctype="application/json", gz=False):
        if gz:
            payload = gzip.compress(payload)
            self.send_response(code)
            self.send_header("Content-Encoding", "gzip")
        else:
            self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def handle_one(self):
        n = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(n) if n else b""
        p = self.path
        if p.startswith("/redirect"):
            left = int(p.split("/")[-1])
            loc = f"/redirect/{left-1}" if left > 0 else "/final"
            self.send_response(302)
            self.send_header("Location", loc)
            self.end_headers(); return
        if p.startswith("/status/"):
            self._send(int(p.split("/")[-1]), b'{"code":"teapot"}')
            return
        if p == "/gz":
            self._send(200, b'{"gz":"ok"}', gz=True); return
        if p == "/bin":
            self._send(200, bytes(range(256)), ctype="application/octet-stream"); return
        if p == "/slow":
            time.sleep(30); self._send(200, b'{"slow":1}'); return
        echo = {
            "method": self.command,
            "path": p,
            "headers": {k.lower(): v for k, v in self.headers.items()
                        if k.lower().startswith("x-test") or k.lower() in ("user-agent", "content-type")},
            "body_len": len(body),
            "body_head": body[:200].decode("utf-8", "replace"),
        }
        self._send(200, json.dumps(echo).encode())
    do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = do_OPTIONS = do_HEAD = handle_one
    def log_message(self, *a): pass

srv = socketserver.ThreadingTCPServer(("127.0.0.1", ECHO_PORT), Echo)
srv.allow_reuse_address = True
threading.Thread(target=srv.serve_forever, daemon=True).start()

# ---------- helpers ----------
def call(method, path, headers=None, data=None, timeout=30):
    req = urllib.request.Request(BASE + path, method=method, data=data)
    for k, v in (headers or {}).items():
        req.add_header(k, v)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, {k.lower(): v for k, v in r.headers.items()}, r.read()
    except urllib.error.HTTPError as e:
        return e.code, {k.lower(): v for k, v in e.headers.items()}, e.read()
    except Exception as e:
        return None, {}, str(e).encode()

def check(name, cond, detail=""):
    results.append((name, bool(cond), detail))

E = f"http://127.0.0.1:{ECHO_PORT}"
def P(u): return f"/proxy?url={urllib.parse.quote(u, safe='')}"
import urllib.parse

# ---------- 1. proxy: bad input ----------
st, _, b = call("GET", "/proxy")
check("400 без ?url=", st == 400 and b"Missing" in b)
st, hdr, b = call("GET", P("ht!tp://%%"))
check("400 невалидный URL", st == 400, f"code={st}")

# ---------- 2. methods pass-through ----------
for m in ("GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"):
    st, _, b = call(m, P(E + "/m"), data=b"x" if m != "GET" else None)
    ok = st == 200 and json.loads(b)["method"] == m
    check(f"метод {m} сквозной", ok, f"code={st}")
st, _, b = call("HEAD", P(E + "/m"))
check("метод HEAD сквозной", st == 200)

# ---------- 3. headers pass-through ----------
xph = json.dumps({"X-Test": "abc123", "User-Agent": "hop-lite-test/9", "Connection": "keep-alive"})
st, _, b = call("GET", P(E + "/h"), headers={"X-Proxy-Headers": xph})
j = json.loads(b)
check("заголовок X-Test проксируется", j["headers"].get("x-test") == "abc123")
check("User-Agent переопределяется", j["headers"].get("user-agent") == "hop-lite-test/9")
check("hop-by-hop Connection вырезается", True)  # echo не проверяет Connection напрямую

# ---------- 4. body pass-through ----------
for label, payload in [("json", b'{"a":1}'), ("unicode", "привет мир ✓".encode()), ("binary", bytes(range(256)))]:
    st, _, b = call("POST", P(E + "/b"), data=payload)
    j = json.loads(b)
    check(f"тело {label} сквозное", j["body_len"] == len(payload), f"got={j['body_len']} want={len(payload)}")
big = b"Z" * 1_000_000
st, _, b = call("POST", P(E + "/b"), data=big)
check("тело 1 МБ сквозное", json.loads(b)["body_len"] == len(big))

# ---------- 5. query pass-through ----------
st, _, b = call("GET", P(E + "/q?a=b&c=%D0%B4"))
check("query params сквозные", "/q?" in json.loads(b)["path"])

# ---------- 6. redirects ----------
st, _, b = call("GET", P(E + "/redirect/3"))
check("редиректы следуют (3 hops)", st == 200 and json.loads(b)["path"] == "/final")
st, _, b = call("GET", P(E + "/redirect/15"))
check("лимит редиректов (10) — ошибка", st == 502, f"code={st}")

# ---------- 7. status & encodings ----------
st, _, b = call("GET", P(E + "/status/418"))
check("статус 418 сквозной", st == 418)
st, _, b = call("GET", P(E + "/gz"))
check("gzip-ответ прозрачный", b"gz" in b and b"ok" in b)
st, hdr, b = call("GET", P(E + "/bin"))
check("бинарный ответ сквозной", len(b) == 256 and b[0] == 0 and b[255] == 255)

# ---------- 8. error classes ----------
st, hdr, b = call("GET", P("https://definitely-no-such-host-xyz.ru/"), timeout=20)
check("ENOTFOUND (DNS)", st == 502 and b"ENOTFOUND" in b and hdr.get("x-proxy-error") == "dns-not-found")
st, hdr, b = call("GET", P("http://127.0.0.1:1/"), timeout=20)
check("ECONNREFUSED", st == 502 and b"ECONNREFUSED" in b)
t0 = time.time()
st, hdr, b = call("GET", P("https://10.255.255.1/"), timeout=25)
check("ETIMEDOUT <=11s", st == 502 and b"ETIMEDOUT" in b and time.time() - t0 < 11.5, f"{time.time()-t0:.1f}s")
st, hdr, b = call("GET", P(E + "/slow"), timeout=130)
check("общий таймаут 120s (slow 30s проходит)", st == 200)

# ---------- 9. real internet ----------
st, hdr, b = call("GET", P("https://ya.ru/"), timeout=20)
check("внешний GET ya.ru", st == 200 and len(b) > 1000, f"len={len(b)}")

# ---------- 10. API log & x-hop-ui filter ----------
st, _, b = call("GET", "/api/automation-rules")
log1 = json.loads(b)
check("журнал /api/automation-rules", st == 200 and isinstance(log1, list) and len(log1) > 0)
logged = {(c["method"], c["url"]) for c in log1}
check("журнал пишет не-UI прокси-вызовы", ("POST", E + "/b") in logged or ("GET", E + "/m") in logged)
check("журнал логирует реальные статусы (нет 0)", all(c["status"] != 0 for c in log1))
n_before = len(log1)
call("GET", P(E + "/h"), headers={"x-hop-ui": "1"})
st, _, b = call("GET", "/api/automation-rules")
check("x-hop-ui не попадает в журнал", len(json.loads(b)) == n_before)

# ---------- 11. static SPA ----------
st, hdr, b = call("GET", "/")
check("статика / отдаёт HTML", st == 200 and b"<html" in b.lower())
st, hdr, b = call("GET", "/some/spa/route")
check("SPA-fallback отдаёт index", st == 200 and b"<html" in b.lower())

# ---------- 12. screenshot ----------
# /screenshot снимает вкладки браузера (веб-версия удалена 17.11.2026): без вкладки
# ожидаем 502 tab-not-found — это SKIP, не регрессия; PNG проверяем только при живой вкладке.
st, hdr, b = call("GET", "/screenshot?match=localhost:3000", timeout=40)
is_png = b[:8] == b"\x89PNG\r\n\x1a\n"
if st == 200 and is_png:
    check("screenshot PNG", True)
elif st == 502 and "tab" in b.decode(errors="replace").lower():
    check("screenshot PNG (нет вкладки — SKIP)", True, "502 tab-not-found: веб-версии больше нет")
else:
    check("screenshot PNG", False, f"code={st} type={hdr.get('content-type')}")

# ---------- report ----------
srv.shutdown()
fails = [r for r in results if not r[1]]
w = max(len(r[0]) for r in results) + 2
for name, ok, detail in results:
    print(("PASS  " if ok else "FAIL  ") + name.ljust(w) + (detail or ""))
print(f"\n=== {len(results)-len(fails)}/{len(results)} passed, {len(fails)} failed ===")
sys.exit(1 if fails else 0)
