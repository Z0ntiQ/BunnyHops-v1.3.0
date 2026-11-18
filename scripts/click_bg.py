#!/usr/bin/env python3
"""E2E Task 1 (шаг 6 плана 2026-11-18): клик при фоновом окне без sleep.

Запуск: python3 scripts/click_bg.py <port>
Сценарий: включить debug-режим, активировать окно через API (окно до этого
неактивно), кликнуть topbar.mock_toggle через /debug/dispatch, проверить,
что BROOP-карта пополнилась полями профиля (round-trip без heartbeat).
Контракт эндпоинтов сверен с src/debug_api.rs commit 3491de0:
  POST /api/debug-mode {"on":true}
  POST /debug/window/activate
  POST /debug/dispatch {"action":"element.click","payload":{"id":...}}
  GET  /debug/broop
Выход: 0 — клик прошёл и карта обновилась; 1 — провал с причиной.
"""
import json
import sys
import urllib.error
import urllib.request

PORT = sys.argv[1] if len(sys.argv) > 1 else sys.exit("usage: click_bg.py <port>")
BASE = f"http://127.0.0.1:{PORT}"


def call(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        BASE + path, data=data, method=method,
        headers={"content-type": "application/json"} if data else {})
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            return r.status, json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e:
        try:
            payload = json.loads(e.read() or b"{}")
        except ValueError:
            payload = {}
        return e.code, payload


def main():
    # 0. Включить debug-режим (гейт всех /debug/*).
    st, body = call("POST", "/api/debug-mode", {"on": True})
    if st != 200:
        print(f"FAIL: debug-mode -> HTTP {st} {body}")
        return 1
    print("ok: debug mode on")

    # 1. Активация окна через новый API (окно до этого неактивно/свёрнуто).
    st, body = call("POST", "/debug/window/activate")
    if st != 200:
        print(f"FAIL: window/activate -> HTTP {st} {body}")
        return 1
    print("ok: window activated via API")

    # 2. Клик по переключателю мока — без предварительного sleep.
    st, body = call("POST", "/debug/dispatch",
                    {"action": "element.click",
                     "payload": {"id": "topbar.mock_toggle"}})
    if st != 200 or not body.get("ok"):
        print(f"FAIL: element.click -> HTTP {st} "
              f"{json.dumps(body, ensure_ascii=False)[:200]}")
        return 1
    print("ok: click accepted (gate passed, no pre-sleep)")

    # 3. Карта должна пополниться полями профиля (профиль появляется после
    # включения мока) — это и есть round-trip-обновление без heartbeat.
    st, body = call("GET", "/debug/broop")
    if st != 200:
        print(f"FAIL: /debug/broop -> HTTP {st}")
        return 1
    items = body.get("elements") or body.get("items") or body.get("objects") or []
    ids = [e.get("id", "") for e in items if isinstance(e, dict)]
    profile_ids = [i for i in ids if i.startswith("profile.")]
    if not profile_ids:
        print(f"FAIL: no profile.* in BROOP map after click "
              f"(elements={len(ids)})")
        return 1
    print(f"ok: round-trip refreshed map, profile ids: {profile_ids[:5]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
