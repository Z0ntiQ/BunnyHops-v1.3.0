//! Плоскость полного управления приложением (control plane).
//!
//! Цель: полностью автоматизировать взаимодействие с Bunny Hops из скриптов
//! и агентов — всё, что делает UI, доступно через REST на этом же порту.
//! Хранилище — state/*.json рядом с бинарником (тот же принцип, что mock/).
//! Аутентификации нет: сервер слушает localhost.

use axum::body::{to_bytes, Body};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::AppState;

const STATE_DIR_NAME: &str = "state";
const HISTORY_CAP: usize = 200;
// Журнал переживает перезапуск (пишется в action_log.json), поэтому держим
// историю с запасом: 200 записей кончались за один активный прогон.
const ACTION_LOG_CAP: usize = 1000;
const MAX_BODY: usize = 10 * 1024 * 1024;

pub static STARTED: OnceLock<Instant> = OnceLock::new();

// ---------- хранилище ----------

fn state_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("BUNNY_HOPS_STATE_DIR") {
        return dir.into();
    }
    let base = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let nested = base.join(STATE_DIR_NAME);
    if nested.is_dir() {
        nested
    } else {
        base
    }
}

pub(crate) fn read_store(file: &str) -> Value {
    fs::read_to_string(state_dir().join(file))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

pub(crate) fn write_store(file: &str, value: &Value) -> Result<(), String> {
    let dir = state_dir();
    let _ = fs::create_dir_all(&dir);
    fs::write(
        dir.join(file),
        serde_json::to_string_pretty(value).unwrap_or_default(),
    )
    .map_err(|e| format!("не удалось записать {file}: {e}"))
}

pub(crate) fn history_store() -> &'static Mutex<Vec<Value>> {
    static H: OnceLock<Mutex<Vec<Value>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(load_history()))
}

fn load_history() -> Vec<Value> {
    match read_store("history.json") {
        Value::Array(items) => items,
        _ => Vec::new(),
    }
}

pub(crate) fn action_log_store() -> &'static Mutex<Vec<Value>> {
    static L: OnceLock<Mutex<Vec<Value>>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(load_action_log()))
}

fn load_action_log() -> Vec<Value> {
    match read_store("action_log.json") {
        Value::Array(items) => items,
        _ => Vec::new(),
    }
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn push_capped(store: &Mutex<Vec<Value>>, rec: Value, cap: usize) {
    let mut log = store.lock().unwrap();
    log.push(rec);
    if log.len() > cap {
        let excess = log.len() - cap;
        log.drain(0..excess);
    }
}

/// Единый логер действий приложения. Через него проходят ВСЕ действия:
/// мутации control-плоскости, прокси-вызовы, mock-движок, транспорт API
/// (см. `api_access_middleware`) и заметки UI (`POST /api/log`).
/// Запись сразу уходит в action_log.json: журнал виден и после перезапуска.
pub fn log_action(area: &str, action: &str, details: Value) {
    let snapshot = {
        let store = action_log_store();
        push_capped(
            store,
            json!({"ts": now_epoch(), "area": area, "action": action, "details": details.clone()}),
            ACTION_LOG_CAP,
        );
        store.lock().unwrap().clone()
    };
    let _ = write_store("action_log.json", &Value::Array(snapshot));
    // Шина отладки: каждое содержательное действие видно внешним WS-клиентам.
    crate::debug_api::publish_action(area, action, &details);
}

/// Запись без деталей: для событий оболочки (запуск окна, закрытие).
pub fn log_simple(area: &str, action: &str) {
    log_action(area, action, json!({}));
}

// ---------- транспортное логирование ----------
//
// Правило пользователя 17.11.2026: каждая функция приложения покрыта
// логированием. Средство — этот middleware: он видит ВСЕ маршруты, включая
// будущие. Правила тишины, чтобы журнал оставался полезным:
// * опросы панели (POLL_GETS) при успешном статусе не пишутся — они каждые 2-4 с;
// * маршруты с семантической записью (SEMANTIC) при успехе не дублируются —
//   содержательная запись богаче; ошибка на таком маршруте пишется всегда.

const POLL_GETS: &[&str] = &[
    "/debug/logs",
    "/api/history",
    "/debug/mock/traffic",
    "/debug/mock/status",
    "/api/status",
    "/api/profiles",
    "/debug/mock/profiles",
    "/debug/mock/profile",
    "/debug/theme",
    "/api/automation-rules",
];

// Пульс карты BROOP: UI шлёт библиотеку элементов каждые 2 с (debug_bridge).
// Рутинный фоновый POST, смысловой ценности в журнале нет — как POLL_GETS.
// Ошибки на этих путях пишутся всегда (условие !success в мидлваре).
const PULSE_POSTS: &[&str] = &["/debug/elements"];

const SEMANTIC: &[&str] = &[
    "/proxy",
    "/debug/request",
    "/api/profiles",
    "/debug/profiles/activate",
    "/debug/profiles/delete",
    "/debug/theme",
    "/debug/history/clear",
    "/api/log",
    "/debug/mock/profile",
    "/debug/mock/profile/delete",
    "/debug/mock/start",
    "/debug/mock/stop",
    "/api/debug-mode",
];

/// Журналирует каждый запрос к API: метод, путь, статус, длительность.
/// Статика и fallback SPA проходят транзитом (путь вне области API).
pub async fn api_access_middleware(req: Request, next: axum::middleware::Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let started = Instant::now();
    let response = next.run(req).await;
    let in_scope =
        path.starts_with("/api/") || path.starts_with("/debug/") || path == "/screenshot";
    if !in_scope {
        return response;
    }
    let status = response.status();
    let success = status.is_success();
    let polled = method == Method::GET && POLL_GETS.contains(&path.as_str());
    let pulsed = method == Method::POST && PULSE_POSTS.contains(&path.as_str());
    let semantic = SEMANTIC.contains(&path.as_str());
    if !success || !(polled || pulsed || semantic) {
        log_action(
            "api",
            &format!("{method} {path}"),
            json!({"status": status.as_u16(), "ms": started.elapsed().as_millis() as u64}),
        );
    }
    response
}

pub(crate) fn json_response(status: StatusCode, body: &Value) -> Response {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json; charset=utf-8")
        .header("Cache-Control", "no-store")
        .body(Body::from(serde_json::to_vec(body).unwrap_or_default()))
        .unwrap()
}

pub(crate) fn err(status: StatusCode, message: &str) -> Response {
    json_response(status, &json!({"ok": false, "error": message}))
}

// Response в axum заведомо крупный (внутри Body), это штатная цена ошибки в обработчике.
#[allow(clippy::result_large_err)]
async fn read_json_body(req: Request) -> Result<Value, Response> {
    let bytes = to_bytes(req.into_body(), MAX_BODY)
        .await
        .map_err(|e| err(StatusCode::PAYLOAD_TOO_LARGE, &format!("тело запроса: {e}")))?;
    if bytes.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| err(StatusCode::BAD_REQUEST, &format!("ожидался JSON: {e}")))
}

// ---------- приёмник действий UI ----------

#[derive(Deserialize)]
struct NoteIn {
    msg: String,
}

async fn log_post_handler(State(_state): State<AppState>, req: Request) -> Response {
    let body = match read_json_body(req).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let input: NoteIn = match serde_json::from_value(body) {
        Ok(v) => v,
        Err(e) => {
            return err(
                StatusCode::BAD_REQUEST,
                &format!("{{\"msg\": \"...\"}}: {e}"),
            )
        }
    };
    log_action("ui", "note", json!({"msg": input.msg}));
    json_response(StatusCode::OK, &json!({"ok": true}))
}

// ---------- профили ----------

#[derive(Deserialize)]
struct ProfileIn {
    name: String,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    headers: Option<serde_json::Map<String, Value>>,
}

pub(crate) fn profiles_doc() -> Value {
    match read_store("profiles.json") {
        v @ Value::Object(_) => v,
        _ => json!({"profiles": []}),
    }
}

fn profiles_list(doc: &Value) -> Vec<Value> {
    doc.get("profiles")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

async fn profiles_list_handler(State(_state): State<AppState>) -> Response {
    let doc = profiles_doc();
    let active = doc.get("active").and_then(Value::as_str).unwrap_or("");
    json_response(
        StatusCode::OK,
        &json!({"ok": true, "active": active, "profiles": profiles_list(&doc)}),
    )
}

async fn profiles_post_handler(State(_state): State<AppState>, req: Request) -> Response {
    let body = match read_json_body(req).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let input: ProfileIn = match serde_json::from_value(body) {
        Ok(v) => v,
        Err(e) => return err(StatusCode::BAD_REQUEST, &format!("профиль: {e}")),
    };
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return err(StatusCode::BAD_REQUEST, "нужно непустое имя профиля");
    }
    let mut doc = profiles_doc();
    let mut list = profiles_list(&doc);
    let entry = json!({
        "name": name,
        "base_url": input.base_url.unwrap_or_default(),
        "headers": input.headers.unwrap_or_default(),
        "updated_at": now_epoch(),
    });
    let mut created = true;
    if let Some(pos) = list
        .iter()
        .position(|p| p.get("name").and_then(Value::as_str) == Some(&name))
    {
        list[pos] = entry.clone();
        created = false;
    } else {
        list.push(entry.clone());
    }
    doc["profiles"] = Value::Array(list);
    if let Err(e) = write_store("profiles.json", &doc) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, &e);
    }
    log_action(
        "profiles",
        if created { "create" } else { "update" },
        json!({"name": name}),
    );
    json_response(
        StatusCode::OK,
        &json!({"ok": true, "created": created, "profile": entry}),
    )
}

async fn profiles_activate_handler(State(_state): State<AppState>, req: Request) -> Response {
    let body = match read_json_body(req).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let mut doc = profiles_doc();
    if !name.is_empty()
        && !profiles_list(&doc)
            .iter()
            .any(|p| p.get("name").and_then(Value::as_str) == Some(&name))
    {
        return err(
            StatusCode::NOT_FOUND,
            &format!("профиль «{name}» не найден"),
        );
    }
    doc["active"] = json!(name);
    if let Err(e) = write_store("profiles.json", &doc) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, &e);
    }
    log_action("profiles", "activate", json!({"name": name}));
    json_response(StatusCode::OK, &json!({"ok": true, "active": name}))
}

async fn profiles_delete_handler(State(_state): State<AppState>, req: Request) -> Response {
    let body = match read_json_body(req).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return err(StatusCode::BAD_REQUEST, "нужно имя профиля");
    }
    let mut doc = profiles_doc();
    let before = profiles_list(&doc).len();
    let kept: Vec<Value> = profiles_list(&doc)
        .into_iter()
        .filter(|p| p.get("name").and_then(Value::as_str) != Some(&name))
        .collect();
    if kept.len() == before {
        return err(
            StatusCode::NOT_FOUND,
            &format!("профиль «{name}» не найден"),
        );
    }
    doc["profiles"] = Value::Array(kept);
    if doc.get("active").and_then(Value::as_str) == Some(&name) {
        doc["active"] = json!("");
    }
    if let Err(e) = write_store("profiles.json", &doc) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, &e);
    }
    log_action("profiles", "delete", json!({"name": name}));
    json_response(StatusCode::OK, &json!({"ok": true}))
}

// ---------- выполнение запроса ----------

#[derive(Deserialize)]
struct RequestIn {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    query: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    headers: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    body: Option<Value>,
    #[serde(default)]
    timeout_s: Option<u64>,
}

fn resolve_target(input: &RequestIn) -> Result<String, String> {
    if let Some(url) = input
        .url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
    {
        return Ok(url.to_string());
    }
    let doc = profiles_doc();
    let wanted = input
        .profile
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| doc.get("active").and_then(Value::as_str).unwrap_or(""));
    if wanted.is_empty() {
        return Err("нужен url или профиль (активный профиль не выбран)".to_string());
    }
    let profile = profiles_list(&doc)
        .into_iter()
        .find(|p| p.get("name").and_then(Value::as_str) == Some(wanted))
        .ok_or_else(|| format!("профиль «{wanted}» не найден"))?;
    let base = profile
        .get("base_url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .ok_or_else(|| format!("у профиля «{wanted}» пустой base_url"))?;
    let mut target = base.trim_end_matches('/').to_string();
    if let Some(path) = input
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        target.push('/');
        target.push_str(path.trim_start_matches('/'));
    }
    if let Some(query) = &input.query {
        let pairs: Vec<String> = query
            .iter()
            .map(|(k, v)| {
                let value = v
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| v.to_string());
                format!("{k}={value}")
            })
            .collect();
        if !pairs.is_empty() {
            target.push('?');
            target.push_str(&pairs.join("&"));
        }
    }
    Ok(target)
}

async fn request_post_handler(State(state): State<AppState>, req: Request) -> Response {
    let body = match read_json_body(req).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let input: RequestIn = match serde_json::from_value(body) {
        Ok(v) => v,
        Err(e) => return err(StatusCode::BAD_REQUEST, &format!("запрос: {e}")),
    };
    let target = match resolve_target(&input) {
        Ok(t) => t,
        Err(e) => return err(StatusCode::BAD_REQUEST, &e),
    };
    let method = Method::from_bytes(
        input
            .method
            .as_deref()
            .unwrap_or("GET")
            .to_uppercase()
            .as_bytes(),
    )
    .map_err(|_| ())
    .unwrap_or(Method::GET);

    let client = state.client.clone();
    let timeout = Duration::from_secs(input.timeout_s.unwrap_or(60).min(300));
    let mut builder = client.request(method.clone(), &target).timeout(timeout);
    if let Some(headers) = &input.headers {
        let mut map = HeaderMap::new();
        for (k, v) in headers {
            if let (Ok(name), Ok(value)) = (
                axum::http::HeaderName::from_bytes(k.as_bytes()),
                axum::http::HeaderValue::from_str(v.to_string().trim_matches('"')),
            ) {
                map.insert(name, value);
            }
        }
        builder = builder.headers(map);
    }
    if let Some(body) = &input.body {
        let payload = match body {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        builder = builder.body(payload);
    }

    let started = Instant::now();
    let outcome = match builder.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let headers: serde_json::Map<String, Value> = resp
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), json!(v.to_str().unwrap_or("<bytes>"))))
                .collect();
            let bytes = resp.bytes().await.unwrap_or_default();
            let text = String::from_utf8_lossy(&bytes).to_string();
            json!({
                "ok": status < 400,
                "status": status,
                "headers": headers,
                "body": text,
                "final_url": target,
                "elapsed_ms": started.elapsed().as_millis() as u64,
            })
        }
        Err(e) => json!({
            "ok": false,
            "status": 0,
            "error": format!("сбой запроса: {e}"),
            "final_url": target,
            "elapsed_ms": started.elapsed().as_millis() as u64,
        }),
    };

    push_capped(
        history_store(),
        json!({
            "ts": now_epoch(),
            "method": method.as_str(),
            "url": target,
            "profile": input.profile,
            "status": outcome.get("status").cloned().unwrap_or(Value::Null),
            "elapsed_ms": outcome.get("elapsed_ms").cloned().unwrap_or(Value::Null),
        }),
        HISTORY_CAP,
    );
    log_action(
        "request",
        "send",
        json!({"method": method.as_str(), "url": target, "status": outcome.get("status")}),
    );
    let status = if outcome.get("error").is_some() {
        StatusCode::BAD_GATEWAY
    } else {
        StatusCode::OK
    };
    json_response(status, &outcome)
}

async fn history_get_handler(State(_state): State<AppState>) -> Response {
    let items: Vec<Value> = history_store()
        .lock()
        .unwrap()
        .iter()
        .rev()
        .cloned()
        .collect();
    json_response(
        StatusCode::OK,
        &json!({"ok": true, "items": items, "total": items.len()}),
    )
}

async fn history_clear_handler(State(_state): State<AppState>) -> Response {
    let mut log = history_store().lock().unwrap();
    let cleared = log.len();
    log.clear();
    drop(log);
    let _ = write_store("history.json", &Value::Array(Vec::new()));
    log_action("history", "clear", json!({"removed": cleared}));
    json_response(StatusCode::OK, &json!({"ok": true, "removed": cleared}))
}

// ---------- тема ----------

async fn theme_get_handler(State(_state): State<AppState>) -> Response {
    let doc = read_store("theme.json");
    let mode = doc
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("dark")
        .to_string();
    json_response(StatusCode::OK, &json!({"ok": true, "mode": mode}))
}

async fn theme_post_handler(State(_state): State<AppState>, req: Request) -> Response {
    let body = match read_json_body(req).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let mode = body
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if !["dark", "light"].contains(&mode.as_str()) {
        return err(StatusCode::BAD_REQUEST, "mode должен быть dark или light");
    }
    if let Err(e) = write_store("theme.json", &json!({"mode": mode})) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, &e);
    }
    log_action("theme", "set", json!({"mode": mode}));
    json_response(StatusCode::OK, &json!({"ok": true, "mode": mode}))
}

// ---------- гейт API отладки ----------

/// Вернуть текущее состояние debug-гейта.
async fn debug_mode_get_handler() -> impl axum::response::IntoResponse {
    axum::Json(json!({ "ok": true, "enabled": crate::debug_api::is_enabled() }))
}

async fn debug_mode_post_handler(State(_state): State<AppState>, req: Request) -> Response {
    let body = match read_json_body(req).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let on = match body.get("on").and_then(Value::as_bool) {
        Some(v) => v,
        None => return err(StatusCode::BAD_REQUEST, r#"нужно {"on": true|false}"#),
    };
    crate::debug_api::set_enabled(on);
    log_action("debug", "mode", json!({"on": on, "via": "api"}));
    json_response(StatusCode::OK, &json!({"ok": true, "enabled": on}))
}

// ---------- журнал действий и статус ----------

async fn status_get_handler(State(_state): State<AppState>) -> Response {
    let doc = profiles_doc();
    let uptime = STARTED.get().map(|t| t.elapsed().as_secs()).unwrap_or(0);
    json_response(
        StatusCode::OK,
        &json!({
            "ok": true,
            "app": "bunny-hops",
            "version": env!("CARGO_PKG_VERSION"),
            "uptime_s": uptime,
            "state_dir": state_dir().display().to_string(),
            "active_profile": doc.get("active").and_then(Value::as_str).unwrap_or(""),
            "profiles_count": profiles_list(&doc).len(),
            "history_count": history_store().lock().unwrap().len(),
            "action_log_count": action_log_store().lock().unwrap().len(),
            "mock_engine": crate::mock_server::control_snapshot(),
            "debug_api": {"enabled": crate::debug_api::is_enabled()},
        }),
    )
}

// ---------- маршруты ----------

/// Минимальный API без гейта: здоровье приложения и переключатель MCP.
pub fn control_router() -> Router<AppState> {
    Router::new()
        .route("/api/status", get(status_get_handler))
        .route(
            "/api/debug-mode",
            axum::routing::post(debug_mode_post_handler).get(debug_mode_get_handler),
        )
}

/// Функциональная поверхность MCP (под гейтом debug_api::gate_or_ui).
pub fn mcp_router() -> Router<AppState> {
    Router::new()
        .route("/debug/request", post(request_post_handler))
        .route("/debug/history", get(history_get_handler))
        .route("/debug/history/clear", post(history_clear_handler))
        .route("/debug/profiles", get(profiles_list_handler))
        .route("/debug/profiles", post(profiles_post_handler))
        .route("/debug/profiles/activate", post(profiles_activate_handler))
        .route("/debug/profiles/delete", post(profiles_delete_handler))
        .route("/debug/theme", get(theme_get_handler))
        .route("/debug/theme", post(theme_post_handler))
        .route("/debug/log", post(log_post_handler))
}
