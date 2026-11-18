//! mock_nav: правая колонка — настройки профиля. Стиль — строки настроек
//! (эталон): клик по строке открывает в центре соответствующий блок.
//! Правила живут в центре кнопками-спойлерами, поэтому в навигации осталось
//! только соединение.

use leptos::prelude::*;
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NavKind {
    Connect,
    Match,
    Variant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NavSel {
    pub kind: NavKind,
    pub rule: usize,
    pub variant: usize,
}

#[component]
pub fn MockNav(
    profile: RwSignal<Value>,
    current: RwSignal<Option<NavSel>>,
    pick: Callback<NavSel>,
) -> impl IntoView {
    view! {
        <div class="px-nav">
            <div class="px-h-row"><div class="px-h">"Настройки"</div></div>

            {move || {
                let p = profile.get();
                if p.get("name").and_then(|x| x.as_str()).is_none() {
                    return view! {
                        <div class="mnav-hint">"Выберите профиль, чтобы увидеть разделы"</div>
                    }
                    .into_any();
                }
                let sel = NavSel { kind: NavKind::Connect, rule: 0, variant: 0 };
                let cls = if current.get() == Some(sel) { "mnav-item active" } else { "mnav-item" };
                view! {
                    <div class=cls on:click=move |_| { pick.run(sel); }>
                        <span class="mnav-label">"Соединение"</span>
                    </div>
                }
                .into_any()
            }}

            <div class="mnav-hint">"Правила — в центральной колонке: нажми на правило, чтобы развернуть его и варианты."</div>
        </div>
    }
}
