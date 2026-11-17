//! search: вынесено из app.rs (атомарные модули, правило пользователя 13.11.2026).

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;


/* ---------- живой поиск по экрану (реализация — window.hopSearch в index.html) ---------- */

pub fn hop_search(q: &str) -> i64 {
    let Some(win) = web_sys::window() else { return 0 };
    let Ok(f) = js_sys::Reflect::get(&win, &JsValue::from_str("hopSearch")) else { return 0 };
    let Ok(f) = f.dyn_into::<js_sys::Function>() else { return 0 };
    match f.call1(&JsValue::NULL, &JsValue::from_str(q)) {
        Ok(v) => v.as_f64().map(|x| x as i64).unwrap_or(0),
        Err(_) => 0,
    }
}
