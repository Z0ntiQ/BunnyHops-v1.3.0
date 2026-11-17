# Твоя задача (d3_proxy)

Напиши файл `frontend/src/proxy.rs` полностью, по «Контракту C» из SPEC (leptos 0.7 csr).
- Все паттерны Leptos/wasm (сигналы, `create_action`, `spawn_local`, fetch с `RequestInit`/`Headers`, `set_timeout`, `input_value`/`input_checked`/`select_value`) бери дословно из приложенных фрагментов `app_fetch.txt`, `app_timer.txt`, `ui_helpers.txt`, `stream_request.txt`. Синтаксис leptos 0.7: `create_rw_signal`, `create_action`, `view! { ... }`, `For`/`Show` как в app_fetch.txt.
- Компонент `ProxyView` — верхний уровень вкладки; `use crate::ui::{input_value, input_checked, select_value};` разрешён (сигнатуры в фрагменте).
- Данные профиля держи как `serde_json::Value` в rw-сигналах; правки полей — через `pointer`/`as_object_mut`.
- UI — три колонки Postman-style по Контракту C; только классы `px-*` из Контракта D.
- Ниже приложены фрагменты эталонного кода.

=== app_fetch.txt ===
fn load_vars() -> Vec<KvRow> {
    ls_get(LS_VARS)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn load_saved() -> Vec<SavedReq> {
    ls_get(LS_SAVED)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn load_groups() -> Vec<SavedGroup> {
    ls_get(LS_GROUPS)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

async fn fetch_api_calls() -> Vec<ApiCall> {
    let win = match web_sys::window() {
        Some(w) => w,
        None => return Vec::new(),
    };
    let promise = win.fetch_with_str("/api/automation-rules");
    let Ok(val) = wasm_bindgen_futures::JsFuture::from(promise).await else {
        return Vec::new();
    };
    let Ok(resp) = val.dyn_into::<web_sys::Response>() else {
        return Vec::new();
    };
    let Ok(text_promise) = resp.text() else {
        return Vec::new();
    };
    let Ok(txt) = wasm_bindgen_futures::JsFuture::from(text_promise).await else {
        return Vec::new();
    };
    serde_json::from_str(&txt.as_string().unwrap_or_default()).unwrap_or_default()
}

fn ts_short() -> String {
    let d = js_sys::Date::new_0();
    format!("{:02}:{:02}:{:02}", d.get_hours(), d.get_minutes(), d.get_seconds())
}

fn fmt_ts(ts: u64) -> String {
    let d = js_sys::Date::new(&JsValue::from_f64(ts as f64 * 1000.0));
    js_sys::Date::to_time_string(&d).into()
}

fn load_scale_idx() -> usize {
    ls_get(LS_SCALE_IDX)


=== app_timer.txt ===
    );

    let _ = Effect::watch(
        move || saved.get().len(),
        move |_, _, _: Option<()>| {
            let cf = cloud_full;
            let cb = Closure::wrap(Box::new(move || {
                cf.set(cloud_overflow());
            }) as Box<dyn FnMut()>);
            if let Some(w) = web_sys::window() {
                let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(
                    cb.as_ref().unchecked_ref(),
                    80,
                );
            }
            cb.forget();
        },
        false,
    );
    {
        let cf = cloud_full;
        let on_resize = Closure::wrap(Box::new(move || {
            cf.set(cloud_overflow());
        }) as Box<dyn FnMut()>);
        if let Some(w) = web_sys::window() {
            let _ = w.add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref());
        }
        on_resize.forget();
    }

    /* ---------- copy ---------- */
    let mark_copied = |copied: RwSignal<bool>| Callback::new(move |_: ()| {
        copied.set(true);
        let cb = Closure::wrap(Box::new(move || copied.set(false)) as Box<dyn FnMut()>);
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                1200,
            );
        }
        cb.forget();


=== ui_helpers.txt ===
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


=== stream_request.txt ===

    let mut h = Vec::new();
    for (k, v) in headers_obj {
        if !k.trim().is_empty() {
            h.push((k.clone(), v.clone()));
        }
    }
    let proxy_url = format!("/proxy?url={}", crate::logic::js_encode_uri_component(target));
    let init = RequestInit::new();
    init.set_method(method);
    let hobj = js_sys::Object::new();
    let headers_json = serde_json::to_string(
        &h.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone()))).collect::<serde_json::Map<_, _>>(),
    )
    .unwrap();
    // порядок ключей в JSON сохраняем вручную: serde_json::Map сортирует только с feature "preserve_order";
    // для контракта прокси порядок не значим.
    js_sys::Reflect::set(&hobj, &JsValue::from_str("x-proxy-headers"), &JsValue::from_str(&headers_json))
        .unwrap();
    // запросы из интерфейса не попадают в лог внешнего API
    let _ = js_sys::Reflect::set(&hobj, &JsValue::from_str("x-hop-ui"), &JsValue::from_str("1"));
    init.set_headers(&JsValue::from(hobj));
    if !body_text.trim().is_empty() && !matches!(method, "GET" | "HEAD") {
        init.set_body(&JsValue::from_str(body_text));
    }
    init.set_signal(Some(signal));

    let req = Request::new_with_str_and_init(&proxy_url, &init)?;
    let resp_val = JsFuture::from(win.fetch_with_request(&req)).await?;
    let resp: Response = resp_val.dyn_into()?;

    let proxy_err = resp.headers().get("x-proxy-error").unwrap_or(None);
    if let Some(_pe) = proxy_err {
        let text = JsFuture::from(resp.text()?).await?.as_string().unwrap_or_default();
        return Ok(StreamOutcome {
            network_error: true,
            status: resp.status() as u16,
            time_ms: 0.0,
            bytes: 0,
            text: if text.is_empty() { "Не удалось подключиться".to_string() } else { text },
            image_blob_url: None,
        });
    }

    let mut headers: Vec<(String, String)> = Vec::new();
    // web_sys HeadersIterator::next() в этой версии возвращает Err с вложенным
    // значением — берём записи через Array::from по Symbol.iterator.
    let entries = js_sys::Array::from(&resp.headers().entries());
    for item in entries.iter() {
        if let Ok(arr) = item.dyn_into::<js_sys::Array>() {
            let k = arr.get(0).as_string().unwrap_or_default();
            let v = arr.get(1).as_string().unwrap_or_default();
            headers.push((k, v));
        }
    }
    let content_type = headers
