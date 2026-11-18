#!/usr/bin/env python3
"""FastMCP server 'broop' — thin JSON-RPC 2.0 client over /rpc endpoint."""

import json
import os
import sys
from pathlib import Path
from typing import Any

import httpx
from mcp.server.mcpserver import MCPServer

LOCK_FILE = Path.home() / ".config" / "bunny-hops" / "rpc.json"

_PORT: int | None = None
_TOKEN: str | None = None
_PID: int | None = None


def _pid_alive(pid: int | None) -> bool:
    """Живость процесса приложения по pid из lock-файла (os.kill(pid, 0))."""
    if not pid:
        return False
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def _ensure_lock() -> tuple[int, str]:
    """Read port and token from lock file. Supports 'port:token', 'port token', or JSON.

    Кэш сбрасывается, если pid из lock-файла мёртв: перечитываем файл заново
    (приложение перезапустилось и записало новый порт/токен).
    """
    global _PORT, _TOKEN, _PID
    if _PORT is not None and _TOKEN is not None:
        if _pid_alive(_PID):
            return _PORT, _TOKEN
        _PORT, _TOKEN, _PID = None, None, None

    if not LOCK_FILE.exists():
        raise RuntimeError(f"Lock file not found: {LOCK_FILE} — запустите приложение BROOP")
    text = LOCK_FILE.read_text().strip()
    if not text:
        raise RuntimeError(f"Lock file empty: {LOCK_FILE}")

    # Try JSON first
    try:
        data = json.loads(text)
        port = int(data.get("port", 0))
        token = data.get("token", "")
        pid = int(data.get("pid", 0)) or None
        if port and token:
            if not _pid_alive(pid):
                raise RuntimeError("приложение не запущено, порт устарел")
            _PORT, _TOKEN, _PID = port, token, pid
            return port, token
    except json.JSONDecodeError:
        pass

    # Try 'port:token' or 'port token'
    for sep in (":", " ", "\t"):
        if sep in text:
            parts = text.split(sep, 1)
            if len(parts) == 2:
                try:
                    port = int(parts[0].strip())
                    token = parts[1].strip()
                    if port and token:
                        _PORT, _TOKEN = port, token
                        return port, token
                except ValueError:
                    continue

    raise RuntimeError(f"Cannot parse lock file {LOCK_FILE}: expected 'port:token' or JSON {{port, token}}")


def _rpc(method: str, params: dict[str, Any] | None = None, rpc_id: int = 1) -> Any:
    """Call JSON-RPC 2.0 /rpc endpoint. Raise RuntimeError with mapped messages on errors."""
    port, token = _ensure_lock()
    base_url = f"http://127.0.0.1:{port}/rpc"
    headers = {"Authorization": f"Bearer {token}", "Content-Type": "application/json"}

    payload = {"jsonrpc": "2.0", "id": rpc_id, "method": method}
    if params is not None:
        payload["params"] = params

    with httpx.Client(timeout=10.0) as client:
        resp = client.post(base_url, headers=headers, json=payload)

    if resp.status_code == 401:
        raise RuntimeError("токен недействителен, перечитай lock-файл")

    try:
        data = resp.json()
    except json.JSONDecodeError:
        raise RuntimeError(f"Invalid JSON response: {resp.text[:200]}")

    if "error" in data:
        err = data["error"]
        code = err.get("code", -32603)
        message = err.get("message", "Unknown error")
        if code == -32603:
            stack = err.get("data", {}).get("stacktrace", "")
            raise RuntimeError(f"{message}\n{stack}".strip())
        raise RuntimeError(f"JSON-RPC error {code}: {message}")

    return data.get("result")


mcp = MCPServer("broop")


@mcp.tool()
def get_application_state() -> Any:
    """Полное состояние приложения (system.get_state)."""
    return _rpc("system.get_state")


@mcp.tool()
def inspect_ui_tree() -> Any:
    """Карта BROOP UI: id, роли, состояния элементов (ui.inspect)."""
    return _rpc("ui.inspect")


@mcp.tool()
def ui_click_by_id(automation_id: str) -> Any:
    """Клик по элементу его BROOP id (ui.click)."""
    return _rpc("ui.click", {"id": automation_id})


@mcp.tool()
def ui_set_text_by_id(automation_id: str, text: str) -> Any:
    """Текст в элемент по BROOP id (ui.type)."""
    return _rpc("ui.type", {"id": automation_id, "text": text})


@mcp.tool()
def wait_for(predicate_json: str, timeout_ms: int = 5000) -> Any:
    """Ждать условие (ui.wait). predicate_json — JSON строкой, например
    {"kind":"element","id":"...","state":"present"} или
    {"kind":"state","path":"...","equals":...}."""
    predicate = json.loads(predicate_json) if isinstance(predicate_json, str) else predicate_json
    return _rpc("ui.wait", {"predicate": predicate, "timeout_ms": timeout_ms})


@mcp.tool()
def execute_business_action(action_name: str, payload_json: str = "{}") -> Any:
    """Бизнес-действие из реестра (app.execute). payload_json — JSON строкой."""
    payload = json.loads(payload_json) if isinstance(payload_json, str) else payload_json
    return _rpc("app.execute", {"action": action_name, "payload": payload})


if __name__ == "__main__":
    mcp.run()