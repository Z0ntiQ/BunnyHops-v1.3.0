//! timefmt: вынесено из app.rs (атомарные модули, правило пользователя 13.11.2026).

use wasm_bindgen::prelude::*;


pub fn ts_short() -> String {
    let d = js_sys::Date::new_0();
    format!("{:02}:{:02}:{:02}", d.get_hours(), d.get_minutes(), d.get_seconds())
}

pub fn fmt_ts(ts: u64) -> String {
    let d = js_sys::Date::new(&JsValue::from_f64(ts as f64 * 1000.0));
    js_sys::Date::to_time_string(&d).into()
}

/// Время ЧЧ:ММ:СС из серверного unix-времени (для панели единого журнала).
pub fn ts_at(ts: u64) -> String {
    let d = js_sys::Date::new(&JsValue::from_f64(ts as f64 * 1000.0));
    format!("{:02}:{:02}:{:02}", d.get_hours(), d.get_minutes(), d.get_seconds())
}

/// Единый логер: событие UI уходит в серверный журнал (POST /api/log).
pub async fn post_log(msg: &str) {
    let win = match web_sys::window() {
        Some(w) => w,
        None => return,
    };
    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_body(&JsValue::from_str(
        &serde_json::json!({"msg": msg}).to_string(),
    ));
    let Ok(req) = web_sys::Request::new_with_str_and_init("/api/log", &init) else {
        return;
    };
    let promise = win.fetch_with_request(&req);
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}
