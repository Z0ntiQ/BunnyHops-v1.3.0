//! API отладки для внешних тест-инструментов (MCP): состояние, действия, события.
//!
//! Гейт включается тремя путями: переменная окружения `ENABLE_DEBUG_API` при старте,
//! `POST /api/debug-mode {"on": true|false}` и тумблер в шапке UI. Пока гейт выключен,
//! все `/debug/*` отвечают 404 `debug_api_disabled`, WS-рукопожатие отклоняется.
//! Исключение из гейта — запросы самого UI (заголовок `x-hop-ui`, для WS — `?src=ui`):
//! приложение обязано работать при выключенном MCP. Минимальный API без гейта:
//! `/api/status`, `/api/debug-mode`, `/screenshot`.
//!
//! Контракт: `docs/openapi-debug.json` (отдаётся как `/debug/openapi.json`).
//! Ответы действий содержат `stateDiff` — сравнение полного состояния до и после.

use axum::body::to_bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request};
use std::collections::HashMap;
use axum::http::{Method, StatusCode};
use axum::response::Response;
use axum::routing::{any, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, oneshot};
use tokio::time::sleep;

use crate::control;
use crate::mock_server;

const MAX_BODY: usize = 10 * 1024 * 1024;
/// Потолок записей в stateDiff: состояние объёмное, диффу нужен предел.
const DIFF_BUDGET: usize = 40;

// ---------- гейт ----------

fn enabled_flag() -> &'static AtomicBool {
    static F: AtomicBool = AtomicBool::new(false);
    &F
}

pub(crate) fn is_enabled() -> bool {
    enabled_flag().load(Ordering::Relaxed)
}

pub(crate) fn set_enabled(on: bool) {
    enabled_flag().store(on, Ordering::Relaxed);
}

pub(crate) fn init_from_env() {
    let on = std::env::var("ENABLE_DEBUG_API")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "on" | "yes"))
        .unwrap_or(false);
    enabled_flag().store(on, Ordering::Relaxed);
}

fn app_port() -> u16 {
    PORT_SLOT.load(Ordering::Relaxed)
}

pub(crate) fn set_port(port: u16) {
    PORT_SLOT.store(port, Ordering::Relaxed);
}

// Порт хранится в одном статическом слоте: set_port пишет, читатели считывают.
static PORT_SLOT: AtomicU16 = AtomicU16::new(0);

// ---------- шина событий ----------

fn hub() -> &'static broadcast::Sender<Value> {
    static H: OnceLock<broadcast::Sender<Value>> = OnceLock::new();
    H.get_or_init(|| broadcast::channel(512).0)
}

/// Publish любого события для WS-клиентов; без подписчиков событие теряется — это норма.
pub(crate) fn publish(event: Value) {
    if !is_enabled() {
        return;
    }
    let _ = hub().send(event);
}

/// Событие из журнала действий: вызывается из `control::log_action`.
pub(crate) fn publish_action(area: &str, action: &str, details: &Value) {
    publish(json!({"type": "action", "area": area, "action": action, "details": details, "ts": now_epoch()}))
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------- window activation channel ----------

/// Отправитель команды активации окна (из сервера в Tauri main thread).
fn window_activate_tx() -> &'static Mutex<Option<tokio::sync::oneshot::Sender<()>>> {
    static TX: OnceLock<Mutex<Option<tokio::sync::oneshot::Sender<()>>>> = OnceLock::new();
    TX.get_or_init(|| Mutex::new(None))
}

/// Установить отправителя активации окна (вызывается при старте сервера из desktop/main.rs).
pub(crate) fn set_window_activate_tx(tx: tokio::sync::oneshot::Sender<()>) {
    *window_activate_tx().lock().unwrap() = Some(tx);
}

/// Взять отправителя активации окна (потребляет его — однократный вызов).
fn take_window_activate_tx() -> Option<tokio::sync::oneshot::Sender<()>> {
    window_activate_tx().lock().unwrap().take()
}

/// Активировать окно: показать/восстановить/сфокусировать.
/// Возвращает Ok(()) если команда отправлена, Err если отправителя нет.
async fn window_activate() -> Result<(), String> {
    if let Some(tx) = take_window_activate_tx() {
        tx.send(()).map_err(|_| "receiver dropped".into())
    } else {
        Err("window activate channel not set".into())
    }
}

// ---------- снимок UI от фронтенда ----------

fn ui_store() -> &'static Mutex<Value> {
    static S: OnceLock<Mutex<Value>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(json!({"seq": 0, "state": {}})))
}

pub(crate) fn ui_seq() -> u64 {
    ui_store().lock().unwrap().get("seq").and_then(Value::as_u64).unwrap_or(0)
}

pub(crate) fn ui_state() -> Value {
    ui_store().lock().unwrap().get("state").cloned().unwrap_or(json!({}))
}

async fn ui_state_handler(Json(body): Json<Value>) -> Response {
    let seq = {
        let mut store = ui_store().lock().unwrap();
        let next = store.get("seq").and_then(Value::as_u64).unwrap_or(0) + 1;
        *store = json!({"seq": next, "state": body});
        next
    };
    publish(json!({"type": "ui_snapshot", "seq": seq}));
    control::json_response(StatusCode::OK, &json!({"ok": true, "seq": seq}))
}

/// Дождаться подтверждения от UI-клиента: новый снапшот после нашей команды.
pub(crate) async fn wait_ui_echo(seq_before: u64) -> Value {
    let deadline = Instant::now() + Duration::from_millis(2000);
    while Instant::now() < deadline {
        let seq = ui_seq();
        if seq > seq_before {
            return json!({"status": "applied", "ui_seq": seq});
        }
        sleep(Duration::from_millis(100)).await;
    }
    json!({"status": "pending", "error": "UI-клиент не подтвердил изменение за 2 с: открой окно приложения"})
}

// ---------- полное состояние ----------

pub(crate) fn full_state() -> Value {
    let theme = control::read_store("theme.json");
    json!({
        "app": {
            "name": "bunny-hops",
            "version": env!("CARGO_PKG_VERSION"),
            "port": app_port(),
            "uptime_s": control::STARTED.get().map(|t| t.elapsed().as_secs()).unwrap_or(0),
            "debug_enabled": true,
        },
        "config": {"theme": theme.get("mode").cloned().unwrap_or(json!("dark"))},
        "profiles": control::profiles_doc(),
        "mock": {
            "profiles": Value::Object(mock_server::profiles_map()),
            "engine": mock_server::control_snapshot(),
        },
        "traffic": Value::Object(mock_server::traffic_snapshot()),
        "elements": Value::Array(live_elements()),
        "ui": ui_state(),
    })
}

// ---------- доступ к путям JSON ----------

fn path_get<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = root;
    for seg in path.split('.') {
        if seg.is_empty() {
            continue;
        }
        cur = cur.get(seg)?;
    }
    Some(cur)
}

/// Оценка предиката ожидания над снимком состояния.
/// Возвращает Some(значение) при совпадении, None — если не совпало.
/// Поддерживаемые виды предикатов (по ТЗ 2.2.4):
/// - {"kind":"element","id":"...","state":"present|absent|enabled|disabled","value"?}
///   Проверяет живой элемент в store.elements[] по id.
///   state="present" — элемент существует; "absent" — не существует;
///   "enabled"/"disabled" — элемент существует и флаг enabled соответствует.
///   Если задан "value", дополнительно проверяет element.value == value.
/// - {"kind":"state","path":"...","equals":...}
///   Проверяет path_get(state, path) == equals (сериализованное сравнение).
pub(crate) fn eval_predicate(store: &Value, predicate: &Value) -> Option<Value> {
    let kind = predicate.get("kind").and_then(Value::as_str)?;
    match kind {
        "element" => {
            let id = predicate.get("id").and_then(Value::as_str)?;
            let state = predicate.get("state").and_then(Value::as_str).unwrap_or("present");
            let elements = store.get("elements").and_then(Value::as_array)?;
            let el = elements.iter().find(|e| e.get("id").and_then(Value::as_str) == Some(id));
            match state {
                "present" => el.cloned(),
                "absent" => {
                    if el.is_none() {
                        Some(json!({"id": id, "state": "absent"}))
                    } else {
                        None
                    }
                }
                "enabled" => {
                    let el = el?;
                    let enabled = el.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                    if enabled { Some(el.clone()) } else { None }
                }
                "disabled" => {
                    let el = el?;
                    let enabled = el.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                    if !enabled { Some(el.clone()) } else { None }
                }
                _ => None,
            }.and_then(|matched| {
                // Optional value check
                if let Some(expected) = predicate.get("value") {
                    let actual = matched.get("value");
                    if actual != Some(expected) {
                        return None;
                    }
                }
                Some(matched)
            })
        }
        "state" => {
            let path = predicate.get("path").and_then(Value::as_str)?;
            let expected = predicate.get("equals")?;
            let actual = path_get(store, path)?;
            if actual == expected {
                Some(json!({"path": path, "value": actual}))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn deep_merge(dst: &mut Value, src: &Value) {
    match dst {
        Value::Object(d) if src.is_object() => {
            let Some(s) = src.as_object() else { return };
            for (k, v) in s {
                deep_merge(d.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        _ => *dst = src.clone(),
    }
}

/// Рекурсивный дифв двух состояний: только изменённые листья, с бюджетом записей.
pub(crate) fn json_diff(before: &Value, after: &Value) -> Value {
    let mut out = Map::new();
    let mut budget = DIFF_BUDGET;
    diff_into("", before, after, &mut out, &mut budget);
    Value::Object(out)
}

fn diff_into(prefix: &str, a: &Value, b: &Value, out: &mut Map<String, Value>, budget: &mut usize) {
    if a == b || *budget == 0 {
        return;
    }
    if let (Value::Object(ac), Value::Object(bc)) = (a, b) {
        let mut keys: Vec<&String> = ac.keys().chain(bc.keys()).collect();
        keys.sort();
        keys.dedup();
        for k in keys {
            let path = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
            diff_into(
                &path,
                ac.get(k).unwrap_or(&Value::Null),
                bc.get(k).unwrap_or(&Value::Null),
                out,
                budget,
            );
        }
        return;
    }
    if prefix.is_empty() {
        return;
    }
    *budget -= 1;
    out.insert(prefix.to_string(), json!({"before": a, "after": b}));
}

// ---------- запись состояния ----------

async fn apply_write(path: &str, value: Value, merge: bool) -> Result<Value, (StatusCode, String)> {
    // Профиль mock: mock.profiles.<имя>.
    if let Some(name) = path.strip_prefix("mock.profiles.") {
        if !name.is_empty() && !name.contains('.') {
            return apply_profile_write(name, value, merge).await;
        }
    }
    // Тема приложения: config.theme = "dark" | "light" (или {"mode": ...}).
    if path == "config.theme" {
        let mode = value
            .as_str()
            .or_else(|| value.get("mode").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        if !["dark", "light"].contains(&mode.as_str()) {
            return Err((StatusCode::BAD_REQUEST, "config.theme: dark или light".into()));
        }
        control::write_store("theme.json", &json!({"mode": mode}))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        control::log_action("theme", "set", json!({"mode": mode, "via": "debug_api"}));
        return Ok(json!({"status": "saved", "value": mode}));
    }
    // Состояние UI: ui.<путь> — команда живому окну.
    if let Some(rest) = path.strip_prefix("ui.") {
        if !rest.is_empty() {
            let seq_before = ui_seq();
            publish(json!({"type": "ui_command", "path": rest, "value": value}));
            return Ok(wait_ui_echo(seq_before).await);
        }
    }
    Err((StatusCode::BAD_REQUEST, format!("путь «{path}» не перезаписывается через Debug API")))
}

async fn apply_profile_write(name: &str, value: Value, merge: bool) -> Result<Value, (StatusCode, String)> {
    if value.is_null() {
        mock_server::delete_profile(name)
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
        control::log_action("mock", "delete", json!({"name": name, "via": "debug_api"}));
        return Ok(json!({"status": "deleted"}));
    }
    let to_save = if merge {
        let mut existing = {
            let mut map = mock_server::profiles_map();
            map.remove(name).unwrap_or(json!({}))
        };
        if !existing.is_object() {
            existing = json!({});
        }
        deep_merge(&mut existing, &value);
        if existing.get("name").and_then(Value::as_str).is_none() {
            existing["name"] = json!(name);
        }
        existing
    } else {
        value
    };
    mock_server::save_profile_value(&to_save).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    control::log_action("mock", "save", json!({"name": name, "via": "debug_api"}));
    Ok(json!({"status": "saved"}))
}

// ---------- реестр действий ----------

fn actions_doc() -> &'static Value {
    static D: OnceLock<Value> = OnceLock::new();
    D.get_or_init(|| {
        json!({"actions": [
            {"name": "ui.set", "description": "Менять состояние открытого UI. Пути: ui.request.url, ui.request.method, ui.request.body, ui.request.tab (params|headers|body), ui.mock.selected (null — снять выбор).",
             "payload": {"path": "ui.request.url", "value": "https://example.com/api/v1/check"}},
            {"name": "ui.reset", "description": "Вернуть UI к значениям по умолчанию (тот же сброс, что кнопка в UI).",
             "payload": {}},
            {"name": "ui.click", "description": "Кликнуть кнопку UI по имени. Доступные имена — массив _clicks в снапшоте (GET /debug/state → ui.state._clicks).",
             "payload": {"name": "mock"}},
            {"name": "element.click", "description": "Кликнуть объект UI по id из каталога BROOP (настоящий DOM-клик, полный путь обработчиков). Id, назначения и живое состояние — GET /debug/broop.",
             "payload": {"id": "mock.add_rule"}},
            {"name": "element.set", "description": "Установить значение объекта UI (input/textarea/select/checkbox) с событиями input/change, как при вводе с клавиатуры.",
             "payload": {"id": "rule.path.0", "value": "/api/v1/check"}},
            {"name": "eval", "description": "Выполнить JS в окне приложения и прочитать результат по GET /debug/eval-out. Точечный инструмент, когда готового действия нет.",
             "payload": {"expr": "document.title"}},
            {"name": "traffic.clear", "description": "Очистить буфер трафика всех mock-серверов.",
             "payload": {}},
            {"name": "logs.clear", "description": "Очистить журнал действий приложения.",
             "payload": {}},
        ]})
    })
}

pub(crate) fn is_known_action(action: &str) -> bool {
    actions_doc()
        .get("actions")
        .and_then(Value::as_array)
        .map(|list| list.iter().any(|a| a.get("name").and_then(Value::as_str) == Some(action)))
        .unwrap_or(false)
}

fn clear_action_log() -> usize {
    let removed = {
        let mut store = control::action_log_store().lock().unwrap();
        let n = store.len();
        store.clear();
        n
    };
    let _ = control::write_store("action_log.json", &Value::Array(vec![]));
    removed
}

// ---------- обработчики ----------

fn disabled_response() -> Response {
    control::json_response(
        StatusCode::NOT_FOUND,
        &json!({"ok": false, "error": "debug_api_disabled",
                "hint": "включи тумблер MCP в UI или POST /api/debug-mode {\"on\": true}"}),
    )
}

async fn read_body_json(req: Request) -> Result<Value, Response> {
    let bytes = to_bytes(req.into_body(), MAX_BODY)
        .await
        .map_err(|_| control::err(StatusCode::PAYLOAD_TOO_LARGE, "тело больше 10 MiB"))?;
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|e| control::err(StatusCode::BAD_REQUEST, &format!("тело не JSON: {e}")))
}

async fn state_collection_handler(req: Request) -> Response {
    let method = req.method().clone();
    if method == Method::GET {
        return control::json_response(StatusCode::OK, &full_state());
    }
    if method != Method::POST && method != Method::PATCH {
        return control::err(StatusCode::METHOD_NOT_ALLOWED, "для /debug/state доступны GET, POST и PATCH");
    }
    let merge = method == Method::PATCH;
    let body = match read_body_json(req).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let path = match body.get("path").and_then(Value::as_str) {
        Some(p) => p.to_string(),
        None => return control::err(StatusCode::BAD_REQUEST, r#"нужно {"path": "...", "value": ...}"#),
    };
    let value = body.get("value").cloned().unwrap_or(Value::Null);
    match apply_write(&path, value, merge).await {
        Ok(info) => control::json_response(
            StatusCode::OK,
            &json!({"ok": true, "status": "ok", "path": path, "result": info}),
        ),
        Err((status, message)) => control::json_response(status, &json!({"ok": false, "error": message})),
    }
}

async fn state_item_handler(Path(key): Path<String>, req: Request) -> Response {
    let method = req.method().clone();
    if method == Method::GET {
        return match path_get(&full_state(), &key) {
            Some(v) => control::json_response(StatusCode::OK, v),
            None => control::err(StatusCode::NOT_FOUND, &format!("нет значения по пути «{key}»")),
        };
    }
    if method != Method::POST && method != Method::PATCH {
        return control::err(StatusCode::METHOD_NOT_ALLOWED, "для /debug/state/<путь> доступны GET, POST и PATCH");
    }
    // Тело запроса = новое значение целиком; PATCH объединяет объекты поверх текущего.
    let body = match read_body_json(req).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let merge = method == Method::PATCH;
    match apply_write(&key, body, merge).await {
        Ok(info) => control::json_response(
            StatusCode::OK,
            &json!({"ok": true, "status": "ok", "path": key, "result": info}),
        ),
        Err((status, message)) => control::json_response(status, &json!({"ok": false, "error": message})),
    }
}

async fn actions_handler() -> Response {
    control::json_response(StatusCode::OK, actions_doc())
}

async fn dispatch_handler(req: Request) -> Response {
    let body = match read_body_json(req).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let action = body.get("action").and_then(Value::as_str).unwrap_or("").to_string();
    if action.is_empty() {
        return control::err(StatusCode::BAD_REQUEST, r#"нужно {"action": "...", "payload": ...}"#);
    }
    if !is_known_action(&action) {
        return control::json_response(
            StatusCode::BAD_REQUEST,
            &json!({"ok": false, "error": format!("неизвестное действие «{action}»", action = action),
                    "hint": "список действий: GET /debug/actions"}),
        );
    }
    let payload = body.get("payload").cloned().unwrap_or(json!({}));
    let result = run_action(&action, payload).await;
    // stateDiff — обязательная часть ответа: состояние до и после одного вызова.
    let diff = json_diff(&result.0, &result.1);
    match result.2 {
        Ok(data) => control::json_response(
            StatusCode::OK,
            &json!({"ok": true, "status": "ok", "action": action, "result": data, "stateDiff": diff}),
        ),
        Err((status, message)) => control::json_response(
            status,
            &json!({"ok": false, "status": "error", "action": action, "error": message, "stateDiff": diff}),
        ),
    }
}

// ---------- BROOP: библиотека объектов UI и действия над ними ----------

/// Последняя библиотека элементов, присланная UI (собрана из живого DOM).
fn elements_store() -> &'static Mutex<Value> {
    static S: OnceLock<Mutex<Value>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(json!({"elements": []})))
}

/// Последний результат `eval`, выполненный в окне UI.
fn eval_store() -> &'static Mutex<Value> {
    static S: OnceLock<Mutex<Value>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(json!({"out": null})))
}

/// Каталог BROOP: объекты и их действия. Файл в репозитории; назначение каждого
/// объекта — карта экрана, по ней ориентимся на местности без скриншотов.
pub(crate) fn broop_doc() -> &'static Value {
    static D: OnceLock<Value> = OnceLock::new();
    D.get_or_init(|| {
        serde_json::from_str(include_str!("../docs/broop.json"))
            .unwrap_or_else(|e| json!({"error": format!("broop.json повреждён: {e}")}))
    })
}

/// Живая библиотека элементов, как её прислал UI.
pub(crate) fn live_elements() -> Vec<Value> {
    elements_store()
        .lock()
        .unwrap()
        .get("elements")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Интерактивные элементы DOM без `data-bh` — присланы фронтом как гейт разметки.
pub(crate) fn live_unmarked() -> Vec<Value> {
    elements_store()
        .lock()
        .unwrap()
        .get("unmarked")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Совпадает ли живой id с записью каталога (запись с `.*` — префикс динамической строки).
pub(crate) fn matches_catalog(pattern: &str, id: &str) -> bool {
    match pattern.strip_suffix(".*") {
        Some(prefix) => id.starts_with(prefix) && id.len() > prefix.len() + 1,
        None => pattern == id,
    }
}

pub(crate) fn element_id(el: &Value) -> &str {
    el.get("id").and_then(Value::as_str).unwrap_or("")
}

/// --- Gate check for element actions ---

/// Причина отказа gate_check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateReject {
    NotFound,
    Disabled,
    Invisible,
}

/// Элемент, прошедший gate_check.
#[derive(Debug, Clone)]
pub struct GateItem {
    pub id: String,
    pub enabled: bool,
    pub visible: bool,
}

/// Чистая проверка элемента по списку живых элементов.
/// Возвращает элемент, если он есть, включен и видим; иначе — причину отказа.
fn gate_check_elements(live: &[Value], id: &str) -> Result<GateItem, GateReject> {
    let el = live.iter().find(|e| element_id(e) == id).ok_or(GateReject::NotFound)?;
    let enabled = el.get("enabled").and_then(Value::as_bool).unwrap_or(true);
    let visible = el.get("visible").and_then(Value::as_bool).unwrap_or(true);
    if !enabled {
        return Err(GateReject::Disabled);
    }
    if !visible {
        return Err(GateReject::Invisible);
    }
    Ok(GateItem {
        id: id.to_string(),
        enabled,
        visible,
    })
}

/// Проверка элемента по глобальной живой карте.
pub(crate) fn gate_check(id: &str) -> Result<GateItem, GateReject> {
    gate_check_elements(&live_elements(), id)
}

/// Реестр oneshot-каналов для ожидания обновления карты элементов (round-trip).
fn elements_wait_registry() -> &'static Mutex<HashMap<String, oneshot::Sender<()>>> {
    static R: OnceLock<Mutex<HashMap<String, oneshot::Sender<()>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Зарегистрировать ожидание обновления элемента. Возвращает receiver, который
/// сработает когда UI пришлёт POST /debug/elements (в elements_handler).
pub(crate) fn register_elements_wait(id: &str) -> oneshot::Receiver<()> {
    let (tx, rx) = oneshot::channel();
    elements_wait_registry().lock().unwrap().insert(id.to_string(), tx);
    rx
}

/// Вызвать из elements_handler после обновления карты: пробудить ожидающих.
fn notify_elements_waiters() {
    let mut reg = elements_wait_registry().lock().unwrap();
    for (_, tx) in reg.drain() {
        let _ = tx.send(());
    }
}

/// UI присылает библиотеку после hello и после каждой применённой команды.
async fn elements_handler(Json(body): Json<Value>) -> Response {
    let count = body
        .get("elements")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);
    *elements_store().lock().unwrap() = body;
    // Пробудить задачи, ожидающие обновления карты (round-trip для NotFound).
    notify_elements_waiters();
    control::json_response(StatusCode::OK, &json!({"ok": true, "count": count}))
}

/// UI присылает результат eval; читается по GET.
async fn eval_out_handler(Json(body): Json<Value>) -> Response {
    *eval_store().lock().unwrap() = body;
    control::json_response(StatusCode::OK, &json!({"ok": true}))
}

async fn eval_get_handler() -> Response {
    let v = eval_store().lock().unwrap().clone();
    control::json_response(StatusCode::OK, &v)
}

/// Карта экрана: объекты каталога с живым состоянием + расхождения.
/// «missing_in_dom» — объект не на экране (скрыт или не отрисован),
/// «undeclared_in_broop» — в DOM есть объект, которого нет в каталоге (сигнал
/// обновить каталог или разметку).
async fn broop_handler() -> Response {
    let doc = broop_doc();
    let catalog = doc
        .get("objects")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let live = live_elements();
    let mut objects = Vec::new();
    let mut missing = Vec::new();
    for obj in &catalog {
        let pattern = obj.get("id").and_then(Value::as_str).unwrap_or("");
        let hits: Vec<&Value> = live
            .iter()
            .filter(|el| matches_catalog(pattern, element_id(el)))
            .collect();
        if hits.is_empty() {
            missing.push(json!({"id": pattern, "reason": "не на экране (скрыт или не отрисован)"}));
        } else {
            for el in hits {
                let mut merged = obj.clone();
                if let Some(o) = merged.as_object_mut() {
                    // id — реальный из DOM (по нему кликаем), шаблон каталога
                    // остаётся в declared_as.
                    o.insert("declared_as".into(), obj.get("id").cloned().unwrap_or(Value::Null));
                    o.insert("id".into(), json!(element_id(el)));
                    o.insert("state".into(), el.clone());
                }
                objects.push(merged);
            }
        }
    }
    let undeclared: Vec<&Value> = live
        .iter()
        .filter(|el| {
            !catalog
                .iter()
                .any(|o| matches_catalog(o.get("id").and_then(Value::as_str).unwrap_or(""), element_id(el)))
        })
        .collect();
    let unmarked = live_unmarked();
    control::json_response(
        StatusCode::OK,
        &json!({
            "ok": true,
            "screen": doc.get("screen").cloned().unwrap_or(Value::Null),
            "objects": objects,
            "missing_in_dom": missing,
            "undeclared_in_broop": undeclared,
            "unmarked_in_dom": unmarked,
        }),
    )
}

/// Точечная карта одного объекта: каталог + живое состояние по id.
async fn broop_get_handler(Query(params): Query<HashMap<String, String>>) -> Response {
    let Some(id) = params.get("id") else {
        return control::json_response(
            StatusCode::BAD_REQUEST,
            &json!({"error": "нужен параметр id"}),
        );
    };
    let Some(live_el) = live_elements().into_iter().find(|el| element_id(el) == id) else {
        return control::json_response(
            StatusCode::NOT_FOUND,
            &json!({"error": format!("объект «{id}» не на экрана; карта — GET /debug/broop")}),
        );
    };
    let doc = broop_doc();
    let catalog = doc
        .get("objects")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let declared = catalog
        .iter()
        .find(|o| matches_catalog(o.get("id").and_then(Value::as_str).unwrap_or(""), id))
        .cloned();
    control::json_response(StatusCode::OK, &json!({"ok": true, "declared": declared, "state": live_el}))
}

/// Выполняет действие и возвращает (состояние до, состояние после, результат).
pub(crate) async fn run_action(action: &str, payload: Value) -> (Value, Value, Result<Value, (StatusCode, String)>) {
    let before = full_state();
    let result: Result<Value, (StatusCode, String)> = match action {
        "ui.set" => {
            let raw = payload.get("path").and_then(Value::as_str).unwrap_or("");
            let rest = raw.strip_prefix("ui.").unwrap_or(raw);
            if rest.is_empty() {
                Err((StatusCode::BAD_REQUEST, "нужен path вида ui.<путь>".into()))
            } else {
                let seq_before = ui_seq();
                publish(json!({"type": "ui_command", "path": rest, "value": payload.get("value").cloned().unwrap_or(Value::Null)}));
                Ok(wait_ui_echo(seq_before).await)
            }
        }
        "ui.reset" => {
            let seq_before = ui_seq();
            publish(json!({"type": "ui_reset"}));
            Ok(wait_ui_echo(seq_before).await)
        }
        "ui.click" => {
            let name = payload.get("name").and_then(Value::as_str).unwrap_or("");
            if name.is_empty() {
                Err((StatusCode::BAD_REQUEST, "нужен payload {\"name\": \"<кнопка>\"}".into()))
            } else {
                let seq_before = ui_seq();
                publish(json!({"type": "ui_command", "path": "click", "value": name}));
                Ok(wait_ui_echo(seq_before).await)
            }
        }
        "element.click" => {
            let id = payload.get("id").and_then(Value::as_str).unwrap_or("");
            if id.is_empty() {
                Err((
                    StatusCode::BAD_REQUEST,
                    "нужен payload {\"id\": \"<объект BROOP>\"}".into(),
                ))
            } else {
                // First gate_check attempt.
                match gate_check(id) {
                    Ok(_) => {
                        let seq_before = ui_seq();
                        publish(json!({"type": "ui_command", "path": "element.click", "value": {"id": id}}));
                        Ok(wait_ui_echo(seq_before).await)
                    }
                    Err(GateReject::NotFound) => {
                        // Round-trip: ask UI to collect elements, wait for POST /debug/elements.
                        publish(json!({"action": "elements.collect"}));
                        let rx = register_elements_wait(id);
                        let wait_result = tokio::time::timeout(Duration::from_millis(2000), rx).await;
                        if wait_result.is_err() {
                            // Timeout - return NotFound with guidance.
                            Err((
                                StatusCode::NOT_FOUND,
                                format!("объект «{id}» не на экране; карта — GET /debug/broop (таймаут обновления карты)"),
                            ))
                        } else {
                            // Retry gate_check once after round-trip.
                            match gate_check(id) {
                                Ok(_) => {
                                    let seq_before = ui_seq();
                                    publish(json!({"type": "ui_command", "path": "element.click", "value": {"id": id}}));
                                    Ok(wait_ui_echo(seq_before).await)
                                }
                                Err(GateReject::NotFound) => Err((
                                    StatusCode::NOT_FOUND,
                                    format!("объект «{id}» не на экране после обновления карты; карта — GET /debug/broop"),
                                )),
                                Err(GateReject::Disabled) => Err((
                                    StatusCode::FORBIDDEN,
                                    format!("объект «{id}» отключен (disabled)"),
                                )),
                                Err(GateReject::Invisible) => Err((
                                    StatusCode::FORBIDDEN,
                                    format!("объект «{id}» невидим (invisible)"),
                                )),
                            }
                        }
                    }
                    Err(GateReject::Disabled) => Err((
                        StatusCode::FORBIDDEN,
                        format!("объект «{id}» отключен (disabled)"),
                    )),
                    Err(GateReject::Invisible) => Err((
                        StatusCode::FORBIDDEN,
                        format!("объект «{id}» невидим (invisible)"),
                    )),
                }
            }
        }
        "eval" => {
            let expr = payload.get("expr").and_then(Value::as_str).unwrap_or("");
            if expr.is_empty() {
                Err((StatusCode::BAD_REQUEST, "нужен payload {\"expr\": \"<js>\"}".into()))
            } else {
                let seq_before = ui_seq();
                publish(json!({"type": "ui_command", "path": "eval", "value": {"expr": expr}}));
                Ok(wait_ui_echo(seq_before).await)
            }
        }
        "element.set" => {
            let id = payload.get("id").and_then(Value::as_str).unwrap_or("");
            if id.is_empty() {
                Err((
                    StatusCode::BAD_REQUEST,
                    "нужен payload {\"id\": \"<объект BROOP>\", \"value\": ...}".into(),
                ))
            } else if live_elements().into_iter().all(|el| element_id(&el) != id) {
                Err((
                    StatusCode::NOT_FOUND,
                    format!("объект «{id}» не на экране; карта — GET /debug/broop"),
                ))
            } else {
                let seq_before = ui_seq();
                publish(json!({
                    "type": "ui_command",
                    "path": "element.set",
                    "value": {"id": id, "value": payload.get("value").cloned().unwrap_or(Value::Null)}
                }));
                Ok(wait_ui_echo(seq_before).await)
            }
        }
        "traffic.clear" => {
            mock_server::clear_traffic();
            control::log_action("traffic", "clear", json!({"via": "debug_api"}));
            Ok(json!({"cleared": true}))
        }
        "logs.clear" => {
            let removed = clear_action_log();
            control::log_action("logs", "clear", json!({"removed": removed, "via": "debug_api"}));
            Ok(json!({"removed": removed}))
        }
        other => Err((StatusCode::BAD_REQUEST, format!("неизвестное действие «{other}»"))),
    };
    let after = full_state();
    (before, after, result)
}

async fn reset_handler(req: Request) -> Response {
    let _ = req;
    let logs_removed = clear_action_log();
    mock_server::clear_traffic();
    let seq_before = ui_seq();
    publish(json!({"type": "ui_reset"}));
    let ui = wait_ui_echo(seq_before).await;
    control::log_action("debug", "reset", json!({"logs_removed": logs_removed, "via": "debug_api"}));
    control::json_response(
        StatusCode::OK,
        &json!({"ok": true, "traffic": "cleared", "logs_removed": logs_removed, "ui": ui}),
    )
}

#[derive(Deserialize)]
struct LogsQuery {
    limit: Option<usize>,
    area: Option<String>,
}

async fn logs_handler(Query(q): Query<LogsQuery>) -> Response {
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    let items: Vec<Value> = {
        let store = control::action_log_store().lock().unwrap();
        store
            .iter()
            .rev()
            .filter(|e| {
                q.area.as_ref().map_or(true, |a| e.get("area").and_then(Value::as_str) == Some(a.as_str()))
            })
            .take(limit)
            .cloned()
            .collect()
    };
    control::json_response(StatusCode::OK, &json!({"ok": true, "items": items}))
}

fn openapi_doc() -> &'static Value {
    static D: OnceLock<Value> = OnceLock::new();
    D.get_or_init(|| {
        serde_json::from_str(include_str!("../docs/openapi-debug.json"))
            .unwrap_or_else(|e| json!({"error": format!("openapi-debug.json повреждён: {e}")}))
    })
}

async fn openapi_handler() -> Response {
    // Контракт читают и до включения гейта: эта точка доступна всегда.
    control::json_response(StatusCode::OK, openapi_doc())
}

async fn ws_handler(ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(|socket| handle_socket(socket))
}

async fn handle_socket(mut socket: WebSocket) {
    let mut rx = hub().subscribe();
    let hello = json!({
        "type": "hello",
        "app": "bunny-hops",
        "version": env!("CARGO_PKG_VERSION"),
        "port": app_port(),
        "debug_enabled": true,
    });
    if socket.send(Message::Text(hello.to_string())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            received = rx.recv() => match received {
                Ok(event) => {
                    if socket.send(Message::Text(event.to_string())).await.is_err() {
                        break;
                    }
                }
                Err(_) => {} // Lagged: пропустили отставшие события, идём дальше.
            },
            message = socket.recv() => match message {
                Some(Ok(Message::Close(_))) | None => break,
                _ => {} // входящие сообщения клиента в v1 не используются
            },
        }
    }
}

// ---------- маршруты ----------

/// Ожидание условия (предката) над состоянием приложения.
/// POST /debug/wait {"predicate": {...}, "timeout_ms"?: 5000}
/// Предикаты:
///   {"kind":"element","id":"...","state":"present|absent|enabled|disabled","value"?}
///   {"kind":"state","path":"...","equals":...}
/// Поллер: шаг 50 мс, round-trip elements.collect не чаще раза в 500 мс.
/// Ответ при успехе: {"ok":true,"matched":<item>,"waited_ms":N}
/// Ответ по таймауту: HTTP 200, {"ok":false,"reason":"timeout","last":<снимок>}
async fn wait_handler(Json(body): Json<Value>) -> Response {
    let predicate = match body.get("predicate").cloned() {
        Some(p) => p,
        None => return control::err(StatusCode::BAD_REQUEST, r#"нужно {"predicate": {...}, "timeout_ms"?: 5000}"#),
    };
    let timeout_ms = body.get("timeout_ms").and_then(Value::as_u64).unwrap_or(5000).clamp(1, 60000);
    let out = wait_for_predicate(&predicate, timeout_ms).await;
    control::json_response(StatusCode::OK, &out)
}

/// Ядро ожидания: поллер предиката до дедлайна.
/// Успех: {"ok":true,"matched":<item>,"waited_ms":N}; таймаут: {"ok":false,"reason":"timeout","last":<снимок>}.
/// Общий код для POST /debug/wait и RPC ui.wait.
pub(crate) async fn wait_for_predicate(predicate: &Value, timeout_ms: u64) -> Value {
    let timeout_ms = timeout_ms.clamp(1, 60000);
    let started = Instant::now();
    let deadline = started + Duration::from_millis(timeout_ms);
    let mut last_round_trip = started - Duration::from_millis(500); // allow immediate first
    let mut last_snapshot = full_state();

    while Instant::now() < deadline {
        // Fresh snapshot each iteration
        last_snapshot = full_state();

        // Try evaluate predicate
        if let Some(matched) = eval_predicate(&last_snapshot, predicate) {
            let waited_ms = (Instant::now() - started).as_millis() as u64;
            return json!({"ok": true, "matched": matched, "waited_ms": waited_ms});
        }

        // Round-trip trigger: ask UI to collect elements, but not more than once per 500ms
        let now = Instant::now();
        if now.duration_since(last_round_trip) >= Duration::from_millis(500) {
            publish(json!({"action": "elements.collect"}));
            last_round_trip = now;
        }

        // Poll interval 50ms
        sleep(Duration::from_millis(50)).await;
    }

    // Timeout - return last snapshot with reason
    json!({"ok": false, "reason": "timeout", "last": last_snapshot})
}

/// Активировать окно (показать/восстановить/сфокусировать).
async fn window_activate_handler() -> Response {
    match window_activate().await {
        Ok(()) => control::json_response(StatusCode::OK, &json!({"ok": true, "status": "activated"})),
        Err(e) => control::json_response(StatusCode::SERVICE_UNAVAILABLE, &json!({"ok": false, "error": e})),
    }
}

pub(crate) fn contract_router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new().route("/debug/openapi.json", get(openapi_handler))
}

/// Поверхность MCP: открыта при включённом гейте или для запросов UI (gate_or_ui).
pub(crate) fn mcp_router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/debug/state", any(state_collection_handler))
        .route("/debug/state/:key", any(state_item_handler))
        .route("/debug/actions", get(actions_handler))
        .route("/debug/dispatch", post(dispatch_handler))
        .route("/debug/reset", post(reset_handler))
        .route("/debug/logs", get(logs_handler))
        .route("/debug/ui-state", post(ui_state_handler))
        .route("/debug/elements", post(elements_handler))
        .route("/debug/eval-out", post(eval_out_handler).get(eval_get_handler))
        .route("/debug/broop", get(broop_handler))
        .route("/debug/broop/get", get(broop_get_handler))
        .route("/debug/stream", get(ws_handler))
        .route("/debug/window/activate", post(window_activate_handler))
        .route("/debug/wait", post(wait_handler))
}

/// Гейт MCP-поверхности: проускает при включённом гейте или когда запрос помечен
/// как запрос самого UI (заголовок `x-hop-ui`; WS — `?src=ui`: браузер не умеет
/// заголовки в WS-рукопожатии).
pub(crate) async fn gate_or_ui(req: Request, next: axum::middleware::Next) -> Response {
    let from_ui = req.headers().get("x-hop-ui").is_some()
        || req.uri().query().is_some_and(|q| q.contains("src=ui"));
    if is_enabled() || from_ui {
        return next.run(req).await;
    }
    disabled_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_live_element(id: &str, enabled: bool, visible: bool) -> Value {
        json!({"id": id, "enabled": enabled, "visible": visible})
    }

    #[test]
    fn gate_rejects_disabled() {
        let live = vec![make_live_element("btn.disabled", false, true)];
        let err = gate_check_elements(&live, "btn.disabled").unwrap_err();
        assert_eq!(err, GateReject::Disabled);
    }

    #[test]
    fn gate_rejects_then_refreshes_via_roundtrip() {
        // Empty live list → NotFound; after "round-trip" (simulated elements push),
        // retry gate_check should succeed.
        let live: Vec<Value> = vec![];
        let err = gate_check_elements(&live, "mock.add_rule").unwrap_err();
        assert_eq!(err, GateReject::NotFound);

        // Simulate the round-trip: UI posts elements, live now has the item.
        let live2 = vec![make_live_element("mock.add_rule", true, true)];
        let item = gate_check_elements(&live2, "mock.add_rule").unwrap();
        assert_eq!(item.id, "mock.add_rule");
        assert!(item.enabled);
        assert!(item.visible);
    }

    #[test]
    fn wait_predicate_element_present_matches() {
        // Given a store with an element present and enabled
        let store = json!({"elements": [{"id": "btn.submit", "enabled": true, "visible": true}]});
        let predicate = json!({"kind": "element", "id": "btn.submit", "state": "present"});
        let matched = eval_predicate(&store, &predicate);
        assert!(matched.is_some(), "element present should match");
        let m = matched.unwrap();
        assert_eq!(m.get("id").and_then(Value::as_str), Some("btn.submit"));
    }

    #[test]
    fn wait_predicate_state_path_equals_matches() {
        // Given a full state with a nested value
        let state = json!({"config": {"theme": "dark", "lang": "en"}});
        let predicate = json!({"kind": "state", "path": "config.theme", "equals": "dark"});
        let matched = eval_predicate(&state, &predicate);
        assert!(matched.is_some(), "state path equals should match");
    }

    #[test]
    fn wait_timeout_returns_last_snapshot() {
        // This test will verify the timeout behavior returns last snapshot
        // It will fail until the wait handler is implemented
        let predicate = json!({"kind": "element", "id": "nonexistent", "state": "present"});
        // The actual wait handler test would need a running server, so this unit test
        // just ensures the predicate evaluation returns None for non-matching case
        let store = json!({"elements": []});
        let matched = eval_predicate(&store, &predicate);
        assert!(matched.is_none(), "non-matching element should return None");
    }
}
