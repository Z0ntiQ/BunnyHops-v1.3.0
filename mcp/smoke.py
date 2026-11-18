#!/usr/bin/env python3
"""Живой smoke MCP-сервера broop (Task 4 Step 3 спеки).

Запускает server.py как stdio-подпроцесс, вызывает get_application_state и
inspect_ui_tree против живого приложения и сверяет с прямыми HTTP-ответами
/rpc (system.get_state, ui.inspect) с Bearer-токеном из lock-файла.
Десктоп отдаёт только /rpc — прямые /debug/* на нём не подняты, эталоном
служит прямой вызов того же метода.

Требует запущенное приложение BROOP (~/.config/bunny-hops/rpc.json).
Запуск: mcp/.venv/bin/python mcp/smoke.py   → exit 0, расхождений нет.
"""
import asyncio
import json
import os
import sys
import urllib.request
from pathlib import Path

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

HERE = Path(__file__).parent
SERVER = HERE / "broop_mcp" / "server.py"
LOCK = Path.home() / ".config" / "bunny-hops" / "rpc.json"

EXPECTED_TOOLS = {
    "get_application_state", "inspect_ui_tree", "ui_click_by_id",
    "ui_set_text_by_id", "wait_for", "execute_business_action",
}


def direct_rpc(port: int, token: str, method: str, params: dict | None = None):
    """Прямой JSON-RPC вызов /rpc с Bearer — эталон для сверки."""
    body: dict = {"jsonrpc": "2.0", "id": 1, "method": method}
    if params is not None:
        body["params"] = params
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/rpc",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json",
                 "Authorization": f"Bearer {token}"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=10) as resp:
        data = json.loads(resp.read())
    if "error" in data:
        raise RuntimeError(f"прямой {method} вернул ошибку: {data['error']}")
    return data["result"]


async def call_tool_json(session: ClientSession, name: str, args: dict):
    """Вызвать MCP-инструмент и разобрать JSON из текстового контента."""
    res = await session.call_tool(name, args)
    if getattr(res, "isError", False):
        raise RuntimeError(f"{name} вернул ошибку: {res.content}")
    text = "".join(c.text for c in res.content if c.type == "text")
    return json.loads(text)


async def main() -> int:
    if not LOCK.exists():
        print(f"FAIL: нет lock-файла {LOCK} — запустите приложение BROOP")
        return 1
    lock = json.loads(LOCK.read_text())
    port, token = int(lock["port"]), lock["token"]

    params = StdioServerParameters(command=sys.executable, args=[str(SERVER)],
                                  env={**os.environ})
    async with stdio_client(params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()
            print("INIT-OK")

            tools = await session.list_tools()
            names = {t.name for t in tools.tools}
            missing = EXPECTED_TOOLS - names
            if missing:
                print(f"FAIL: нет инструментов: {sorted(missing)}")
                return 1
            print(f"TOOLS-OK: {len(names)} инструментов")

            mismatches = 0
            for tool, method in [("get_application_state", "system.get_state"),
                                 ("inspect_ui_tree", "ui.inspect")]:
                try:
                    got = await call_tool_json(session, tool, {})
                except RuntimeError as e:
                    print(f"FAIL: {e}")
                    mismatches += 1
                    continue
                expected = direct_rpc(port, token, method)
                if got == expected:
                    print(f"MATCH-OK: {tool} == прямой {method}")
                else:
                    print(f"MISMATCH: {tool} != прямой {method}")
                    mismatches += 1

            # доп. проверка пути до /rpc и обратно через wait_for
            try:
                wait_res = await call_tool_json(session, "wait_for", {
                    "predicate_json": json.dumps(
                        {"kind": "state", "path": "app.name",
                         "equals": "bunny-hops"}),
                    "timeout_ms": 1000,
                })
                if wait_res.get("ok") is not True:
                    print(f"FAIL: wait_for: {wait_res}")
                    mismatches += 1
                else:
                    print(f"WAIT-OK: waited_ms={wait_res.get('waited_ms')}")
            except RuntimeError as e:
                print(f"FAIL: {e}")
                mismatches += 1

            if mismatches:
                print(f"FAIL: расхождений: {mismatches}")
                return 1
            print("SMOKE-PASS")
            return 0


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
