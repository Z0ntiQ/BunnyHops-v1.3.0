pub mod app;
mod mock_view;
mod logic;

mod stream;
mod ui;

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(app::App);
}
