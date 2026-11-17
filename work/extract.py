#!/usr/bin/env python3
"""Достаёт последний fenced-блок из ответов делегатов в staging-файлы."""
import os, re, sys

W = os.path.dirname(os.path.abspath(__file__))
MAP = {
    "d1_random": ("mock_random.rs", "rust"),
    "d2_server": ("mock_server.rs", "rust"),
    "d3_proxy":  ("proxy.rs", "rust"),
    "d4_css":    ("styles-proxy.css", "css"),
    "d5_tests":  ("api_matrix_mock.py", "python"),
}
os.makedirs(os.path.join(W, "staging"), exist_ok=True)
for tid, (fname, lang) in MAP.items():
    p = os.path.join(W, "out", tid + ".answer.md")
    if not os.path.exists(p):
        print(f"skip {tid}: нет ответа")
        continue
    text = open(p).read()
    pat = r"```(?:" + lang + r")?\s*\n(.*?)```"
    blocks = re.findall(pat, text, re.S)
    blocks = [b for b in blocks if len(b) > 200]
    if not blocks:
        print(f"FAIL {tid}: нет fenced-блока >200 символов")
        continue
    code = max(blocks, key=len)
    open(os.path.join(W, "staging", fname), "w").write(code)
    print(f"OK {tid} -> staging/{fname} ({len(code)} chars, блоков: {len(blocks)})")
