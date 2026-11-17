//! UI-компоненты: KvEditor, CopyButton, чипы статуса — 1:1 с App.jsx.

use leptos::prelude::*;
use leptos::callback::Callable;
use wasm_bindgen::prelude::Closure;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use crate::logic::{format_size, KvRow};

pub const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
pub const BODYLESS: [&str; 2] = ["GET", "HEAD"];

pub fn is_bodyless(m: &str) -> bool {
    BODYLESS.contains(&m)
}

/* ---------- generic key/value editor (KvEditor) ---------- */

fn parse_kv_text(text: &str) -> Vec<KvRow> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            let (k, v) = match l.find(':') {
                Some(p) => (l[..p].trim(), l[p + 1..].trim()),
                None => (l.trim(), ""),
            };
            let v = if v.len() >= 2
                && ((v.starts_with('"') && v.ends_with('"'))
                    || (v.starts_with('\'') && v.ends_with('\'')))
            {
                &v[1..v.len() - 1]
            } else {
                v
            };
            let key: String = k
                .chars()
                .filter(|c| {
                    c.is_ascii_alphanumeric()
                        || matches!(c, '!' | '#' | '$' | '%' | '&' | '\'' | '*' | '+' | '-' | '.' | '^' | '_' | '`' | '|' | '~')
                })
                .collect();
            if key.is_empty() {
                return None;
            }
            let value: String = v.chars().filter(|c| matches!(c, '\u{20}'..='\u{7e}')).collect();
            Some(KvRow { key, value: value.trim_end().to_string(), on: true })
        })
        .collect()
}

fn kv_to_text(rows: &[KvRow]) -> String {
    // Все строки списка видны в Bulk edit, и пустые тоже (запрос пользователя
    // 17.11.2026: создал строку «Add new» — она должна быть в окне).
    // Пустая строка = пустая строка текста; при наборе лишние пустые строки
    // отфильтрует parse_kv_text.
    rows.iter()
        .map(|r| {
            if r.key.trim().is_empty() {
                String::new()
            } else {
                format!("{}: {}", r.key, r.value)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Максимум высоты панели журнала: от низа окна (отступ 30px) до нижней
/// границы таб-бара (.tabs). Ручка высоты — actlog-resize в панели журнала.
/// Start dragging the log height from the panel's top-edge handle.
/// Call AFTER setting the starting height.
pub fn start_log_drag(ev: &web_sys::PointerEvent, lh: RwSignal<i32>) {
    ev.prevent_default();
    if let Some(t) = ev.target() {
        if let Some(he) = t.dyn_ref::<web_sys::HtmlElement>() {
            let _ = he.set_pointer_capture(ev.pointer_id());
        }
    }
    let drag_start = create_rw_signal(Some((ev.client_y(), lh.get_untracked())));
    let mv: std::rc::Rc<std::cell::RefCell<Option<Closure<dyn FnMut(web_sys::PointerEvent)>>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let up: std::rc::Rc<std::cell::RefCell<Option<Closure<dyn FnMut(web_sys::PointerEvent)>>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let mv_c = Closure::wrap(Box::new(move |ev: web_sys::PointerEvent| {
        if let Some((y0, h0)) = drag_start.get() {
            let max = log_h_max();
            lh.set((h0 + (y0 - ev.client_y())).clamp(60, max));
        }
    }) as Box<dyn FnMut(_)>);
    let (mv2, up2) = (mv.clone(), up.clone());
    let up_c = Closure::wrap(Box::new(move |_: web_sys::PointerEvent| {
        drag_start.set(None);
        crate::storage::ls_set("hoplite.logH", &lh.get_untracked().to_string());
        if let Some(w) = web_sys::window() {
            if let Some(m) = mv2.borrow().as_ref() {
                let _ = w.remove_event_listener_with_callback(
                    "pointermove", m.as_ref().unchecked_ref());
            }
            if let Some(u) = up2.borrow().as_ref() {
                let _ = w.remove_event_listener_with_callback(
                    "pointerup", u.as_ref().unchecked_ref());
            }
        }
        // снять с себя владение, но НЕ дропать: up-замыкание
        // исполняется прямо сейчас, drop = use-after-free.
        std::mem::forget(mv2.borrow_mut().take());
        std::mem::forget(up2.borrow_mut().take());
    }) as Box<dyn FnMut(_)>);
    if let Some(w) = web_sys::window() {
        let _ = w.add_event_listener_with_callback(
            "pointermove", mv_c.as_ref().unchecked_ref());
        let _ = w.add_event_listener_with_callback(
            "pointerup", up_c.as_ref().unchecked_ref());
    }
    *mv.borrow_mut() = Some(mv_c);
    *up.borrow_mut() = Some(up_c);
}

fn log_h_max() -> i32 {
    let Some(win) = web_sys::window() else { return 600 };
    let inner = win
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(900.0) as i32;
    // Верхняя граница журнала — низ поля URL: панель тянется вплоть до инпута.
    // 17.11.2026: измеряли несуществующий .tabs -> потолок 180px (дефект).
    let bottom = win
        .document()
        .and_then(|d| d.query_selector(".url-input").ok().flatten())
        .map(|e| e.get_bounding_client_rect().bottom() as i32)
        .unwrap_or(inner - 210);
    (inner - 30 - bottom).max(60)
}

/// Тайп-машина плейсхолдеров: ключ, затем значение, с той же скоростью, что живой
/// плейсхолдер поля URL (90ms/символ). Ровно один проход, без цикла (17.11.2026).
pub fn type_placeholders_once(
    key: &'static str,
    value: &'static str,
    out_key: RwSignal<String>,
    out_val: RwSignal<String>,
) {
    wasm_bindgen_futures::spawn_local(async move {
        for i in 0..=key.chars().count() {
            out_key.set(key.chars().take(i).collect());
            sleep_ms(90).await;
        }
        for i in 0..=value.chars().count() {
            out_val.set(value.chars().take(i).collect());
            sleep_ms(90).await;
        }
    });
}

fn sleep_ms(ms: i32) -> impl std::future::Future<Output = ()> {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let _ = web_sys::window()
            .expect("no window")
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
    });
    async move {
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
}

/// get_rows/set_rows — произвольное хранилище (сигнал или строка внутри шага),
/// как rows/onChange в React-версии.
/// example_*/empty_hint — строка-пример, пока список пуст (URL Params);
/// без них панель пустая, как раньше.
#[component]
pub fn KvEditor(
    get_rows: Callback<(), Vec<KvRow>>,
    set_rows: Callback<Vec<KvRow>>,
    key_placeholder: &'static str,
    value_placeholder: &'static str,
    #[prop(default = "")] example_key: &'static str,
    #[prop(default = "")] example_value: &'static str,
    #[prop(default = "")] empty_hint: &'static str,
    /* «фактический результат» примера: метка + текст (URL, заголовок, подстановка).
       Текст может читать сигналы — обновляется реактивно (17.11.2026). */
    #[prop(default = "")] preview_label: &'static str,
    #[prop(optional)] preview: Option<Callback<(), String>>,
) -> impl IntoView {
    let bulk_open = create_rw_signal(false);
    let bulk_text = create_rw_signal(String::new());
    let bulk_ta = create_node_ref::<leptos::html::Textarea>();

    // Открытие: наполнить поле из строк, показать панель, увести фокус в textarea.
    let open_bulk = {
        let get_rows = get_rows;
        move |_: leptos::ev::MouseEvent| {
            bulk_text.set(kv_to_text(&get_rows.run(())));
            bulk_open.set(true);
            if let Some(w) = web_sys::window() {
                let cb = Closure::once_into_js(move || {
                    if let Some(el) = bulk_ta.get() {
                        let _ = el.focus();
                    }
                });
                let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(
                    cb.as_ref().unchecked_ref(),
                    60,
                );
            }
        }
    };
    let close_bulk = move |_: leptos::ev::MouseEvent| bulk_open.set(false);

    /* клик по строке-примеру добавляет настоящую строку — тот же путь, что «+ Add new» */
    let add_row = move |_: leptos::ev::MouseEvent| {
        let mut rs = get_rows.run(());
        rs.push(KvRow { key: String::new(), value: String::new(), on: true });
        set_rows.run(rs);
    };

    view! {
        <div class="kv-list" class:bulk-open=move || bulk_open.get()>
            <div class="kv-toolbar">
                <div class="morph" class:morph-open=move || bulk_open.get()>
                    <button
                        class="morph-pill"
                        aria-haspopup="dialog"
                        aria-expanded=move || bulk_open.get().to_string()
                        on:click=open_bulk
                    >
                        <svg aria-hidden="true" width="14" height="14" fill="none" viewBox="0 0 16 16">
                            <path
                                d="M2.75 4.25a1.5 1.5 0 0 1 1.5-1.5h7.5a1.5 1.5 0 0 1 1.5 1.5v5a1.5 1.5 0 0 1-1.5 1.5H7l-2.75 2.5v-2.5a1.5 1.5 0 0 1-1.5-1.5v-5Z"
                                stroke="currentColor" stroke-linejoin="round" stroke-width="1.4"
                            />
                        </svg>
                        "Bulk edit"
                    </button>
                    <Show when={move || bulk_open.get()}>
                        <div class="morph-panel" role="dialog" aria-label="Bulk edit">
                            <div class="morph-head">
                                <svg aria-hidden="true" width="14" height="14" fill="none" viewBox="0 0 16 16">
                                    <path
                                        d="M2.75 4.25a1.5 1.5 0 0 1 1.5-1.5h7.5a1.5 1.5 0 0 1 1.5 1.5v5a1.5 1.5 0 0 1-1.5 1.5H7l-2.75 2.5v-2.5a1.5 1.5 0 0 1-1.5-1.5v-5Z"
                                        stroke="currentColor" stroke-linejoin="round" stroke-width="1.4"
                                    />
                                </svg>
                                "Bulk edit"
                            </div>
                            <textarea
                                class="kv-bulk-text"
                                node_ref=bulk_ta
                                prop:value=bulk_text
                                placeholder="Header: value — по одной строке"
                                spellcheck="false"
                                on:input=move |ev| {
                                    let t = input_value(ev);
                                    bulk_text.set(t.clone());
                                    set_rows.run(parse_kv_text(&t));
                                }
                                on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                                    if ev.key() == "Escape" {
                                        ev.stop_propagation();
                                        bulk_open.set(false);
                                    } else if ev.key() == "Enter"
                                        && (ev.meta_key() || ev.ctrl_key())
                                    {
                                        ev.prevent_default();
                                        bulk_open.set(false);
                                    }
                                }
                            />
                            <div class="morph-foot">
                                <span class="morph-hint">{"применяется сразу · ⌘↵ или Esc — закрыть"}</span>
                                <button class="morph-done" on:click=close_bulk>"Готово"</button>
                            </div>
                        </div>
                    </Show>
                </div>
                <div class="kv-toolbar-right">
                    <button class="kv-add" on:click=add_row>
                        "+ Add new"
                    </button>
                </div>
            </div>
            <Show when={move || get_rows.run(()).is_empty() && !example_key.is_empty()}>
                <KvEmptyState
                    get_rows=get_rows
                    set_rows=set_rows
                    example_key=example_key
                    example_value=example_value
                    empty_hint=empty_hint
                    preview_label=preview_label
                    preview=preview
                />
            </Show>
            <For
                each={move || get_rows.run(()).into_iter().enumerate().collect::<Vec<_>>()}
                key={|(i, _)| *i}
                children={move |(i, row)| {
                    let key_ref = create_node_ref::<leptos::html::Input>();
                    let val_ref = create_node_ref::<leptos::html::Input>();
                    let chk_ref = create_node_ref::<leptos::html::Input>();
                    /* плейсхолдеры строки печатаются один раз при её появлении,
                       как в поле URL (17.11.2026); у строк со значением плейсхолдер скрыт */
                    let ph_key = create_rw_signal(String::new());
                    let ph_val = create_rw_signal(String::new());
                    type_placeholders_once(key_placeholder, value_placeholder, ph_key, ph_val);
                    create_effect(move |_| {
                        let rs = get_rows.run(());
                        if let Some(r) = rs.get(i) {
                            if let Some(el) = chk_ref.get() {
                                if el.checked() != r.on { el.set_checked(r.on); }
                            }
                            if let Some(el) = key_ref.get() {
                                if el.value() != r.key { el.set_value(&r.key); }
                            }
                            if let Some(el) = val_ref.get() {
                                if el.value() != r.value { el.set_value(&r.value); }
                            }
                        }
                    });
                    view! {
                        <div class="kv-row">
                            <input
                                node_ref=chk_ref
                                type="checkbox"
                                class="kv-check"
                                prop:checked=row.on
                                on:change=move |ev| {
                                    let mut rs = get_rows.run(());
                                    if i < rs.len() {
                                        rs[i].on = input_checked(ev);
                                    }
                                    set_rows.run(rs);
                                }
                                title="Включить/выключить"
                            />
                            <input
                                node_ref=key_ref
                                class="kv-input"
                                prop:value=row.key
                                placeholder=move || ph_key.get()
                                spellcheck="false"
                                on:input=move |ev| {
                                    let v = input_value(ev);
                                    let mut rs = get_rows.run(());
                                    if i < rs.len() {
                                        rs[i].key = v;
                                    }
                                    set_rows.run(rs);
                                }
                            />
                            <input
                                node_ref=val_ref
                                class="kv-input"
                                prop:value=row.value
                                placeholder=move || ph_val.get()
                                spellcheck="false"
                                on:input=move |ev| {
                                    let v = input_value(ev);
                                    let mut rs = get_rows.run(());
                                    if i < rs.len() {
                                        rs[i].value = v;
                                    }
                                    set_rows.run(rs);
                                }
                            />
                            <button
                                class="kv-remove"
                                title="Удалить"
                                on:click=move |_| {
                                    let mut rs = get_rows.run(());
                                    if i < rs.len() {
                                        rs.remove(i);
                                    }
                                    set_rows.run(rs);
                                }
                            >
                                "×"
                            </button>
                        </div>
                    }
                }}
            />
        </div>
    }
}

/// Разбивает текст превью на сегменты: {{токены}} и обычный текст.
fn split_tokens(s: &str) -> Vec<(bool, String)> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find("{{") {
        if let Some(j) = rest[i..].find("}}") {
            if i > 0 {
                out.push((false, rest[..i].to_string()));
            }
            out.push((true, rest[i..i + j + 2].to_string()));
            rest = &rest[i + j + 2..];
            continue;
        }
        break;
    }
    if !rest.is_empty() {
        out.push((false, rest.to_string()));
    }
    out
}

/// Плашка-пример пустого KvEditor: создаётся вместе с Show — плейсхолдеры печатаются
/// ровно один раз на появление, со скоростью плейсхолдера поля URL (17.11.2026).
#[component]
fn KvEmptyState(
    get_rows: Callback<(), Vec<KvRow>>,
    set_rows: Callback<Vec<KvRow>>,
    example_key: &'static str,
    example_value: &'static str,
    #[prop(default = "")] empty_hint: &'static str,
    #[prop(default = "")] preview_label: &'static str,
    /* пробрасывается как есть; обязателен — у внутреннего компонента всегда Some/None явно */
    preview: Option<Callback<(), String>>,
) -> impl IntoView {
    let add_row = move |_: leptos::ev::MouseEvent| {
        let mut rs = get_rows.run(());
        rs.push(KvRow { key: String::new(), value: String::new(), on: true });
        set_rows.run(rs);
    };
    let ph_key = create_rw_signal(String::new());
    let ph_val = create_rw_signal(String::new());
    type_placeholders_once(example_key, example_value, ph_key, ph_val);
    view! {
        <button class="kv-empty" on:click=add_row title="Добавить строку">
            <span class="kv-empty-row">
                <span class="kv-empty-check"/>
                <span class="kv-empty-cell">{move || ph_key.get()}</span>
                <span class="kv-empty-cell">{move || ph_val.get()}</span>
            </span>
            <Show when={move || preview.is_some()}>
                <span class="kv-empty-url">
                    <span class="kv-empty-url-label">{preview_label}</span>
                    <span class="kv-empty-url-val">{move || {
                        let s = preview.map(|cb| cb.run(())).unwrap_or_default();
                        split_tokens(&s)
                            .into_iter()
                            .map(|(is_token, txt)| {
                                let cls = if is_token { "kv-token" } else { "" };
                                view! { <span class=cls>{txt}</span> }
                            })
                            .collect::<Vec<_>>()
                    }}</span>
                </span>
            </Show>
            <span class="kv-empty-hint">{empty_hint}</span>
        </button>
    }
}

/* ---------- копирование в буфер с фолбэком (copyText) ---------- */

async fn copy_text(text: &str) -> bool {
    let win = web_sys::window().unwrap();
    let nav = win.navigator();
    let clip = nav.clipboard();
    if JsFuture::from(clip.write_text(text)).await.is_ok() {
        return true;
    }
    // фолбэк: скрытый textarea + document.execCommand('copy')
    let doc = win.document().unwrap();
    let ta: web_sys::HtmlTextAreaElement = doc
        .create_element("textarea")
        .unwrap()
        .dyn_into()
        .unwrap();
    ta.set_value(text);
    let style = ta.dyn_ref::<web_sys::HtmlElement>().unwrap().style();
    let _ = style.set_property("position", "fixed");
    let _ = style.set_property("opacity", "0");
    doc.body().unwrap().append_child(&ta).unwrap();
    ta.select();
    let ok = doc
        .clone()
        .dyn_into::<web_sys::HtmlDocument>()
        .ok()
        .and_then(|d| d.exec_command("copy").ok())
        .unwrap_or(false);
    let _ = doc.body().unwrap().remove_child(&ta);
    ok
}

#[component]
pub fn CopyButton(
    get_text: Callback<(), String>,
    ok: ReadSignal<bool>,
    on_copied: Callback<()>,
    title: &'static str,
) -> impl IntoView {
    view! {
        <button
            class=move || if ok.get() { "copy-btn ok" } else { "copy-btn" }
            title=move || if ok.get() { "Скопировано!".to_string() } else { title.to_string() }
            on:click=move |_| {
                let text = get_text.run(());
                let ok_c = on_copied;
                wasm_bindgen_futures::spawn_local(async move {
                    if copy_text(&text).await {
                        ok_c.run(());
                    }
                });
            }
        >
            {move || if ok.get() { "✓".to_string() } else { "⧉".to_string() }}
        </button>
    }
}

/* ---------- resultChip / failChip ---------- */

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StepResult {
    pub status: StepStatusVal,
    #[serde(default)]
    pub time_ms: f64,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub capture_note: String,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum StepStatusVal {
    Code(u16),
    Text(String),
}

fn status_class(code: u16) -> &'static str {
    if code < 400 {
        "s-2xx"
    } else if code < 500 {
        "s-4xx"
    } else {
        "s-5xx"
    }
}

pub fn result_chip(r: StepResult) -> AnyView {
    let cls = match &r.status {
        StepStatusVal::Code(c) => status_class(*c),
        StepStatusVal::Text(_) => "s-5xx",
    };
    let status_text = match &r.status {
        StepStatusVal::Code(c) => c.to_string(),
        StepStatusVal::Text(t) => t.clone(),
    };
    view! {
        <>
            <span class=format!("status-chip mini {cls}")>{status_text}</span>
            <span class="mini-meta">{format!("{:.0} ms", r.time_ms)}</span>
            <span class="mini-meta">{format_size(Some(r.bytes))}</span>
            <Show when={let n = r.capture_note.clone(); move || !n.is_empty()}>
                <span class="mini-meta capture-ok">{r.capture_note.clone()}</span>
            </Show>
        </>
    }
    .into_any()
}

pub fn fail_chip(r: StepResult) -> AnyView {
    let status_text = match &r.status {
        StepStatusVal::Code(c) => c.to_string(),
        StepStatusVal::Text(t) => t.clone(),
    };
    view! {
        <>
            <span class="status-chip mini s-5xx">{status_text}</span>
            <Show when={let ms = r.time_ms; move || ms != 0.0}>
                <span class="mini-meta">{format!("{:.0} ms", r.time_ms)}</span>
            </Show>
            <Show when={let n = r.capture_note.clone(); move || !n.is_empty()}>
                <span class="mini-meta capture-warn">{r.capture_note.clone()}</span>
            </Show>
        </>
    }
    .into_any()
}

/* ---------- DOM helpers ---------- */

pub fn input_value(ev: web_sys::Event) -> String {
    ev.target()
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap()
        .dyn_ref::<web_sys::HtmlInputElement>()
        .map(|i| i.value())
        .or_else(|| {
            ev.target()
                .unwrap()
                .dyn_ref::<web_sys::HtmlTextAreaElement>()
                .map(|t| t.value())
        })
        .unwrap_or_default()
}

pub fn input_checked(ev: web_sys::Event) -> bool {
    ev.target()
        .unwrap()
        .dyn_ref::<web_sys::HtmlInputElement>()
        .map(|i| i.checked())
        .unwrap_or(false)
}

pub fn select_value(ev: web_sys::Event) -> String {
    ev.target()
        .unwrap()
        .dyn_into::<web_sys::HtmlSelectElement>()
        .map(|s| s.value())
        .unwrap_or_default()
}

pub fn new_id() -> String {
    let uuid = web_sys::window()
        .and_then(|w| w.crypto().ok())
        .map(|c| c.random_uuid())
        .unwrap_or_default();
    if !uuid.is_empty() {
        return uuid;
    }
    format!("{}-{}", js_now(), js_random())
}

pub fn js_now() -> f64 {
    web_sys::window().map(|w| w.performance().unwrap().now()).unwrap_or(0.0)
}

pub fn js_random() -> f64 {
    js_sys::Math::random()
}
