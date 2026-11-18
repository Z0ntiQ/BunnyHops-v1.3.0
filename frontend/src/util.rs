//! util: мелкие хелперы, вынесено из app.rs (атомарные модули, правило пользователя 13.11.2026).
//! js_now/new_id не дублируем — они уже есть в ui.rs.

use crate::logic::*;
use crate::storage::*;

pub fn sleep_ms_local(ms: i32) -> impl std::future::Future<Output = ()> {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let _ = web_sys::window()
            .expect("no window")
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
    });
    async move {
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
}

/// fetch с пометкой «запрос из UI» (заголовок `x-hop-ui`): сервер пропускает
/// такие запросы сквозь MCP-гейт — приложение работает и при выключенном MCP.
pub fn ui_fetch(
    url: &str,
    method: Option<&str>,
    body: Option<&str>,
) -> Result<js_sys::Promise, String> {
    let win = web_sys::window().ok_or("no window")?;
    let init = web_sys::RequestInit::new();
    if let Some(m) = method {
        init.set_method(m);
    }
    let headers = web_sys::Headers::new().map_err(|_| "headers")?;
    headers.set("x-hop-ui", "1").map_err(|_| "x-hop-ui")?;
    headers
        .set("Content-Type", "application/json")
        .map_err(|_| "content-type")?;
    init.set_headers(&wasm_bindgen::JsValue::from(&headers));
    if let Some(b) = body {
        init.set_body(&wasm_bindgen::JsValue::from_str(b));
    }
    let req = web_sys::Request::new_with_str_and_init(url, &init).map_err(|_| "request")?;
    Ok(win.fetch_with_request(&req))
}

pub fn load_scale_idx() -> usize {
    ls_get(LS_SCALE_IDX)
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|v| *v < SCALES.len())
        .unwrap_or(DEFAULT_SCALE_IDX)
}
