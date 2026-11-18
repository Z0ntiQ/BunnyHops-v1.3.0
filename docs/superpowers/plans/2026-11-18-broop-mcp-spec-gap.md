# BROOP → MCP-спека: закрытие четырёх разрывов — план

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** сделать автоматизацию BROOP соответствующей ТЗ `mcp_desktop_automation_spec-2.md`: клики не зависят от активности окна, есть ожидание условия, JSON-RPC 2.0 с токеном, MCP-сервер поверх API.

**Architecture:** ядро (BROOP-карта, data-bh, dispatch, stateDiff) готово и не меняется. Сверху достраиваются четыре слоя: round-trip обновление карты через WS вместо heartbeat, `/debug/wait`, `/rpc` отдельным атомарным модулем с Bearer-токеном и lock-файлом, тонкий Python MCP-сервер. `/debug/*` остаётся обратимо совместимым — на нём живые скрипты.

**Tech Stack:** Rust (axum 0.7, Leptos/wasm frontend), Python 3 + `mcp[cli]` (FastMCP).

**Spec:** `~/.hermes/attachments/mcp_desktop_automation_spec-2.md` (Блоки 1 и 2, матрица выбора).

## Global Constraints

- Никаких новых HTTP-зависимостей в Rust: только axum 0.7, уже в `Cargo.toml`.
- Атомарные модули: `/rpc` живёт в новом `src/rpc_api.rs`, не раздувает `debug_api.rs` (глобальное правило пользователя).
- Контракт `/debug/*` не ломать: существующие скрипты (`scripts/` скилла) должны продолжать работать без правок.
- Тесты Rust — только `#[cfg(test)]` в модуле, паттерн уже есть в `src/mock_random.rs`.
- Коммит после каждой задачи, локально; `git push` никогда без явной команды.
- Тесты frontend: `cargo test -p bunny-hops-ui --target wasm32-unknown-unknown` не запускаются в CI-режиме — проверка wasm-части идёт E2E-скриптом через debug API (см. Task 1 Step 6).

## Review Focus

- Окно в фоне, heartbeat мёртв: клик по существующему элементу обязан пройти через round-trip, а не получить 404. (Task 1)
- Элемент `disabled` или `visible:false`: гейт отвечает структурированной ошибкой с причиной, молчаливый клик запрещён. (Task 1)
- Lock-файл после перезапуска приложения (stale pid): MCP-клиент обязан это обнаружить и сказать «перезапусти/переподключись», а не слать запросы в мёртвый порт. (Task 3, 4)
- Malformed JSON-RPC: отсутствующий `params`, неизвестный `method` — коды `-32602` / `-32601`, процесс приложения не падает. (Task 3)
- Таймаут ожидания: `/debug/wait` по таймауту возвращает последний снимок и причину, а не голую 408. (Task 2)

---

### Task 1: Round-trip обновление карты + активация окна (ТЗ 2.2.4-запрет sleep, 2.2.5)

**Files:**
- Modify: `src/debug_api.rs` (гейт `element.click`: при промахе — WS round-trip и повторная проверка)
- Modify: `frontend/src/debug_bridge.rs` (обработчик сообщения `{"action":"elements.collect"}` → `collect()` + POST `/debug/elements` + ack)
- Modify: `desktop/src/main.rs` (обработчик активации окна: показать/восстановить/сфокусировать окно через AppKit-связывания, уже используемые в desktop-крейте)
- Test: unit-тест гейта в `src/debug_api.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: существующий WS-канал server→page и POST `/debug/elements` (страница→сервер).
- Produces: WS-сообщение `{"action":"elements.collect"}`; внутренний `fn gate_check(store,&id) -> Result<Item, GateReject{reason}>` с причинами `not_found|disabled|invisible`; HTTP `POST /debug/window/activate` → `{"ok":true}`.

- [ ] **Step 1:** Написать падающий unit-тест `gate_rejects_then_refreshes_via_roundtrip`: пустой store + мок round-trip, заполняющий store одним элементом → `gate_check` после refresh возвращает `Ok(item)`; отдельный тест `gate_rejects_disabled` → `GateReject{reason:"disabled"}`.
- [ ] **Step 2:** `cargo test -p bunny-hops-server gate_` → FAIL (нет `gate_check`).
- [ ] **Step 3:** Вынести проверку гейта в `gate_check`, промахи классифицировать по причинам. Реализовать в обработчике `element.click`: `not_found` → послать в page `elements.collect`, дождаться POST `/debug/elements` (таймаут 2 с, канал oneshot по id элемента), повторить `gate_check` один раз; `disabled|invisible` → сразу структурированная ошибка.
- [ ] **Step 4:** `cargo test -p bunny-hops-server gate_` → PASS.
- [ ] **Step 5:** В `debug_bridge.rs` добавить ветку `elements.collect`: `push_elements_now()` + ответ тем же PUT/POST на `/debug/elements`. В `desktop/src/main.rs` — активация окна по флагу из обработчика `/debug/window/activate` (route зарегистрировать рядом с `mcp_router`).
- [ ] **Step 6:** E2E-проверка на живом приложении со свёрнутым/фоновым окном: скрипт `scripts/click_bg.py` — `POST /debug/window/activate`, затем `element.click` по `topbar.mock_toggle`, затем `GET /debug/broop` содержит `profile.*` после клика. Запуск: `python3 scripts/click_bg.py $(cat <port-file>)`. Ожидаемый результат: exit 0, клик прошёл при неактивном до активации окне.
- [ ] **Step 7:** Коммит: `feat(debug): round-trip element map refresh and window activation`.

### Task 2: `/debug/wait` — явное ожидание условия (ТЗ 2.2.4)

**Files:**
- Modify: `src/debug_api.rs` (route `POST /debug/wait`, поллер 50 мс)
- Test: unit-тесты предикатов в том же модуле

**Interfaces:**
- Consumes: `gate_check` и round-trip `elements.collect` из Task 1; `full_state()` и `path_get()` из текущего кода.
- Produces: `POST /debug/wait {"predicate": {...}, "timeout_ms": 5000}` с предикатами `{"kind":"element","id":...,"state":"present|absent|enabled|disabled","value"?}` и `{"kind":"state","path":...,"equals":...}`; ответ `{"ok":true,"matched":<item>,"waited_ms":N}` или `{"ok":false,"reason":"timeout","last":<снимок>}` со статусом HTTP 200 (ошибка в теле, не в статусе — клиенту нужен снимок).

- [ ] **Step 1:** Падающие unit-тесты `wait_predicate_element_present_matches`, `wait_predicate_state_path_equals_matches`, `wait_timeout_returns_last_snapshot`.
- [ ] **Step 2:** `cargo test -p bunny-hops-server wait_` → FAIL.
- [ ] **Step 3:** Реализовать `eval_predicate(store, state, &p) -> Option<Value>` и поллер: шаг 50 мс, каждый цикл — актуальный store (последний push + round-trip раз в 500 мс, не чаще), таймаут из запроса (пол 5000 мс).
- [ ] **Step 4:** `cargo test -p bunny-hops-server wait_` → PASS.
- [ ] **Step 5:** E2E: в `scripts/click_bg.py` заменить `sleep`-паузы на `/debug/wait` (появление `profile.new_name` после клика, expect < 1 с). Прогон скрипта → exit 0.
- [ ] **Step 6:** Коммит: `feat(debug): explicit condition waiting endpoint`.

### Task 3: JSON-RPC 2.0 + Bearer-токен + lock-файл (ТЗ 1.2.1, 1.2.5)

**Files:**
- Create: `src/rpc_api.rs` (router `/rpc`, маппинг методов, коды ошибок)
- Modify: `desktop/src/main.rs` (генерация токена при старте, запись lock-файла `~/.config/bunny-hops/rpc.json` `{port, token, pid}`, режим 0600)
- Test: `#[cfg(test)] mod tests` в `src/rpc_api.rs` (axum-роутер in-memory, как позволяет текущий код `mcp_router`)

**Interfaces:**
- Consumes: диспатч-обработчик и `full_state()`/`broop_handler` из `debug_api.rs` (вызов функций, не HTTP на самого себя).
- Produces: `POST /rpc` c envelope JSON-RPC 2.0; методы `system.get_state`, `ui.inspect`, `ui.click{id}`, `ui.type{id,text}`, `ui.wait{predicate,timeout_ms}`, `app.execute{action,payload}`; ошибки `-32601` (нет метода), `-32602` (параметры), `-32603` (внутренняя, `data.stacktrace`); `Authorization: Bearer <token>` обязателен на `/rpc`, `/debug/*` остаются без авторизации.

- [ ] **Step 1:** Падающие тесты: `rpc_requires_bearer` (401 без заголовка), `rpc_unknown_method_is_32601`, `rpc_missing_params_is_32602`, `rpc_get_state_returns_result`.
- [ ] **Step 2:** `cargo test -p bunny-hops-server rpc_` → FAIL.
- [ ] **Step 3:** Реализовать `pub(crate) fn rpc_router(state, token) -> Router`: разбор envelope, таблица методов → существующие обработчики, envelope ответа. Токен — `String` в состоянии роутера, сгенерирован `rand` 32 hex на старте.
- [ ] **Step 4:** `cargo test -p bunny-hops-server rpc_` → PASS.
- [ ] **Step 5:** В `desktop/src/main.rs`: после получения порта — запись lock-файла (права 0600, атомарно: temp+rename), удаление при завершении.
- [ ] **Step 6:** Ручная проверка: `curl -H "Authorization: Bearer $(jq -r .token ~/.config/bunny-hops/rpc.json)" -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"system.get_state"}' http://127.0.0.1:$(jq -r .port ...)/rpc` → `result` с состоянием; без заголовка → 401.
- [ ] **Step 7:** Коммит: `feat(rpc): json-rpc 2.0 endpoint with bearer token and lockfile`.

### Task 4: MCP-сервер FastMCP поверх `/rpc` (ТЗ 1.3, 2.3)

**Files:**
- Create: `~/.hermes/skills/bunny-hops/mcp/server.py`
- Create: `~/.hermes/skills/bunny-hops/mcp/smoke.py` (stdio-проверка без Hermes)
- Modify: `~/.hermes/skills/bunny-hops/SKILL.md` (секция «MCP-сервер»: как запустить, регистрация)

**Interfaces:**
- Consumes: lock-файл `~/.config/bunny-hops/rpc.json` (проверка живости `pid` через `os.kill(pid,0)` перед каждым вызовом), `/rpc` из Task 3.
- Produces: инструменты MCP: `execute_business_action(action_name, payload_json)`, `get_application_state()`, `inspect_ui_tree()`, `ui_click_by_id(automation_id)`, `ui_set_text_by_id(automation_id, text)`, `wait_for(predicate_json, timeout_ms)` — имена и сигнатуры строго как в ТЗ (Блок 1 §1.3, Блок 2 §2.3).

- [ ] **Step 1:** `python3 -m venv ~/.hermes/skills/bunny-hops/mcp/.venv && ~/.hermes/skills/bunny-hops/mcp/.venv/bin/pip install "mcp[cli]"` — зафиксировать версию в `mcp/requirements.txt`.
- [ ] **Step 2:** Написать `server.py`: чтение lock-файла, `_rpc(method, params)` c Bearer, шесть инструментов-обёрток; при stale pid — ошибка «приложение не запущено, порт устарел».
- [ ] **Step 3:** `smoke.py`: запускает server.py как stdio-подпроцесс, вызывает `get_application_state` и `inspect_ui_tree` против живого приложения, сверяет с прямыми ответами `/debug/state` и `/debug/broop`. Прогон: `mcp/.venv/bin/python mcp/smoke.py` → exit 0, расхождений нет.
- [ ] **Step 4:** Регистрация в Hermes: `manage_connections` (локальный MCP-сервер) — показать пользователю, подтверждение остаётся за ним.
- [ ] **Step 5:** Обновить `SKILL.md` (запуск, путь к venv, список инструментов).
- [ ] **Step 6:** Коммит: `feat(mcp): FastMCP server over /rpc`.

---

## Порядок и зависимости

Task 1 → Task 2 (поллер использует round-trip) → Task 3 (метод `ui.wait` использует Task 2) → Task 4 (обёртка над Task 3). Задачи 1 и 3 можно делать параллельно, если менять разные файлы; `desktop/src/main.rs` трогуют обе — при параллели коммитить последовательно.
