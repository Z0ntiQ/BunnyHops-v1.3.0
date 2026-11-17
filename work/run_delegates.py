#!/usr/bin/env python3
"""Прогон 5 веб-делегатов: Kilo/LLM7/OVH через мост :8110.
Читает work/briefs/<id>.md, пишет work/out/<id>.answer.md + <id>.raw.json.
При отказе — ротация на fallback-канал той же части."""
import json, os, re, sys, threading, urllib.request, urllib.error

W = os.path.dirname(os.path.abspath(__file__))
PROXY = urllib.request.ProxyHandler({"http": "http://127.0.0.1:8110", "https": "http://127.0.0.1:8110"})
OPENER = urllib.request.build_opener(PROXY)

SYSTEM = (
    "Ты — senior Rust/Leptos/CSS/Python разработчик, пишешь один модуль существующего приложения по контракту.\n"
    "Правила ответа:\n"
    "- Ровно один fenced-блок с полным содержимым файла, без какого-либо текста вокруг него.\n"
    "- Дословно соблюдай сигнатуры, имена, JSON-схемы и списки классов из SPEC.\n"
    "- Используй только перечисленные зависимости. Не выдумывай API: приложенные фрагменты существующего кода — эталон синтаксиса библиотек.\n"
    "- Перед выводом мысленно проверь: код компилируется, тесты проходят.\n"
)

def channel_kilo(model):
    return ("https://api.kilo.ai/api/gateway/v1/chat/completions", None, model)

def channel_llm7(model):
    return ("https://api.llm7.io/v1/chat/completions", None, model)

def channel_ovh(model):
    return ("https://oai.endpoints.kepler.ai.cloud.ovh.net/v1/chat/completions", None, model)

TASKS = {
    "d1_random": dict(max_tokens=24000, chain=[channel_kilo("nvidia/nemotron-3-ultra-550b-a55b:free"), channel_ovh("gpt-oss-120b")]),
    "d2_server": dict(max_tokens=24000, chain=[channel_kilo("nvidia/nemotron-3-ultra-550b-a55b:free"), channel_ovh("gpt-oss-120b")]),
    "d3_proxy":  dict(max_tokens=30000, chain=[channel_kilo("nvidia/nemotron-3-ultra-550b-a55b:free"), channel_ovh("gpt-oss-120b")]),
    "d4_css":    dict(max_tokens=10000, chain=[channel_kilo("nvidia/nemotron-3.5-lightning:free"), channel_kilo("nvidia/nemotron-3.5-nano:free")]),
    "d5_tests":  dict(max_tokens=16000, chain=[channel_kilo("baidu/ernie-5.5-flash-preview:free"), channel_kilo("nvidia/nemotron-3.5-lightning:free")]),
}

def call(base, key, model, prompt, max_tokens):
    """Стриминг SSE: дельты content/reasoning копятся до finish_reason."""
    body = {
        "model": model,
        "messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": prompt}],
        "max_tokens": max_tokens,
        "temperature": 0.15,
        "stream": True,
    }
    req = urllib.request.Request(base, data=json.dumps(body).encode(),
                                headers={"Content-Type": "application/json", "Accept": "text/event-stream"})
    if key:
        req.add_header("Authorization", "Bearer " + key)
    content, reasoning, finish, usage = [], [], None, None
    with OPENER.open(req, timeout=900) as r:
        for raw in r:
            line = raw.decode("utf-8", "replace").strip()
            if not line.startswith("data:"):
                continue
            payload = line[5:].strip()
            if payload == "[DONE]":
                break
            try:
                ch = json.loads(payload)["choices"][0]
            except Exception:
                continue
            d = ch.get("delta") or {}
            if d.get("content"):
                content.append(d["content"])
            if d.get("reasoning_content"):
                reasoning.append(d["reasoning_content"])
            if ch.get("finish_reason"):
                finish = ch["finish_reason"]
    return {"text": "".join(content) or "".join(reasoning), "finish": finish, "usage": usage}

def run(tid):
    spec = open(os.path.join(W, "SPEC.md")).read()
    brief = open(os.path.join(W, "briefs", tid + ".md")).read()
    t = TASKS[tid]
    log = []
    for (base, key, model) in t["chain"]:
        try:
            res = call(base, key, model, spec + "\n\n=== ПРИЛОЖЕННЫЕ ФРАГМЕНТЫ СУЩЕСТВУЮЩЕГО КОДА (ЭТАЛОН СИНТАКСИСА) ===\n" + brief, t["max_tokens"])
            text = res["text"]
            if not text.strip():
                log.append(f"{model}: пустой content finish={res['finish']}")
                continue
            os.makedirs(os.path.join(W, "out"), exist_ok=True)
            with open(os.path.join(W, "out", tid + ".answer.md"), "w") as f:
                f.write(text)
            with open(os.path.join(W, "out", tid + ".raw.json"), "w") as f:
                json.dump({"model": model, "usage": res.get("usage"), "finish": res["finish"]}, f)
            print(f"[done] {tid} model={model} chars={len(text)} finish={res['finish']}", flush=True)
            return
        except Exception as e:
            log.append(f"{model}: {e}")
    print(f"[FAIL] {tid}: " + " | ".join(log), flush=True)

def staggered(tid, delay):
    import time
    time.sleep(delay)
    run(tid)

threads = [threading.Thread(target=staggered, args=(t, i * 20), daemon=True) for i, t in enumerate(TASKS)]
for th in threads: th.start()
for th in threads: th.join()
print("[all done]", flush=True)
