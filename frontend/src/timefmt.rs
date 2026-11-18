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

/// Единый логер: событие UI уходит в серверный журнал (POST /debug/log).
pub async fn post_log(msg: &str) {
    let body = serde_json::json!({"msg": msg}).to_string();
    let Ok(promise) = crate::util::ui_fetch("/debug/log", Some("POST"), Some(&body)) else {
        return;
    };
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}
