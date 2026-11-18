# -*- coding: utf-8 -*-
"""Functional API matrix for BunnyProx mock gateway."""
import json, os, socket, sys, threading, time, http.server, urllib.request, urllib.error

ECHO_PORT = 3198
LISTEN_PORT = 3199
PROFILE_NAME = "pytest-mock"
BASE = os.environ.get("HOP_URL", "http://127.0.0.1:3000").rstrip("/")
GATEWAY = f"http://127.0.0.1:{LISTEN_PORT}"
results = []

def _enable_debug_api():
    # v1.2.0: /debug/* закрыты гейтом; тесты работают по полной поверхности.
    req = urllib.request.Request(BASE + "/api/debug-mode", data=b'{"on": true}',
                                 headers={"Content-Type": "application/json"}, method="POST")
    try:
        urllib.request.urlopen(req, timeout=5)
    except Exception:
        pass

_enable_debug_api()
srv = None

# ---------- local echo server ----------
class Echo(http.server.BaseHTTPRequestHandler):
    def _send(self, code, payload=b"", ctype="application/json"):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def handle_one(self):
        n = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(n) if n else b""
        payload = {
            "method": self.command,
            "path": self.path,
            "body": body.decode("utf-8", "replace"),
            "status": 201,
        }
        self._send(201, json.dumps(payload).encode())
    do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = do_OPTIONS = handle_one
    def log_message(self, *a): pass

class EchoServer(http.server.ThreadingHTTPServer):
    allow_reuse_address = True
    daemon_threads = True

# ---------- helpers ----------
def call_url(url, method="GET", data=None, headers=None, timeout=5):
    req = urllib.request.Request(url, method=method, data=data)
    for k, v in (headers or {}).items():
        req.add_header(k, v)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, {k.lower(): v for k, v in r.headers.items()}, r.read()
    except urllib.error.HTTPError as e:
        return e.code, {k.lower(): v for k, v in e.headers.items()}, e.read()
    except Exception as e:
        return None, {}, str(e).encode()

def api(method, path, payload=None, timeout=5):
    data = None
    headers = {}
    if payload is not None:
        data = json.dumps(payload).encode()
        headers["Content-Type"] = "application/json"
    return call_url(BASE + path, method=method, data=data, headers=headers, timeout=timeout)

def api_json(method, path, payload=None, timeout=5):
    st, hdr, b = api(method, path, payload, timeout)
    try:
        val = json.loads(b.decode())
    except Exception:
        val = None
    return st, hdr, b, val

def check(name, ok, detail=""):
    results.append((name, bool(ok), detail or ""))

def can_connect(port, timeout=1.0):
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=timeout):
            return True
    except Exception:
        return False

def make_profile():
    return {
        "name": PROFILE_NAME,
        "listen_port": LISTEN_PORT,
        "upstream": None,
        "rules": [
            {
                "id": "r1",
                "label": "test",
                "method": "*",
                "match": "prefix",
                "path": "/api/test",
                "enabled": True,
                "active_variant": "v1",
                "cycle": False,
                "variants": [
                    {
                        "id": "v1",
                        "label": "ok",
                        "status": 200,
                        "json": "{\"v\":1,\"id\":\"{{guid}}\"}",
                        "randomize": True,
                        "delay_ms": 0,
                    }
                ],
            }
        ],
    }

# ---------- tests ----------
def T1_crud():
    st, hdr, b, val = api_json("POST", "/debug/mock/profile", make_profile())
    check("T1 create profile", st == 200 and isinstance(val, dict) and val.get("ok") is True, f"status={st} body={b[:120]!r}")
    st, hdr, b, val = api_json("GET", f"/debug/mock/profile?name={PROFILE_NAME}")
    ok = st == 200 and isinstance(val, dict) and val.get("name") == PROFILE_NAME and val.get("listen_port") == LISTEN_PORT and len(val.get("rules", [])) == 1
    check("T1 get profile", ok, f"status={st}")
    st, hdr, b, val = api_json("GET", "/debug/mock/profiles")
    ok = st == 200 and isinstance(val, dict) and any(p.get("name") == PROFILE_NAME for p in val.get("profiles", []))
    check("T1 list profiles", ok, f"status={st}")

def T2_start_stop():
    st, hdr, b, val = api_json("POST", "/debug/mock/start", {"name": PROFILE_NAME})
    check("T2 start", st == 200 and isinstance(val, dict) and val.get("ok") is True and val.get("port") == LISTEN_PORT, f"status={st} body={b[:120]!r}")
    check("T2 port listening", can_connect(LISTEN_PORT), "")
    st, hdr, b, val = api_json("POST", "/debug/mock/stop", {"name": PROFILE_NAME})
    check("T2 stop", st == 200 and isinstance(val, dict) and val.get("ok") is True, f"status={st}")
    time.sleep(0.2)
    check("T2 port refused", not can_connect(LISTEN_PORT), "")

def T3_randomize():
    st, hdr, b, val = api_json("POST", "/debug/mock/start", {"name": PROFILE_NAME})
    check("T3 start", st == 200 and isinstance(val, dict) and val.get("ok") is True, f"status={st}")
    url = f"{GATEWAY}/api/test/x"
    st1, h1, b1 = call_url(url, timeout=5)
    st2, h2, b2 = call_url(url, timeout=5)
    ok_json = False
    id1 = id2 = None
    try:
        j1 = json.loads(b1.decode())
        j2 = json.loads(b2.decode())
        id1 = j1.get("id")
        id2 = j2.get("id")
        ok_json = isinstance(id1, str) and isinstance(id2, str) and id1 != id2
    except Exception:
        pass
    check("T3 randomize json", st1 == 200 and st2 == 200 and ok_json, f"status={st1}/{st2} id={id1}/{id2}")
    check("T3 content-type", h1.get("content-type", "").startswith("application/json") and h2.get("content-type", "").startswith("application/json"), f"{h1.get('content-type')}")

def T4_static():
    st, hdr, b, val = api_json("GET", f"/debug/mock/profile?name={PROFILE_NAME}")
    if st != 200 or not isinstance(val, dict):
        check("T4 load profile", False, f"status={st}")
        return
    val["rules"][0]["variants"][0]["randomize"] = False
    st, hdr, b, val2 = api_json("POST", "/debug/mock/profile", val)
    check("T4 save static", st == 200 and isinstance(val2, dict) and val2.get("ok") is True, f"status={st}")
    api_json("POST", "/debug/mock/start", {"name": PROFILE_NAME})
    url = f"{GATEWAY}/api/test/x"
    st1, h1, b1 = call_url(url, timeout=5)
    st2, h2, b2 = call_url(url, timeout=5)
    check("T4 static equal", st1 == 200 and st2 == 200 and b1 == b2, f"status={st1}/{st2} len={len(b1)}/{len(b2)}")

def T5_no_rule():
    st, hdr, b, val = api_json("GET", f"/debug/mock/profile?name={PROFILE_NAME}")
    if st != 200 or not isinstance(val, dict):
        check("T5 load profile", False, f"status={st}")
        return
    val["upstream"] = None
    st, hdr, b, val2 = api_json("POST", "/debug/mock/profile", val)
    check("T5 save no-upstream", st == 200 and isinstance(val2, dict) and val2.get("ok") is True, f"status={st}")
    api_json("POST", "/debug/mock/start", {"name": PROFILE_NAME})
    st, hdr, b = call_url(f"{GATEWAY}/nope", timeout=5)
    err = None
    try:
        err = json.loads(b.decode()).get("error")
    except Exception:
        pass
    # поведение закреплено e2e 16.11.2026: нет правила и нет upstream → 404 no matching rule (не 502)
    check("T5 no-rule 404", st == 404 and isinstance(err, str) and len(err) > 0, f"status={st} body={b[:120]!r}")

def T6_passthrough():
    st, hdr, b, val = api_json("GET", f"/debug/mock/profile?name={PROFILE_NAME}")
    if st != 200 or not isinstance(val, dict):
        check("T6 load profile", False, f"status={st}")
        return
    val["upstream"] = f"http://127.0.0.1:{ECHO_PORT}"
    st, hdr, b, val2 = api_json("POST", "/debug/mock/profile", val)
    check("T6 save upstream", st == 200 and isinstance(val2, dict) and val2.get("ok") is True, f"status={st}")
    api_json("POST", "/debug/mock/start", {"name": PROFILE_NAME})
    payload = b'{"hello":"world"}'
    st, hdr, b = call_url(f"{GATEWAY}/echo", method="POST", data=payload, headers={"Content-Type": "application/json"}, timeout=5)
    body_field = None
    try:
        body_field = json.loads(b.decode()).get("body")
    except Exception:
        pass
    check("T6 passthrough status", st == 201, f"status={st}")
    check("T6 passthrough body", body_field == payload.decode(), f"body={body_field!r}")

def T7_traffic():
    st, hdr, b, val = api_json("GET", f"/debug/mock/traffic?name={PROFILE_NAME}")
    ok = st == 200 and isinstance(val, dict) and isinstance(val.get("traffic"), list)
    found = False
    if ok:
        for item in val["traffic"]:
            if item.get("path") == "/api/test/x" and item.get("status") == 200:
                found = True
                break
    check("T7 traffic contains", ok and found, f"status={st} count={len(val.get('traffic', [])) if isinstance(val, dict) else '?'}")

def T7b_traffic_all():
    # глобальная панель «Трафик»: без name — трафик всех профилей, записи с полем profile
    st, hdr, b, val = api_json("GET", "/debug/mock/traffic")
    ok = st == 200 and isinstance(val, dict) and isinstance(val.get("traffic"), list)
    found = False
    if ok:
        for item in val["traffic"]:
            if item.get("profile") == PROFILE_NAME and item.get("path") == "/api/test/x":
                found = True
                break
    check("T7b traffic all-profiles", ok and found, f"status={st} count={len(val.get('traffic', [])) if isinstance(val, dict) else '?'}")

def T8_preview():
    st, hdr, b, val = api_json("POST", "/debug/mock/preview", {"json": "{{int:5:5}}", "randomize": True})
    ok = st == 200 and isinstance(val, dict) and val.get("text") == "5"
    check("T8 preview", ok, f"status={st} body={b[:120]!r}")

# ---------- runner ----------
# (хвост ответа модели d5 оборвался на T8; раннер дописан валидатором
#  по образцу api_matrix.py; cleanup учитывает POST /debug/mock/profile/delete
#  — фактический маршрут control_router в mock_server.rs)
def main():
    global srv
    srv = EchoServer(("127.0.0.1", ECHO_PORT), Echo)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    for fn in (T1_crud, T2_start_stop, T3_randomize, T4_static, T5_no_rule,
               T6_passthrough, T7_traffic, T7b_traffic_all, T8_preview):
        try:
            fn()
        except Exception as e:
            check(fn.__name__, False, f"exception: {e!r}")
    try:
        api_json("POST", "/debug/mock/stop", {"name": PROFILE_NAME})
        api_json("POST", "/debug/mock/profile/delete", {"name": PROFILE_NAME})
    except Exception:
        pass
    srv.shutdown()
    fails = [r for r in results if not r[1]]
    w = max(len(r[0]) for r in results) + 2
    for name, ok, detail in results:
        print(("PASS  " if ok else "FAIL  ") + name.ljust(w) + (detail or ""))
    print(f"\n=== {len(results)-len(fails)}/{len(results)} passed, {len(fails)} failed ===")
    sys.exit(1 if fails else 0)


if __name__ == "__main__":
    main()
