//! Debug-мост: WS-клиент `/debug/stream` + реестр живых сигналов UI.
//!
//! Бэкенд шлёт команды `ui_command`/`ui_reset`; мост применяет их к зарегистрированным
//! сигналам и подтверждает состояние снапшотом на `POST /debug/ui-state`.
//! Реестр — thread-local: WASM однопоточный, все колбэки живут в одном потоке.

use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{MessageEvent, Response, WebSocket};

// ---------- реестр состояний ----------
//
// Колбэки хранятся как Rc и вызываются вне заимствования RefCell: обработчик может
// синхронно перерисовать UI, и новый компонент зарегистрирует свой колбэк — повторное
// заимствование в этот момент паниковало и убивало весь wasm (инцидент 18.11.2026,
// «Новый профиль ничего не делает»).

thread_local! {
    static SETTERS: RefCell<HashMap<String, Rc<dyn Fn(Value)>>> = RefCell::new(HashMap::new());
    static SNAPSHOTS: RefCell<Vec<(u64, String, Rc<dyn Fn() -> Value>)>> = const { RefCell::new(Vec::new()) };
    static RESETS: RefCell<Vec<(u64, Rc<dyn Fn()>)>> = const { RefCell::new(Vec::new()) };
    static CLICKS: RefCell<HashMap<String, Rc<dyn Fn()>>> = RefCell::new(HashMap::new());
    static NEXT_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

fn next_id() -> u64 {
    NEXT_ID.with(|c| {
        let id = c.get();
        c.set(id + 1);
        id
    })
}

/// Зарегистрировать запись значения UI по пути команды (например `request.url`).
pub fn register_setter(path: &str, setter: impl Fn(Value) + 'static) {
    SETTERS.with(|s| s.borrow_mut().insert(path.to_string(), Rc::new(setter)));
}

/// Убрать сеттер пути (при размонтировании вьюхи, чьи сигналы он читал).
pub fn unregister_setter(path: &str) {
    SETTERS.with(|s| s.borrow_mut().remove(path));
}

/// Убрать обработчик клика по имени (при размонтировании его вьюхи).
pub fn unregister_click(name: &str) {
    CLICKS.with(|c| c.borrow_mut().remove(name));
}

/// Зарегистрировать вклад в снапшот: объект под ключом `prefix`.
/// Возвращает id для `unregister_snapshot` — вьюха обязана сняться при
/// размонтировании, иначе снапшот читает уже удалённые сигналы (инцидент
/// 18.11.2026: закрытие вкладки Mock останавливало снимки и карту BROOP).
pub fn register_snapshot(prefix: &str, build: impl Fn() -> Value + 'static) -> u64 {
    let id = next_id();
    SNAPSHOTS.with(|s| s.borrow_mut().push((id, prefix.to_string(), Rc::new(build))));
    id
}

pub fn unregister_snapshot(id: u64) {
    SNAPSHOTS.with(|s| s.borrow_mut().retain(|(sid, _, _)| *sid != id));
}

/// Зарегистрировать сброс секции UI для команды `ui_reset`; id — для снятия.
pub fn register_reset(reset: impl Fn() + 'static) -> u64 {
    let id = next_id();
    RESETS.with(|r| r.borrow_mut().push((id, Rc::new(reset))));
    id
}

pub fn unregister_reset(id: u64) {
    RESETS.with(|r| r.borrow_mut().retain(|(rid, _)| *rid != id));
}

/// Зарегистрировать клик кнопки UI по имени (команда `click` от действия `ui.click`).
pub fn register_click(name: &str, action: impl Fn() + 'static) {
    CLICKS.with(|c| c.borrow_mut().insert(name.to_string(), Rc::new(action)));
}

fn run_click(name: &str) -> bool {
    let action = CLICKS.with(|c| c.borrow().get(name).cloned());
    match action {
        Some(action) => {
            action();
            true
        }
        None => false,
    }
}

fn click_names() -> Vec<Value> {
    let mut names: Vec<String> = CLICKS.with(|c| c.borrow().keys().cloned().collect());
    names.sort();
    names.into_iter().map(Value::String).collect()
}

fn snapshot() -> Value {
    let mut root = serde_json::Map::new();
    let parts: Vec<(u64, String, Rc<dyn Fn() -> Value>)> =
        SNAPSHOTS.with(|parts| parts.borrow().clone());
    for (_, name, build) in parts {
        // Защита от паники: забытая регистрация после размонтирования читает
        // удалённый сигнал; такой вклад пропускаем, остальной снапшот жив.
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build()));
        match built {
            Ok(v) => {
                if let Some(obj) = v.as_object() {
                    for (k, v) in obj.clone() {
                        root.insert(k, v);
                    }
                }
            }
            Err(_) => {
                root.insert(
                    format!("_stale_{name}"),
                    Value::String("removed after unmount".into()),
                );
            }
        }
    }
    Value::Object(root)
}

fn snapshot_with_clicks() -> Value {
    let mut v = snapshot();
    if let Some(obj) = v.as_object_mut() {
        obj.insert("_clicks".into(), Value::Array(click_names()));
    }
    v
}

fn apply_command(path: &str, value: Value) -> bool {
    let setter = SETTERS.with(|s| s.borrow().get(path).cloned());
    let Some(setter) = setter else {
        return false;
    };
    setter(value);
    true
}

fn run_resets() {
    let resets: Vec<(u64, Rc<dyn Fn()>)> = RESETS.with(|r| r.borrow().clone());
    for (_, reset) in resets {
        reset();
    }
}

// ---------- HTTP для моста ----------

pub async fn api_get(path: &str) -> Result<Value, String> {
    let promise = crate::util::ui_fetch(path, None, None)?;
    let resp_val = JsFuture::from(promise)
        .await
        .map_err(|e| format!("{e:?}"))?;
    let resp: Response = resp_val.dyn_into().map_err(|e| format!("{e:?}"))?;
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    parse_json_response(resp).await
}

pub async fn api_post(path: &str, body: &Value) -> Result<Value, String> {
    let promise = crate::util::ui_fetch(path, Some("POST"), Some(&body.to_string()))?;
    let resp_val = JsFuture::from(promise)
        .await
        .map_err(|e| format!("{e:?}"))?;
    let resp: Response = resp_val.dyn_into().map_err(|e| format!("{e:?}"))?;
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    parse_json_response(resp).await
}

async fn parse_json_response(resp: Response) -> Result<Value, String> {
    let text = resp.text().map_err(|e| format!("{e:?}"))?;
    let value = JsFuture::from(text).await.map_err(|e| format!("{e:?}"))?;
    let text = value.as_string().unwrap_or_default();
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

async fn post_snapshot() {
    let _ = api_post("/debug/ui-state", &snapshot_with_clicks()).await;
    post_elements().await;
}

/// Отдать библиотеку элементов (BROOP). Перерисовка после команды идёт
/// микрозадачей, а панели открываются асинхронным запросом, поэтому
/// отправляем дважды: сразу и ещё раз после паузы.
async fn post_elements() {
    delay(50).await;
    let body = serde_json::json!({
        "elements": crate::debug_elements::collect(),
        "unmarked": crate::debug_elements::unmarked(),
    });
    let _ = api_post("/debug/elements", &body).await;
    delay(400).await;
    let body = serde_json::json!({
        "elements": crate::debug_elements::collect(),
        "unmarked": crate::debug_elements::unmarked(),
    });
    let _ = api_post("/debug/elements", &body).await;
}

// ---------- разбор событий ----------

fn handle_message(text: &str) {
    let Ok(message) = serde_json::from_str::<Value>(text) else {
        return;
    };
    match message.get("type").and_then(Value::as_str).unwrap_or("") {
        "hello" => spawn_local(post_snapshot()),
        "ui_command" => {
            let path = message.get("path").and_then(Value::as_str).unwrap_or("");
            let value = message.get("value").cloned().unwrap_or(Value::Null);
            if path == "click" {
                if run_click(value.as_str().unwrap_or("")) {
                    spawn_local(post_snapshot());
                }
            } else if path == "element.click" {
                let id = value.get("id").and_then(Value::as_str).unwrap_or("");
                if crate::debug_elements::click(id) {
                    spawn_local(post_snapshot());
                }
            } else if path == "element.set" {
                let id = value.get("id").and_then(Value::as_str).unwrap_or("");
                let val = value.get("value").cloned().unwrap_or(Value::Null);
                if crate::debug_elements::set_value(id, &val) {
                    spawn_local(post_snapshot());
                }
            } else if path == "eval" {
                // Выполнить JS в окне; результат уходит на /debug/eval-out.
                let expr = value.get("expr").and_then(Value::as_str).unwrap_or("");
                let out = crate::debug_elements::eval(expr);
                spawn_local(async move {
                    let body = serde_json::json!({ "out": out });
                    let _ = api_post("/debug/eval-out", &body).await;
                });
                spawn_local(post_snapshot());
            } else if apply_command(path, value) {
                spawn_local(post_snapshot());
            }
        }
        "ui_reset" => {
            run_resets();
            spawn_local(post_snapshot());
        }
        _ => {} // action/ui_snapshot/hint — в UI не отражаются
    }
}

// ---------- WS-клиент ----------

static ACTIVE: AtomicBool = AtomicBool::new(false);
static LOOP_STARTED: AtomicBool = AtomicBool::new(false);

/// Включить/выключить постоянное соединение с `/debug/stream`.
pub fn set_active(on: bool) {
    ACTIVE.store(on, Ordering::Relaxed);
    if !on {
        return;
    }
    if LOOP_STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    spawn_local(async move {
        loop {
            if ACTIVE.load(Ordering::Relaxed) {
                let _ = connection().await;
            } else {
                delay(1000).await;
            }
            // Пауза перед переподключением: сервер мог перезапуститься или гейт мог выключиться.
            delay(2000).await;
        }
    });
}

/// Пауза для кода вне модуля (например цикла опроса в UI).
pub async fn sleep_ms(ms: i32) {
    delay(ms).await;
}

async fn delay(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let callback = Closure::once_into_js(move || {
            let _ = resolve.call0(&JsValue::NULL);
        });
        let _ = web_sys::window().unwrap().set_timeout_with_callback_and_timeout_and_arguments_0(
            callback.unchecked_ref::<js_sys::Function>(),
            ms,
        );
    });
    let _ = JsFuture::from(promise).await;
}

async fn push_elements_now() {
    let body = serde_json::json!({
        "elements": crate::debug_elements::collect(),
        "unmarked": crate::debug_elements::unmarked(),
    });
    let _ = api_post("/debug/elements", &body).await;
}

async fn connection() -> Result<(), JsValue> {
    let win = web_sys::window().ok_or("no window")?;
    let loc = win.location();
    let proto = if loc.protocol().unwrap_or_default() == "https:" { "wss" } else { "ws" };
    let url = format!("{proto}://{}/debug/stream?src=ui", loc.host().unwrap_or_default());
    let ws = WebSocket::new(&url)?;

    let opened = js_sys::Promise::new(&mut |resolve, reject| {
        let ok = resolve.clone();
        let err = reject.clone();
        let onopen = Closure::once_into_js(move || {
            let _ = ok.call0(&JsValue::NULL);
        });
        let onerror = Closure::once_into_js(move |e: JsValue| {
            let _ = err.call1(&JsValue::NULL, &e);
        });
        ws.set_onopen(Some(onopen.unchecked_ref()));
        ws.set_onerror(Some(onerror.unchecked_ref()));
    });
    JsFuture::from(opened).await?;

    let onmessage = Closure::wrap(Box::new(move |event: MessageEvent| {
        if let Some(text) = event.data().as_string() {
            handle_message(&text);
        }
    }) as Box<dyn FnMut(MessageEvent)>);
    ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));

    let closed = js_sys::Promise::new(&mut |resolve, _reject| {
        let done = resolve.clone();
        let ws_close = ws.clone();
        let onclose = Closure::once_into_js(move || {
            // Снимаем обработчики до выхода: колбэки освобождаются вместе с соединением.
            ws_close.set_onmessage(None);
            ws_close.set_onopen(None);
            ws_close.set_onerror(None);
            ws_close.set_onclose(None);
            let _ = done.call0(&JsValue::NULL);
        });
        ws.set_onclose(Some(onclose.unchecked_ref()));
    });

    // Пульс карты: библиотека элементов уходит раз в 2 секунды, не завися от
    // команд — панель BROOP остаётся свежей сама, даже если командный путь
    // пуша завис.
    let pulse_ws = ws.clone();
    let cb = Closure::wrap(Box::new(move || {
        if pulse_ws.ready_state() == WebSocket::OPEN {
            spawn_local(push_elements_now());
        }
    }) as Box<dyn FnMut()>);
    let pulse_id = win.set_interval_with_callback_and_timeout_and_arguments_0(
        cb.as_ref().unchecked_ref(),
        2000,
    )?;
    cb.forget();

    // Первый снапшот сразу после рукопожатия: backend читает ui.* сразу после hello.
    post_snapshot().await;
    let _ = JsFuture::from(closed).await;
    let _ = win.clear_interval_with_handle(pulse_id);
    Ok(())
}
