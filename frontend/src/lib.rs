pub mod app;
mod actlog;
mod method_select;
mod pill_nav;
mod search;
mod steps_util;
mod storage;
mod timefmt;
mod types;
mod util;
mod http_status;
mod mock_view;
mod mock_nav;
mod logic;

mod stream;
mod ui;
mod debug_bridge;
mod debug_elements;

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(app::App);
}
