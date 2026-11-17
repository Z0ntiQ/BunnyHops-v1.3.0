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

pub fn load_scale_idx() -> usize {
    ls_get(LS_SCALE_IDX)
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|v| *v < SCALES.len())
        .unwrap_or(DEFAULT_SCALE_IDX)
}
