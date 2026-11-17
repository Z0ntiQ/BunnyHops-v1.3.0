# Твоя задача (d5_tests)

Напиши файл `tests/api_matrix_mock.py` полностью, по «Контракту E» из SPEC.
- Стиль, хелперы HTTP и формат вывода (PASS/FAIL строки, exit code) — дословно по приложенным `matrix_head.txt` / `matrix_tail.txt`.
- Только stdlib. Echo-сервер — `http.server.ThreadingHTTPServer` в потоке.
- Ответ — один ```python блок.

=== matrix_head.txt ===
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



=== matrix_tail.txt ===
st, hdr, b = call("GET", P("https://zontiq.ru/"), timeout=25)
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
st, hdr, b = call("GET", "/screenshot?match=localhost:3000", timeout=40)
is_png = b[:8] == b"\x89PNG\r\n\x1a\n"
check("screenshot PNG", st == 200 and is_png, f"code={st} type={hdr.get('content-type')}")

# ---------- report ----------
srv.shutdown()
fails = [r for r in results if not r[1]]
w = max(len(r[0]) for r in results) + 2
for name, ok, detail in results:
    print(("PASS  " if ok else "FAIL  ") + name.ljust(w) + (detail or ""))
print(f"\n=== {len(results)-len(fails)}/{len(results)} passed, {len(fails)} failed ===")
sys.exit(1 if fails else 0)
