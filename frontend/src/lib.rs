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
mod mock_view;
mod logic;

mod stream;
mod ui;

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(app::App);
}
