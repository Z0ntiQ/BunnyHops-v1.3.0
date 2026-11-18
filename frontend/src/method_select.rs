//! method_select: вынесено из app.rs (атомарные модули, правило пользователя 13.11.2026).

use leptos::prelude::*;
use crate::ui::METHODS;



/* ---------- цветной выпадающий список методов — стиль Spring Dropdown ---------- */
/// Цвет точки-маркера метода (тёмный фон, яркие тона как в референсе).
pub fn method_dot_color(m: &str) -> &'static str {
    match m {
        "GET" => "#21c363",
        "POST" => "#ff9f0a",
        "PUT" => "#3ec9ff",
        "PATCH" => "#c69bff",
        "DELETE" => "#ff5a5f",
        "HEAD" => "#7aa2c9",
        _ => "#9a9a9a",
    }
}

#[component]
pub fn MethodSelect(
    get_method: Callback<(), String>,
    set_method: Callback<String>,
) -> impl IntoView {
    let open = create_rw_signal(false);
    let current = move || get_method.run(());
    let cur_color = move || method_dot_color(&current());
    view! {
        <div
            class=move || format!("method-picker{}", if open.get() { " is-open" } else { "" })
            style=("--mc", cur_color)
        >
            <button data-bh="request.method"
                type="button"
                class="method-select"
                aria-haspopup="listbox"
                aria-expanded=move || open.get().to_string()
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <span class="method-dot"></span>
                <span class="method-name">{move || current()}</span>
                <svg class="method-chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>
            </button>
            <Show when=move || open.get()>
                <div class="method-menu-backdrop" on:click=move |_| open.set(false)></div>
                <div class="method-menu" role="listbox" aria-label="Метод запроса">
                    {METHODS
                        .iter()
                        .map(|m| {
                            let m = (*m).to_string();
                            let cls = {
                                let m = m.clone();
                                move || {
                                    if current() == m {
                                        "method-menu-item is-cur".to_string()
                                    } else {
                                        "method-menu-item".to_string()
                                    }
                                }
                            };
                            let dot_color = method_dot_color(&m);
                            let check = {
                                let m = m.clone();
                                move || if current() == m { "✓" } else { "" }
                            };
                            let pick = {
                                let m = m.clone();
                                move |_| {
                                    set_method.run(m.clone());
                                    open.set(false);
                                }
                            };
                            view! {
                                <button data-bh=format!("request.method_option.{}", m) type="button" class=cls style=("--mc", move || dot_color) on:click=pick>
                                    <span class="method-dot"></span>
                                    <span class="method-menu-name">{m.clone()}</span>
                                    <span class="method-check">{check}</span>
                                </button>
                            }
                        })
                        .collect::<Vec<_>>()}
                </div>
            </Show>
        </div>
    }
}
