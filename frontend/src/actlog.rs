//! actlog: вынесено из app.rs (атомарные модули, правило пользователя 13.11.2026).

use wasm_bindgen::JsCast;
use crate::timefmt::ts_at;


#[derive(serde::Deserialize)]
pub struct LogItem {
    pub(crate) ts: u64,
    pub(crate) area: String,
    pub(crate) action: String,
    #[serde(default)]
    pub(crate) details: serde_json::Value,
}

pub fn render_log_item(it: &LogItem) -> String {
    if it.area == "ui" && it.action == "note" {
        let msg = it.details.get("msg").and_then(serde_json::Value::as_str).unwrap_or("");
        return format!("{} {}", ts_at(it.ts), msg);
    }
    let det = match &it.details {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, v)| format!("{}={}", k, v.as_str().unwrap_or(&v.to_string())))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    };
    format!("{} [{}] {} {}", ts_at(it.ts), it.area, it.action, det)
        .trim_end()
        .to_string()
}

/// Серверный журнал действий, старые первыми (панель показывает реверсом).
pub async fn fetch_action_log() -> Vec<String> {
    let win = match web_sys::window() {
        Some(w) => w,
        None => return Vec::new(),
    };
    let promise = win.fetch_with_str("/api/action-log");
    let Ok(val) = wasm_bindgen_futures::JsFuture::from(promise).await else {
        return Vec::new();
    };
    let Ok(resp) = val.dyn_into::<web_sys::Response>() else {
        return Vec::new();
    };
    let Ok(text_promise) = resp.text() else {
        return Vec::new();
    };
    let Ok(txt) = wasm_bindgen_futures::JsFuture::from(text_promise).await else {
        return Vec::new();
    };
    #[derive(serde::Deserialize)]
    struct LogResp {
        items: Vec<LogItem>,
    }
    let Ok(doc) = serde_json::from_str::<LogResp>(&txt.as_string().unwrap_or_default()) else {
        return Vec::new();
    };
    let mut lines: Vec<String> = doc.items.iter().map(render_log_item).collect();
    lines.reverse();
    lines
}
