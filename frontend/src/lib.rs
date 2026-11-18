// Конвенция Leptos: функции-компоненты — PascalCase (fn IconAlpha() и т. п.),
// snake_case для них не применим — стандартный allow из шаблона Leptos.
#![allow(non_snake_case)]

mod actlog;
pub mod app;
mod http_status;
mod logic;
mod method_select;
mod mock_nav;
mod mock_view;
mod pill_nav;
mod search;
mod steps_util;
mod storage;
mod timefmt;
mod types;
mod util;

mod debug_bridge;
mod debug_elements;
mod stream;
mod ui;

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(app::App);
}
