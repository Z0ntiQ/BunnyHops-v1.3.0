```rust
use leptos::*;
use leptos::html::{Input, Textarea, Select};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{window, Request, RequestInit, Headers, Response};
use js_sys::{Object, Reflect, Promise, Array};
use serde_json::{Value, Map, json};
use std::rc::Rc;
use std::cell::RefCell;

use crate::ui::{input_value, input_checked, select_value};

async fn fetch_json_get(url: &str) -> Result<Value, String> {
    let win = window().unwrap();
    let resp_val = wasm_bindgen_futures::JsFuture::from(win.fetch_with_str(url)).await
        .map_err(|e| format!("{e:?}"))?;
    let resp: Response = resp_val.dyn_into()?;
    if !resp.ok() { return Err(format!("HTTP {}", resp.status())); }
    let txt = resp.text().unwrap();
    let val = wasm_bindgen_futures::JsFuture::from(txt).await.map_err(|e| format!("{e:?}"))?;
    let s = val.as_string().unwrap_or_default();
    serde_json::from_str(&s).map_err(|e| e.to_string())
}

async fn fetch_json_post(url: &str, body: &Value) -> Result<Value, String> {
    let win = window().unwrap();
    let init = RequestInit::new();
    init.set_method("POST");
    let headers = Headers::new().unwrap();
    headers.set("Content-Type", "application/json").unwrap();
    init.set_headers(&headers);
    init.set_body(&JsValue::from_str(&body.to_string()));
    let req = Request::new_with_str_and_init(url, &init).unwrap();
    let resp_val = wasm_bindgen_futures::JsFuture::from(win.fetch_with_request(&req)).await
        .map_err(|e| format!("{e:?}"))?;
    let resp: Response = resp_val.dyn_into()?;
    if !resp.ok() { return Err(format!("HTTP {}", resp.status())); }
    let txt = resp.text().unwrap();
    let val = wasm_bindgen_futures::JsFuture::from(txt).await.map_err(|e| format!("{e:?}"))?;
    let s = val.as_string().unwrap_or_default();
    serde_json::from_str(&s).map_err(|e| e.to_string())
}

#[component]
pub fn ProxyView() -> impl IntoView {
    let profiles = create_rw_signal(Vec::<Value>::new());
    let selected_profile = create_rw_signal(Option::<String>::None);
    let profile_data = create_rw_signal(Value::Null);
    let rules = create_rw_signal(Vec::<Value>::new());
    let selected_rule_idx = create_rw_signal(Option::<usize>::None);
    let variants = create_rw_signal(Vec::<Value>::new());
    let selected_variant_idx = create_rw_signal(Option::<usize>::None);
    let traffic = create_rw_signal(Vec::<Value>::new());
    let status_msg = create_rw_signal(String::new());
    let status_err = create_rw_signal(false);
    let preview_result = create_rw_signal(String::new());
    let polling_active = create_rw_signal(false);

    let load_profiles = move || {
        spawn_local(async move {
            match fetch_json_get("/api/mock/profiles").await {
                Ok(v) => {
                    if let Some(arr) = v.get("profiles").and_then(|x| x.as_array()) {
                        profiles.set(arr.clone());
                    }
                }
                Err(e) => {
                    status_msg.set(e);
                    status_err.set(true);
                }
            }
        });
    };

    let load_profile = move |name: String| {
        spawn_local(async move {
            match fetch_json_get(&format!("/api/mock/profile?name={}", name)).await {
                Ok(v) => {
                    let rules_arr = v.get("rules").and_then(|x| x.as_array()).cloned().unwrap_or_default();
                    rules.set(rules_arr.clone());
                    profile_data.set(v);
                    selected_profile.set(Some(name));
                    selected_rule_idx.set(None);
                    variants.set(Vec::new());
                    selected_variant_idx.set(None);
                    preview_result.set(String::new());
                }
                Err(e) => {
                    status_msg.set(e);
                    status_err.set(true);
                }
            }
        });
    };

    let save_profile = move || {
        let data = profile_data.get();
        if data.is_null() { return; }
        spawn_local(async move {
            match fetch_json_post("/api/mock/profile", &data).await {
                Ok(_) => {
                    status_msg.set("Профиль сохранён".into());
                    status_err.set(false);
                    load_profiles();
                }
                Err(e) => {
                    status_msg.set(e);
                    status_err.set(true);
                }
            }
        });
    };

    let delete_profile = move || {
        if let Some(name) = selected_profile.get() {
            if !window().unwrap().confirm_with_message(&format!("Удалить профиль '{}'?", name)).unwrap() {
                return;
            }
            spawn_local(async move {
                let body = json!({"name": name});
                match fetch_json_post("/api/mock/profile/delete", &body).await {
                    Ok(_) => {
                        status_msg.set("Профиль удалён".into());
                        status_err.set(false);
                        selected_profile.set(None);
                        profile_data.set(Value::Null);
                        rules.set(Vec::new());
                        load_profiles();
                    }
                    Err(e) => {
                        status_msg.set(e);
                        status_err.set(true);
                    }
                }
            });
        }
    };

    let start_profile = move || {
        if let Some(name) = selected_profile.get() {
            spawn_local(async move {
                let body = json!({"name": name});
                match fetch_json_post("/api/mock/start", &body).await {
                    Ok(v) => {
                        let port = v.get("port").and_then(|x| x.as_u64()).unwrap_or(0);
                        status_msg.set(format!("Запущен на порту {}", port));
                        status_err.set(false);
                        load_profiles();
                        polling_active.set(true);
                    }
                    Err(e) => {
                        status_msg.set(e);
                        status_err.set(true);
                    }
                }
            });
        }
    };

    let stop_profile = move || {
        if let Some(name) = selected_profile.get() {
            spawn_local(async move {
                let body = json!({"name": name});
                match fetch_json_post("/api/mock/stop", &body).await {
                    Ok(_) => {
                        status_msg.set("Остановлен".into());
                        status_err.set(false);
                        load_profiles();
                        polling_active.set(false);
                    }
                    Err(e) => {
                        status_msg.set(e);
                        status_err.set(true);
                    }
                }
            });
        }
    };

    let create_profile = move || {
        let win = window().unwrap();
        if let Some(name) = win.prompt_with_message("Имя профиля (a-z0-9_-):").unwrap() {
            let sanitized: String = name.chars()
                .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-')
                .collect();
            if sanitized.is_empty() { return; }
            let new_profile = json!({
                "name": sanitized,
                "listen_port": 3100,
                "upstream": Value::Null,
                "rules": []
            });
            spawn_local(async move {
                match fetch_json_post("/api/mock/profile", &new_profile).await {
                    Ok(_) => {
                        load_profiles();
                        load_profile(sanitized);
                    }
                    Err(e) => {
                        status_msg.set(e);
                        status_err.set(true);
                    }
                }
            });
        }
    };

    let select_rule = move |idx: usize| {
        selected_rule_idx.set(Some(idx));
        let rule = rules.get().get(idx).cloned().unwrap_or(Value::Null);
        let vars = rule.get("variants").and_then(|x| x.as_array()).cloned().unwrap_or_default();
        variants.set(vars);
        selected_variant_idx.set(None);
        preview_result.set(String::new());
    };

    let add_rule = move || {
        let mut r = rules.get();
        let new_rule = json!({
            "id": format!("r{}", r.len() + 1),
            "label": "Новое правило",
            "method": "*",
            "match": "prefix",
            "path": "/api/",
            "enabled": true,
            "active_variant": "v1",
            "cycle": false,
            "variants": [{
                "id": "v1",
                "label": "OK",
                "status": 200,
                "json": "{}",
                "randomize": false,
                "delay_ms": 0
            }]
        });
        r.push(new_rule);
        rules.set(r.clone());
        update_profile_rules(r);
    };

    let delete_rule = move |idx: usize| {
        let mut r = rules.get();
        if idx < r.len() {
            r.remove(idx);
            rules.set(r.clone());
            update_profile_rules(r);
            selected_rule_idx.set(None);
            variants.set(Vec::new());
        }
    };

    let update_profile_rules = move |new_rules: Vec<Value>| {
        let mut pd = profile_data.get();
        if let Some(obj) = pd.as_object_mut() {
            obj.insert("rules".into(), Value::Array(new_rules));
            profile_data.set(pd);
        }
    };

    let update_rule_field = move |idx: usize, key: &str, val: Value| {
        let mut r = rules.get();
        if let Some(rule) = r.get_mut(idx).and_then(|x| x.as_object_mut()) {
            rule.insert(key.into(), val);
            rules.set(r.clone());
            update_profile_rules(r);
        }
    };

    let add_variant = move || {
        if let Some(ridx) = selected_rule_idx.get() {
            let mut r = rules.get();
            if let Some(rule) = r.get_mut(ridx).and_then(|x| x.as_object_mut()) {
                let vars = rule.get_mut("variants").and_then(|x| x.as_array_mut()).unwrap();
                let vid = format!("v{}", vars.len() + 1);
                let new_var = json!({
                    "id": vid,
                    "label": format!("Вариант {}", vars.len() + 1),
                    "status": 200,
                    "json": "{}",
                    "randomize": false,
                    "delay_ms": 0
                });
                vars.push(new_var);
                rule.insert("active_variant".into(), Value::String(vid.clone()));
                rules.set(r.clone());
                update_profile_rules(r);
                let vars_clone = vars.clone();
                variants.set(vars_clone);
            }
        }
    };

    let delete_variant = move |vidx: usize| {
        if let Some(ridx) = selected_rule_idx.get() {
            let mut r = rules.get();
            if let Some(rule) = r.get_mut(ridx).and_then(|x| x.as_object_mut()) {
                let vars = rule.get_mut("variants").and_then(|x| x.as_array_mut()).unwrap();
                if vidx < vars.len() {
                    vars.remove(vidx);
                    if let Some(first) = vars.first() {
                        if let Some(id) = first.get("id").and_then(|x| x.as_str()) {
                            rule.insert("active_variant".into(), Value::String(id.into()));
                        }
                    }
                    rules.set(r.clone());
                    update_profile_rules(r);
                    let vars_clone = vars.clone();
                    variants.set(vars_clone);
                    selected_variant_idx.set(None);
                }
            }
        }
    };

    let select_variant = move |vidx: usize| {
        selected_variant_idx.set(Some(vidx));
        if let Some(ridx) = selected_rule_idx.get() {
            let mut r = rules.get();
            if let Some(rule) = r.get_mut(ridx).and_then(|x| x.as_object_mut()) {
                if let Some(v) = variants.get().get(vidx).and_then(|x| x.get("id")).and_then(|x| x.as_str()) {
                    rule.insert("active_variant".into(), Value::String(v.into()));
                    rules.set(r.clone());
                    update_profile_rules(r);
                }
            }
        }
    };

    let update_variant_field = move |vidx: usize, key: &str, val: Value| {
        if let Some(ridx) = selected_rule_idx.get() {
            let mut r = rules.get();
            if let Some(rule) = r.get_mut(ridx).and_then(|x| x.as_object_mut()) {
                let vars = rule.get_mut("variants").and_then(|x| x.as_array_mut()).unwrap();
                if let Some(var) = vars.get_mut(vidx).and_then(|x| x.as_object_mut()) {
                    var.insert(key.into(), val);
                    rules.set(r.clone());
                    update_profile_rules(r);
                    let vars_clone = vars.clone();
                    variants.set(vars_clone);
                }
            }
        }
    };

    let preview_variant = move || {
        if let Some(vidx) = selected_variant_idx.get() {
            if let Some(var) = variants.get().get(vidx) {
                let json_str = var.get("json").and_then(|x| x.as_str()).unwrap_or("{}");
                let randomize = var.get("randomize").and_then(|x| x.as_bool()).unwrap_or(false);
                spawn_local(async move {
                    let body = json!({"json": json_str, "randomize": randomize});
                    match fetch_json_post("/api/mock/preview", &body).await {
                        Ok(v) => {
                            if let Some(text) = v.get("text").and_then(|x| x.as_str()) {
                                preview_result.set(text.into());
                            }
                        }
                        Err(e) => {
                            preview_result.set(format!("Ошибка: {}", e));
                        }
                    }
                });
            }
        }
    };

    let poll_traffic = move || {
        if let Some(name) = selected_profile.get() {
            if polling_active.get() {
                spawn_local(async move {
                    match fetch_json_get(&format!("/api/mock/traffic?name={}", name)).await {
                        Ok(v) => {
                            if let Some(arr) = v.get("traffic").and_then(|x| x.as_array()) {
                                traffic.set(arr.clone());
                            }
                        }
                        Err(_) => {}
                    }
                });
            }
        }
        let cb = Closure::wrap(Box::new(move || {
            poll_traffic();
        }) as Box<dyn FnMut()>);
        if let Some(w) = window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                2000,
            );
        }
        cb.forget();
    };

    let on_profile_change = move |ev: web_sys::Event| {
        let val = input_value(ev);
        let mut pd = profile_data.get();
        if let Some(obj) = pd.as_object_mut() {
            obj.insert("name".into(), Value::String(val));
            profile_data.set(pd);
        }
    };

    let on_port_change = move |ev: web_sys::Event| {
        let val = input_value(ev);
        if let Ok(port) = val.parse::<u16>() {
            let mut pd = profile_data.get();
            if let Some(obj) = pd.as_object_mut() {
                obj.insert("listen_port".into(), Value::Number(port.into()));
                profile_data.set(pd);
            }
        }
    };

    let on_upstream_change = move |ev: web_sys::Event| {
        let val = input_value(ev);
        let mut pd = profile_data.get();
        if let Some(obj) = pd.as_object_mut() {
            if val.trim().is_empty() {
                obj.insert("upstream".into(), Value::Null);
            } else {
                obj.insert("upstream".into(), Value::String(val));
            }
            profile_data.set(pd);
        }
    };

    let on_rule_label_change = move |idx: usize, ev: web_sys::Event| {
        let val = input_value(ev);
        update_rule_field(idx, "label", Value::String(val));
    };

    let on_rule_method_change = move |idx: usize, ev: web_sys::Event| {
        let val = select_value(ev);
        update_rule_field(idx, "method", Value::String(val));
    };

    let on_rule_match_change = move |idx: usize, ev: web_sys::Event| {
        let val = select_value(ev);
        update_rule_field(idx, "match", Value::String(val));
    };

    let on_rule_path_change = move |idx: usize, ev: web_sys::Event| {
        let val = input_value(ev);
        update_rule_field(idx, "path", Value::String(val));
    };

    let on_rule_enabled_change = move |idx: usize, ev: web_sys::Event| {
        let val = input_checked(ev);
        update_rule_field(idx, "enabled", Value::Bool(val));
    };

    let on_rule_cycle_change = move |idx: usize, ev: web_sys::Event| {
        let val = input_checked(ev);
        update_rule_field(idx, "cycle", Value::Bool(val));
    };

    let on_variant_label_change = move |vidx: usize, ev: web_sys::Event| {
        let val = input_value(ev);
        update_variant_field(vidx, "label", Value::String(val));
    };

    let on_variant_status_change = move |vidx: usize, ev: web_sys::Event| {
        let val = input_value(ev);
        if let Ok(s) = val.parse::<u16>() {
            update_variant_field(vidx, "status", Value::Number(s.into()));
        }
    };

    let on_variant_delay_change = move |vidx: usize, ev: web_sys::Event| {
        let val = input_value(ev);
        if let Ok(d) = val.parse::<u64>() {
            update_variant_field(vidx, "delay_ms", Value::Number(d.into()));
        }
    };

    let on_variant_randomize_change = move |vidx: usize, ev: web_sys::Event| {
        let val = input_checked(ev);
        update_variant_field(vidx, "randomize", Value::Bool(val));
    };

    let on_variant_json_change = move |vidx: usize, ev: web_sys::Event| {
        let val = input_value(ev);
        update_variant_field(vidx, "json", Value::String(val));
    };

    Effect::new(move |_| {
        load_profiles();
    });

    Effect::new(move |_| {
        if polling_active.get() {
            poll_traffic();
        }
    });

    view! {
        <div class="px-layout">
            <div class="px-side">
                <div class="px-h">Профили</div>
                <button class="px-btn" on:click=move |_| create_profile()>
                    "+ Новый профиль"
                </button>
                <div class="px-rules">
                    <For
                        each=move || profiles.get()
                        key=|p| p.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string()
                        children=move |p| {
                            let name = p.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                            let running = p.get("running").and_then(|x| x.as_bool()).unwrap_or(false);
                            let rules_count = p.get("rules").and_then(|x| x.as_array()).map(|a| a.len()).unwrap_or(0);
                            let is_active = selected_profile.get().as_deref() == Some(&name);
                            view! {
                                <div class=format!("px-item {}", if is_active { "active" } else { "" })
                                    on:click=move |_| load_profile(name.clone())>
                                    <span class=format!("px-dot {}", if running { "on" } else { "" })}></span>
                                    <span>{name}</span>
                                    <span class="px-status">{format!("{} правил", rules_count)}</span>
                                </div>
                            }
                        }
                    />
                </div>
                <Show when=move || selected_profile.get().is_some()>
                    <button class="px-btn" on:click=move |_| delete_profile()>
                        "Удалить профиль"
                    </button>
                </Show>
            </div>

            <div class="px-main">
                <Show when=move || selected_profile.get().is_none() fallback=|| view! { <div class="px-empty">Выберите или создайте профиль слева</div> }>
                    <div class="px-field">
                        <label class="px-label">"Имя"</label>
                        <input class="px-input" type="text" readonly=true
                            prop:value=move || profile_data.get().get("name").and_then(|x| x.as_str()).unwrap_or("").to_string() />
                    </div>
                    <div class="px-grid2">
                        <div class="px-field">
                            <label class="px-label">"Порт"</label>
                            <input class="px-input" type="number" min="1" max="65535"
                                prop:value=move || profile_data.get().get("listen_port").and_then(|x| x.as_u64()).unwrap_or(0).to_string()
                                on:input=on_port_change />
                        </div>
                        <div class="px-field">
                            <label class="px-label">"Upstream (опционально)"</label>
                            <input class="px-input" type="text" placeholder="http://127.0.0.1:8080"
                                prop:value=move || profile_data.get().get("upstream").and_then(|x| x.as_str()).unwrap_or("").to_string()
                                on:input=on_upstream_change />
                        </div>
                    </div>
                    <div class="px-grid2">
                        <button class="px-btn primary" on:click=move |_| save_profile()>"Сохранить"</button>
                        <Show when=move || {
                            let pd = profile_data.get();
                            pd.get("listen_port").is_some()
                        } fallback=|| view! { <div></div> }>
                            <Show when=move || {
                                let profiles_list = profiles.get();
                                let name = selected_profile.get().unwrap_or_default();
                                profiles_list.iter().any(|p| p.get("name").and_then(|x| x.as_str()) == Some(&name) && p.get("running").and_then(|x| x.as_bool()).unwrap_or(false))
                            } fallback=move || view! {
                                <button class="px-btn primary" on:click=move |_| start_profile()>"Запустить"</button>
                            }>
                                <button class="px-btn" on:click=move |_| stop_profile()>"Остановить"</button>
                            </Show>
                        </Show>
                    </div>
                    <Show when=move || status_err.get() fallback=|| view! { <div class="px-status">{move || status_msg.get()}</div> }>
                        <div class="px-status err">{move || status_msg.get()}</div>
                    </Show>

                    <div class="px-h">Правила</div>
                    <div class="px-rules">
                        <For
                            each=move || rules.get()
                            key=|r| r.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string()
                            children=move |r| {
                                let idx = rules.get().iter().position(|x| x == &r).unwrap_or(0);
                                let is_sel = selected_rule_idx.get() == Some(idx);
                                let label = r.get("label").and_then(|x| x.as_str()).unwrap_or("");
                                let enabled = r.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
                                view! {
                                    <div class=format!("px-item {}", if is_sel { "active" } else { "" })
                                        on:click=move |_| select_rule(idx)>
                                        <input type="checkbox" class="px-check" checked=enabled on:click=move |e| e.stop_propagation() on:change=move |ev| on_rule_enabled_change(idx, ev) />
                                        <span>{label}</span>
                                    </div>
                                }
                            }
                        />
                    </div>
                    <button class="px-btn" on:click=move |_| add_rule()>"＋ Правило"</button>

                    <Show when=move || selected_rule_idx.get().is_some() fallback=|| view! { <div class="px-empty">Выберите правило для редактирования</div> }>
                        <div class="px-field">
                            <label class="px-label">"Label"</label>
                            <input class="px-input" type="text"
                                prop:value=move || {
                                    selected_rule_idx.get().and_then(|i| rules.get().get(i)).and_then(|r| r.get("label")).and_then(|x| x.as_str()).unwrap_or("").to_string()
                                }
                                on:input=move |ev| on_rule_label_change(selected_rule_idx.get().unwrap(), ev) />
                        </div>
                        <div class="px-grid2">
                            <div class="px-field">
                                <label class="px-label">"Method"</label>
                                <select class="px-input" prop:value=move || {
                                    selected_rule_idx.get().and_then(|i| rules.get().get(i)).and_then(|r| r.get("method")).and_then(|x| x.as_str()).unwrap_or("*").to_string()
                                } on:change=move |ev| on_rule_method_change(selected_rule_idx.get().unwrap(), ev)>
                                    <option value="*">"*"</option>
                                    <option value="GET">"GET"</option>
                                    <option value="POST">"POST"</option>
                                    <option value="PUT">"PUT"</option>
                                    <option value="PATCH">"PATCH"</option>
                                    <option value="DELETE">"DELETE"</option>
                                </select>
                            </div>
                            <div class="px-field">
                                <label class="px-label">"Match"</label>
                                <select class="px-input" prop:value=move || {
                                    selected_rule_idx.get().and_then(|i| rules.get().get(i)).and_then(|r| r.get("match")).and_then(|x| x.as_str()).unwrap_or("prefix").to_string()
                                } on:change=move |ev| on_rule_match_change(selected_rule_idx.get().unwrap(), ev)>
                                    <option value="exact">"exact"</option>
                                    <option value="prefix">"prefix"</option>
                                </select>
                            </div>
                        </div>
                        <div class="px-field">
                            <label class="px-label">"Path"</label>
                            <input class="px-input" type="text"
                                prop:value=move || {
                                    selected_rule_idx.get().and_then(|i| rules.get().get(i)).and_then(|r| r.get("path")).and_then(|x| x.as_str()).unwrap_or("").to_string()
                                }
                                on:input=move |ev| on_rule_path_change(selected_rule_idx.get().unwrap(), ev) />
                        </div>
                        <div class="px-field">
                            <label class="px-check-label">
                                <input type="checkbox" class="px-check"
                                    prop:checked=move || {
                                        selected_rule_idx.get().and_then(|i| rules.get().get(i)).and_then(|r| r.get("cycle")).and_then(|x| x.as_bool()).unwrap_or(false)
                                    }
                                    on:change=move |ev| on_rule_cycle_change(selected_rule_idx.get().unwrap(), ev) />
                                "Cycle variants"
                            </label>
                        </div>
                        <button class="px-btn" on:click=move |_| {
                            if let Some(idx) = selected_rule_idx.get() { delete_rule(idx); }
                        }>"Удалить правило"</button>

                        <div class="px-h">Варианты ответа</div>
                        <div class="px-rules">
                            <For
                                each=move || variants.get()
                                key=|v| v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string()
                                children=move |v| {
                                    let vidx = variants.get().iter().position(|x| x == &v).unwrap_or(0);
                                    let is_sel = selected_variant_idx.get() == Some(vidx);
                                    let label = v.get("label").and_then(|x| x.as_str()).unwrap_or("");
                                    view! {
                                        <div class=format!("px-chip {}", if is_sel { "active" } else { "" })
                                            on:click=move |_| select_variant(vidx)>
                                            {label}
                                        </div>
                                    }
                                }
                            />
                        </div>
                        <button class="px-btn" on:click=move |_| add_variant()>"＋ Вариант"</button>

                        <Show when=move || selected_variant_idx.get().is_some() fallback=|| view! { <div class="px-empty">Выберите вариант для редактирования</div> }>
                            <div class="px-field">
                                <label class="px-label">"Label"</label>
                                <input class="px-input" type="text"
                                    prop:value=move || {
                                        selected_variant_idx.get().and_then(|i| variants.get().get(i)).and_then(|v| v.get("label")).and_then(|x| x.as_str()).unwrap_or("").to_string()
                                    }
                                    on:input=move |ev| on_variant_label_change(selected_variant_idx.get().unwrap(), ev) />
                            </div>
                            <div class="px-grid2">
                                <div class="px-field">
                                    <label class="px-label">"Status"</label>
                                    <input class="px-input" type="number" min="100" max="599"
                                        prop:value=move || {
                                            selected_variant_idx.get().and_then(|i| variants.get().get(i)).and_then(|v| v.get("status")).and_then(|x| x.as_u64()).unwrap_or(200).to_string()
                                        }
                                        on:input=move |ev| on_variant_status_change(selected_variant_idx.get().unwrap(), ev) />
                                </div>
                                <div class="px-field">
                                    <label class="px-label">"Delay ms"</label>
                                    <input class="px-input" type="number" min="0"
                                        prop:value=move || {
                                            selected_variant_idx.get().and_then(|i| variants.get().get(i)).and_then(|v| v.get("delay_ms")).and_then(|x| x.as_u64()).unwrap_or(0).to_string()
                                        }
                                        on:input=move |ev| on_variant_delay_change(selected_variant_idx.get().unwrap(), ev) />
                                </div>
                            </div>
                            <div class="px-field">
                                <label class="px-check-label">
                                    <input type="checkbox" class="px-check"
                                        prop:checked=move || {
                                            selected_variant_idx.get().and_then(|i| variants.get().get(i)).and_then(|v| v.get("randomize")).and_then(|x| x.as_bool()).unwrap_or(false)
                                        }
                                        on:change=move |ev| on_variant_randomize_change(selected_variant_idx.get().unwrap(), ev) />
                                    "Рандомизировать {{...}}"
                                </label>
                            </div>
                            <div class="px-field">
                                <label class="px-label">"JSON Body"</label>
                                <textarea class="px-json" rows="14"
                                    prop:value=move || {
                                        selected_variant_idx.get().and_then(|i| variants.get().get(i)).and_then(|v| v.get("json")).and_then(|x| x.as_str()).unwrap_or("{}").to_string()
                                    }
                                    on:input=move |ev| on_variant_json_change(selected_variant_idx.get().unwrap(), ev) />
                            </div>
                            <div class="px-grid2">
                                <button class="px-btn primary" on:click=move |_| preview_variant()>"Предпросмотр"</button>
                                <button class="px-btn" on:click=move |_| {
                                    if let Some(vidx) = selected_variant_idx.get() { delete_variant(vidx); }
                                }>"Удалить вариант"</button>
                            </div>
                            <Show when=move || !preview_result.get().is_empty() fallback=|| view! { <div></div> }>
                                <div class="px-field">
                                    <label class="px-label">"Результат предпросмотра"</label>
                                    <textarea class="px-json" rows="6" readonly=true prop:value=move || preview_result.get() />
                                </div>
                            </Show>
                        </Show>
                    </Show>
                </Show>
            </div>

            <div class="px-monitor">
                <div class="px-h">Трафик</div>
                <Show when=move || polling_active.get() fallback=|| view! { <div class="px-empty">Запустите профиль для мониторинга</div> }>
                    <table class="px-table">
                        <thead>
                            <tr>
                                <th>Method</th>
                                <th>Path</th>
                                <th>Status</th>
                                <th>ms</th>
                            </tr>
                        </thead>
                        <tbody>
                            <For
                                each=move || traffic.get()
                                key=|t| t.get("ts").and_then(|x| x.as_f64()).unwrap_or(0.0).to_string()
                                children=move |t| {
                                    let method = t.get("method").and_then(|x| x.as_str()).unwrap_or("");
                                    let path = t.get("path").and_then(|x| x.as_str()).unwrap_or("");
                                    let status = t.get("status").and_then(|x| x.as_u64()).unwrap_or(0);
                                    let ms = t.get("ms").and_then(|x| x.as_u64()).unwrap_or(0);
                                    let status_class = if status < 400 { "ok" } else { "err" };
                                    view! {
                                        <tr class="px-row">
                                            <td>{method}</td>
                                            <td>{path}</td>
                                            <td class=status_class>{status}</td>
                                            <td>{ms}</td>
                                        </tr>
                                    }
                                }
                            />
                        </tbody>
                    </table>
                </Show>
            </div>
        </div>
    }
}
```