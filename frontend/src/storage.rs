//! storage: вынесено из app.rs (атомарные модули, правило пользователя 13.11.2026).

use wasm_bindgen::JsCast;
use crate::logic::*;
use crate::types::*;

pub const LS_STEPS: &str = "hoplite.steps";
pub const LS_VARS: &str = "hoplite.vars";
pub const LS_SCALE_IDX: &str = "hoplite.scaleIdx";
pub const LS_LOG_H: &str = "hoplite.logH";
pub const LS_TRF_H: &str = "hoplite.trfH";
pub const LS_SAVED: &str = "hoplite.saved";
pub const LS_GROUPS: &str = "hoplite.groups";

/* ---------- localStorage ---------- */

pub fn ls_get(key: &str) -> Option<String> {
    web_sys::window()?
        .local_storage()
        .ok()??
        .get_item(key)
        .ok()?
}

/// Максимальная высота панели журнала: от нижнего края окна (отступ 30px)
/// до нижней границы панели табов запроса (.tabs).
pub fn tab_bottom_gap() -> Option<i32> {
    let win = web_sys::window()?;
    let doc = win.document()?;
    let inner_h = win.inner_height().ok()?.as_f64()? as i32;
    // 17.11.2026: ориентир — низ поля URL (раньше был .tabs, его больше нет).
    let el = doc.query_selector(".url-input").ok().flatten()?;
    let rect = el.get_bounding_client_rect();
    Some((inner_h - 30 - rect.bottom() as i32).max(60))
}

pub(crate) fn ls_set(key: &str, value: &str) {
    if let Some(Some(store)) = web_sys::window().and_then(|w| w.local_storage().ok()) {
        let _ = store.set_item(key, value);
    }
}

pub fn ls_del(key: &str) {
    if let Some(Some(store)) = web_sys::window().and_then(|w| w.local_storage().ok()) {
        let _ = store.remove_item(key);
    }
}

pub fn load_steps() -> Vec<Step> {
    ls_get(LS_STEPS)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn load_vars() -> Vec<KvRow> {
    ls_get(LS_VARS)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn load_saved() -> Vec<SavedReq> {
    ls_get(LS_SAVED)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn load_groups() -> Vec<SavedGroup> {
    ls_get(LS_GROUPS)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub async fn fetch_api_calls() -> Vec<ApiCall> {
    let win = match web_sys::window() {
        Some(w) => w,
        None => return Vec::new(),
    };
    let promise = win.fetch_with_str("/api/automation-rules");
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
    serde_json::from_str(&txt.as_string().unwrap_or_default()).unwrap_or_default()
}
