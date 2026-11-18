use leptos::prelude::*;
use serde_json::{json, Value};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::Response;

use crate::http_status::{status_label, STATUS_CLASSES};
use crate::mock_nav::{MockNav, NavKind, NavSel};

/// Строка JSON с заменой всех токенов {{...}} на 1: токен вне кавычек —
/// заглушка-число, токен внутри строки остаётся частью строки.
fn strip_tokens(t: &str) -> String {
    let mut out = String::new();
    let mut rest = t;
    while let Some(a) = rest.find("{{") {
        let b = match rest[a + 2..].find("}}") {
            Some(b) => a + 2 + b,
            None => break,
        };
        out.push_str(&rest[..a]);
        out.push('1');
        rest = &rest[b + 2..];
    }
    out.push_str(rest);
    out
}

/// Вариант битый: status вне HTTP-диапазона или JSON не парсится
/// (сначала как есть, затем с токенами-заглушками).
fn var_bad(v: &Value) -> bool {
    let s = v.get("status").and_then(|x| x.as_u64()).unwrap_or(200);
    if !(100..=599).contains(&s) {
        return true;
    }
    match v.get("json").and_then(|x| x.as_str()) {
        Some(t) if !t.trim().is_empty() && t.trim() != "{}" => {
            serde_json::from_str::<Value>(t).is_err()
                && serde_json::from_str::<Value>(&strip_tokens(t)).is_err()
        }
        _ => false,
    }
}
use crate::ui::{input_checked, input_value, select_value};

/// Мини-плюс для кнопок добавления: SVG в стиле системных иконок (stroke currentColor).
#[component]
fn IconPlus() -> impl IntoView {
    view! {
        <svg aria-hidden="true" width="11" height="11" viewBox="0 0 16 16" fill="none">
            <path
                d="M8 3.25v9.5M3.25 8h9.5"
                stroke="currentColor" stroke-width="1.6" stroke-linecap="round"
            />
        </svg>
    }
}

/// Стрелка направления порядка (вверх): переключатель asc/desc,
/// запрос пользователя 17.11.2026.
fn IconArrowUp() -> impl IntoView {
    view! {
        <svg aria-hidden="true" width="11" height="11" viewBox="0 0 16 16" fill="none">
            <path
                d="M8 13V3.5M3.75 7.75 8 3.5l4.25 4.25"
                stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"
            />
        </svg>
    }
}

/// Стрелка направления порядка (вниз).
fn IconArrowDown() -> impl IntoView {
    view! {
        <svg aria-hidden="true" width="11" height="11" viewBox="0 0 16 16" fill="none">
            <path
                d="M8 3v9.5M3.75 8.25 8 12.5l4.25-4.25"
                stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"
            />
        </svg>
    }
}

/// Переключатель алфавитного порядка: литеры «АЯ», цвет по текущему тексту.
fn IconAlpha() -> impl IntoView {
    view! {
        <svg aria-hidden="true" width="13" height="11" viewBox="0 0 16 12" fill="none">
            <text x="0" y="10" font-size="10" font-weight="700" fill="currentColor">АЯ</text>
        </svg>
    }
}

/// Количество правил в записи профиля: API шлёт число, полный объект — массив.
fn rules_count_of(p: &Value) -> usize {
    match p.get("rules") {
        Some(Value::Array(a)) => a.len(),
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0) as usize,
        _ => 0,
    }
}

async fn fetch_json_get(url: &str) -> Result<Value, String> {
    let promise = crate::util::ui_fetch(url, None, None)?;
    let resp_val = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|e| format!("{e:?}"))?;
    let resp: Response = resp_val.dyn_into().map_err(|e| format!("{e:?}"))?;
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let txt = resp.text().unwrap();
    let val = wasm_bindgen_futures::JsFuture::from(txt)
        .await
        .map_err(|e| format!("{e:?}"))?;
    let s = val.as_string().unwrap_or_default();
    serde_json::from_str(&s).map_err(|e| e.to_string())
}

async fn fetch_json_post(url: &str, body: &Value) -> Result<Value, String> {
    let promise = crate::util::ui_fetch(url, Some("POST"), Some(&body.to_string()))?;
    let resp_val = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|e| format!("{e:?}"))?;
    let resp: Response = resp_val.dyn_into().map_err(|e| format!("{e:?}"))?;
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let txt = resp.text().unwrap();
    let val = wasm_bindgen_futures::JsFuture::from(txt)
        .await
        .map_err(|e| format!("{e:?}"))?;
    let s = val.as_string().unwrap_or_default();
    serde_json::from_str(&s).map_err(|e| e.to_string())
}

#[component]
pub fn ProxyView() -> impl IntoView {
    let profiles = RwSignal::new(Vec::<Value>::new());
    let selected_profile = RwSignal::new(Option::<String>::None);
    let profile_data = RwSignal::new(Value::Null);
    let rules = RwSignal::new(Vec::<Value>::new());
    let selected_rule_idx = RwSignal::new(Option::<usize>::None);
    let variants = RwSignal::new(Vec::<Value>::new());
    let selected_variant_idx = RwSignal::new(Option::<usize>::None);
    let nav_sel = RwSignal::new(Option::<NavSel>::None);
    // Какое правило сейчас развёрнуто в центре; открыто всегда не больше одного.
    let open_rule = RwSignal::new(Option::<usize>::None);
    let status_msg = RwSignal::new(String::new());
    let status_err = RwSignal::new(false);
    let new_profile_open = RwSignal::new(false);
    let new_profile_name = RwSignal::new(String::new());
    let open_new_profile = move || {
        new_profile_name.set(String::new());
        new_profile_open.update(|o| *o = !*o);
    };
    crate::debug_bridge::register_click("mock.new_profile", open_new_profile);
    let snap_panel_id = crate::debug_bridge::register_snapshot("mock_panel", move || {
        serde_json::json!({
            "mock_new_profile_open": new_profile_open.try_get().unwrap_or(false),
            "mock_selected_profile": selected_profile.try_get().flatten(),
        })
    });
    // Модалка нового правила: тип совпадения и код ответа; название правила
    // ставится автоматически по коду из таблицы статусов REST (17.11.2026).
    let rule_name_open = RwSignal::new(false);
    let rule_method_draft = RwSignal::new("*".to_string());
    let rule_match_draft = RwSignal::new("prefix".to_string());
    let rule_status_draft = RwSignal::new("200".to_string());
    // Порядок списка профилей: по умолчанию история создания; иконки в шапке
    // переключают алфавитный порядок и направление (запрос пользователя 17.11.2026).
    let sort_alpha = RwSignal::new(false);
    let sort_desc = RwSignal::new(false);

    let load_profiles = move || {
        let order = if sort_alpha.get() { "alpha" } else { "history" };
        let dir = if sort_desc.get() { "desc" } else { "asc" };
        spawn_local(async move {
            match fetch_json_get(&format!("/debug/mock/profiles?order={order}&dir={dir}")).await {
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
            match fetch_json_get(&format!("/debug/mock/profile?name={}", name)).await {
                Ok(v) => {
                    // успешная загрузка снимает старый статус: после удаления и
                    // повторного создания профиля с тем же именем в панели висело
                    // «Профиль удалён» (инцидент 17.11.2026).
                    status_msg.set(String::new());
                    status_err.set(false);
                    let rules_arr = v
                        .get("rules")
                        .and_then(|x| x.as_array())
                        .cloned()
                        .unwrap_or_default();
                    rules.set(rules_arr.clone());
                    profile_data.set(v);
                    selected_profile.set(Some(name));
                    selected_rule_idx.set(None);
                    variants.set(Vec::new());
                    selected_variant_idx.set(None);
                    nav_sel.set(None);
                    open_rule.set(None);
                }
                Err(e) => {
                    status_msg.set(e);
                    status_err.set(true);
                }
            }
        });
    };

    // ----- Debug-мост: выбор mock-профиля командами отладочного API -----
    {
        let lp = load_profile;
        let sp = selected_profile;
        crate::debug_bridge::register_setter("mock.selected", move |v| match v.as_str() {
            Some(name) if !name.is_empty() => lp(name.to_string()),
            _ => sp.set(None),
        });
        let sp = selected_profile;
        let snap_mock_id = crate::debug_bridge::register_snapshot(
            "mock",
            move || serde_json::json!({ "selected": sp.try_get().flatten() }),
        );
        let sp = selected_profile;
        let reset_id = crate::debug_bridge::register_reset(move || sp.set(None));
        // Размонтирование вкладки Mock: убираем все регистрации этой вьюхи.
        // Иначе снимки и команды читают удалённые сигналы и останавливают
        // мост (инцидент 18.11.2026: после закрытия Mock встал BROOP).
        on_cleanup(move || {
            crate::debug_bridge::unregister_snapshot(snap_panel_id);
            crate::debug_bridge::unregister_snapshot(snap_mock_id);
            crate::debug_bridge::unregister_reset(reset_id);
            crate::debug_bridge::unregister_setter("mock.selected");
            crate::debug_bridge::unregister_click("mock.new_profile");
        });
    }

    let save_profile = move || {
        let data = profile_data.get();
        if data.is_null() {
            return;
        }
        spawn_local(async move {
            match fetch_json_post("/debug/mock/profile", &data).await {
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

    let del_profile_confirm = RwSignal::new(None::<String>);

    let delete_profile = move |name: String| {
        spawn_local(async move {
            let check = name.clone();
            let body = json!({"name": name});
            match fetch_json_post("/debug/mock/profile/delete", &body).await {
                Ok(_) => {
                    status_msg.set("Профиль удалён".into());
                    status_err.set(false);
                    if selected_profile.get_untracked().as_deref() == Some(check.as_str()) {
                        selected_profile.set(None);
                        profile_data.set(Value::Null);
                        rules.set(Vec::new());
                        nav_sel.set(None);
                    }
                    load_profiles();
                }
                Err(e) => {
                    status_msg.set(e);
                    status_err.set(true);
                }
            }
        });
    };

    let start_profile = move |name: String| {
        spawn_local(async move {
            let body = json!({"name": name});
            match fetch_json_post("/debug/mock/start", &body).await {
                Ok(v) => {
                    let port = v.get("port").and_then(|x| x.as_u64()).unwrap_or(0);
                    status_msg.set(format!("Запущен на порту {}", port));
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

    let stop_profile = move |name: String| {
        spawn_local(async move {
            let body = json!({"name": name});
            match fetch_json_post("/debug/mock/stop", &body).await {
                Ok(_) => {
                    status_msg.set("Остановлен".into());
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

    let create_profile = move || {
        // имя берём из inline-формы: window.prompt в webview Tauri не поддерживается (16.11.2026)
        let name = new_profile_name.get();
        let sanitized: String = name
            .chars()
            .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-')
            .collect();
        if sanitized.is_empty() {
            return;
        }
        let new_profile = json!({
            "name": sanitized,
            "listen_port": 3100,
            "upstream": Value::Null,
            "rules": []
        });
        spawn_local(async move {
            match fetch_json_post("/debug/mock/profile", &new_profile).await {
                Ok(_) => {
                    new_profile_open.set(false);
                    new_profile_name.set(String::new());
                    load_profiles();
                    load_profile(sanitized);
                }
                Err(e) => {
                    status_msg.set(e);
                    status_err.set(true);
                }
            }
        });
    };

    let select_rule = move |idx: usize| {
        selected_rule_idx.set(Some(idx));
        let rule = rules.get().get(idx).cloned().unwrap_or(Value::Null);
        let vars = rule
            .get("variants")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        variants.set(vars);
        selected_variant_idx.set(None);
        nav_sel.set(Some(NavSel {
            kind: NavKind::Match,
            rule: idx,
            variant: 0,
        }));
    };

    let update_profile_rules = move |new_rules: Vec<Value>| {
        let mut pd = profile_data.get();
        if let Some(obj) = pd.as_object_mut() {
            obj.insert("rules".into(), Value::Array(new_rules));
            profile_data.set(pd);
        }
    };

    // Правило создаётся из модалки: название, метод, тип совпадения и код
    // ответа задаёт пользователь (17.11.2026).
    let add_rule = move |method: String, match_type: String, status: u64| {
        // Название — автоматически по коду из таблицы статусов REST.
        let label = status_label(status);
        let mut r = rules.get();
        let new_rule = json!({
            "id": format!("r{}", r.len() + 1),
            "label": label.clone(),
            "method": method,
            "match": match_type,
            "path": "/api/",
            "enabled": true,
            "active_variant": "v1",
            "cycle": false,
            "variants": [{
                "id": "v1",
                "label": label,
                "status": status,
                "json": "{}",
                "randomize": false,
                "delay_ms": 0
            }]
        });
        r.push(new_rule);
        let new_idx = r.len() - 1;
        rules.set(r.clone());
        update_profile_rules(r.clone());
        selected_rule_idx.set(Some(new_idx));
        if let Some(Value::Array(v)) = r[new_idx].get("variants") {
            variants.set(v.clone());
        }
        selected_variant_idx.set(Some(0));
        nav_sel.set(Some(NavSel {
            kind: NavKind::Match,
            rule: new_idx,
            variant: 0,
        }));
        open_rule.set(Some(new_idx));
    };

    // Сохранение модалки нового правила: код ответа берём из поля, по
    // умолчанию 200; название подставляется автоматически по коду.
    let save_rule_draft = move || {
        let match_type = rule_match_draft.get();
        let method = rule_method_draft.get();
        let status = rule_status_draft
            .get()
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|s| (100..600).contains(s))
            .unwrap_or(200);
        rule_name_open.set(false);
        add_rule(method, match_type, status);
    };

    let delete_rule = move |idx: usize| {
        let mut r = rules.get();
        if idx < r.len() {
            r.remove(idx);
            rules.set(r.clone());
            update_profile_rules(r);
            selected_rule_idx.set(None);
            variants.set(Vec::new());
            nav_sel.set(None);
            open_rule.set(None);
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
            let vars_clone = {
                let rule = match r.get_mut(ridx).and_then(|x| x.as_object_mut()) {
                    Some(x) => x,
                    None => return,
                };
                let n = rule
                    .get("variants")
                    .and_then(|x| x.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                let vid = format!("v{}", n + 1);
                let new_var = json!({
                    "id": vid,
                    "label": status_label(200),
                    "status": 200,
                    "json": "{}",
                    "randomize": false,
                    "delay_ms": 0
                });
                rule.insert("active_variant".into(), Value::String(vid));
                match rule.get_mut("variants").and_then(|x| x.as_array_mut()) {
                    Some(vars) => {
                        vars.push(new_var);
                        vars.clone()
                    }
                    None => Vec::new(),
                }
            };
            rules.set(r.clone());
            update_profile_rules(r);
            variants.set(vars_clone.clone());
            if !vars_clone.is_empty() {
                selected_variant_idx.set(Some(vars_clone.len() - 1));
                nav_sel.set(Some(NavSel {
                    kind: NavKind::Variant,
                    rule: ridx,
                    variant: vars_clone.len() - 1,
                }));
                open_rule.set(Some(ridx));
            }
        }
    };

    let delete_variant = move |vidx: usize| {
        if let Some(ridx) = selected_rule_idx.get() {
            let mut r = rules.get();
            let vars_clone = {
                let rule = match r.get_mut(ridx).and_then(|x| x.as_object_mut()) {
                    Some(x) => x,
                    None => return,
                };
                let vars = match rule.get_mut("variants").and_then(|x| x.as_array_mut()) {
                    Some(x) => x,
                    None => return,
                };
                if vidx >= vars.len() {
                    return;
                }
                vars.remove(vidx);
                let first_id = vars
                    .first()
                    .and_then(|v| v.get("id"))
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string());
                let out = vars.clone();
                if let Some(id) = first_id {
                    rule.insert("active_variant".into(), Value::String(id));
                }
                out
            };
            rules.set(r.clone());
            update_profile_rules(r);
            variants.set(vars_clone);
            selected_variant_idx.set(None);
            open_rule.set(Some(ridx));
        }
    };

    let select_variant = move |vidx: usize| {
        selected_variant_idx.set(Some(vidx));
        if let Some(ridx) = selected_rule_idx.get() {
            nav_sel.set(Some(NavSel {
                kind: NavKind::Variant,
                rule: ridx,
                variant: vidx,
            }));
            let mut r = rules.get();
            if let Some(rule) = r.get_mut(ridx).and_then(|x| x.as_object_mut()) {
                if let Some(v) = variants
                    .get()
                    .get(vidx)
                    .and_then(|x| x.get("id"))
                    .and_then(|x| x.as_str())
                {
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
            let vars_clone = {
                let rule = match r.get_mut(ridx).and_then(|x| x.as_object_mut()) {
                    Some(x) => x,
                    None => return,
                };
                let vars = match rule.get_mut("variants").and_then(|x| x.as_array_mut()) {
                    Some(x) => x,
                    None => return,
                };
                if let Some(var) = vars.get_mut(vidx).and_then(|x| x.as_object_mut()) {
                    var.insert(key.into(), val);
                }
                vars.clone()
            };
            rules.set(r.clone());
            update_profile_rules(r);
            variants.set(vars_clone);
        }
    };

    let _on_profile_change = move |ev: web_sys::Event| {
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

    let on_variant_status_change = move |vidx: usize, ev: web_sys::Event| {
        let val = select_value(ev);
        if let Ok(s) = val.parse::<u16>() {
            update_variant_field(vidx, "status", Value::Number(s.into()));
            // Название варианта и правила — автоматически по коду из таблицы.
            update_variant_field(vidx, "label", Value::String(status_label(s as u64)));
            if let Some(ridx) = selected_rule_idx.get() {
                update_rule_field(ridx, "label", Value::String(status_label(s as u64)));
            }
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

    // ----- Центральная колонка: один логический блок на выбранный узел дерева -----
    let sv_pick = select_variant;
    let sr_pick = select_rule;
    let pick = move |sel: NavSel| match sel.kind {
        NavKind::Connect => nav_sel.set(Some(sel)),
        NavKind::Match => {
            if selected_rule_idx.get_untracked() != Some(sel.rule) {
                sr_pick(sel.rule);
            }
            open_rule.set(Some(sel.rule));
            nav_sel.set(Some(sel));
        }
        NavKind::Variant => {
            if selected_rule_idx.get_untracked() != Some(sel.rule) {
                sr_pick(sel.rule);
            }
            sv_pick(sel.variant);
            open_rule.set(Some(sel.rule));
            nav_sel.set(Some(sel));
        }
    };
    let av_nav = add_variant;
    let sr_nav = select_rule;
    let nav_add_variant = move |ri: usize| {
        if selected_rule_idx.get_untracked() != Some(ri) {
            sr_nav(ri);
        }
        av_nav();
    };
    let block_head =
        |title: &'static str| -> AnyView { view! { <div class="px-h">{title}</div> }.into_any() };
    let pick_cb: Callback<NavSel> = Callback::new(move |sel: NavSel| pick(sel));
    let add_rule_cb: Callback<()> = Callback::new(move |_: ()| {
        // «+» открывает модалку с типом и кодом; название — по коду (17.11.2026).
        rule_method_draft.set("*".to_string());
        rule_match_draft.set("prefix".to_string());
        rule_status_draft.set("200".to_string());
        rule_name_open.set(true);
    });
    let connect_block = move || -> AnyView {
        view! {
            <div class="px-block">
                {block_head("Соединение")}
                <div class="px-field">
                    <label class="px-label">"Имя"</label>
                    <input data-bh="mock.name" class="px-input" type="text" readonly=true
                        prop:value=move || profile_data.get().get("name").and_then(|x| x.as_str()).unwrap_or("").to_string() />
                </div>
                <div class="px-grid2">
                    <div class="px-field">
                        <label class="px-label">"Порт"</label>
                        <input data-bh="mock.listen_port" class="px-input" type="number" min="1" max="65535"
                            prop:value=move || profile_data.get().get("listen_port").and_then(|x| x.as_u64()).unwrap_or(0).to_string()
                            on:input=on_port_change />
                    </div>
                    <div class="px-field">
                        <label class="px-label">"Upstream (опционально)"</label>
                        <input data-bh="mock.upstream" class="px-input" type="text" placeholder="http://127.0.0.1:8080"
                            prop:value=move || profile_data.get().get("upstream").and_then(|x| x.as_str()).unwrap_or("").to_string()
                            on:input=on_upstream_change />
                    </div>
                </div>
                <button data-bh="mock.save_profile" class="px-btn primary" on:click=move |_| save_profile()>"Сохранить"</button>
            </div>
        }.into_any()
    };
    // Доступ к полям правила/варианта прямо из JSON правил, по индексам.
    let rfield =
        move |ri: usize, key: &str| -> Option<Value> { rules.get().get(ri)?.get(key).cloned() };
    // Опция помечается выбранной по значению правила. При первом монтировании
    // карточки из модалки prop:value срабатывает до вставки option-элементов,
    // и браузер оставляет первую опцию (баг модалки нового правила, 17.11.2026).
    // Атрибут selected применяется после вставки опций и доводит вид до истины.
    let opt_is = move |ri: usize, key: &str, val: &str| -> bool {
        rfield(ri, key)
            .and_then(|x| x.as_str().map(String::from))
            .as_deref()
            == Some(val)
    };
    let vfield = move |ri: usize, vi: usize, key: &str| -> Option<Value> {
        rules
            .get()
            .get(ri)?
            .get("variants")
            .and_then(|x| x.as_array())?
            .get(vi)?
            .get(key)
            .cloned()
    };
    // Правила — кнопка-спойлер на каждый rule: шапка как строка настроек
    // (шеврон слева, название по центру, статус справа; шеврон поворачивается
    // при открытии). Открыта всегда не больше одной кнопки. Внутри — условия
    // сопоставления и список вариантов; каждый вариант — вложенный спойлер,
    // новый вариант создаётся кнопкой «+» в теле правила.
    let rules_panel = move || -> AnyView {
        let ar_h = add_rule_cb;
        view! {
            <div class="px-block">
                <div class="px-h-row">
                    // В шапке только иконка «+»: слово убрано по просьбе
                    // пользователя 17.11.2026, смысл подсказывает тултип при наведении.
                    <button data-bh="mock.add_rule" class="px-mini" data-tip="Добавить правило" aria-label="Добавить правило"
                        on:click=move |_| { ar_h.run(()); }><IconPlus /></button>
                </div>
                <div class="mrules">
                    <For
                        each=move || {
                            let rs = rules.get();
                            rs.into_iter().enumerate().collect::<Vec<(usize, Value)>>()
                        }
                        key=|(_, r)| r.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string()
                        children=move |(ri, _rule)| {
                            let om = open_rule;
                            let sr2 = select_rule;
                            let dr2 = delete_rule;
                            let sp2 = save_profile;
                            let navav2 = nav_add_variant;
                            view! {
                                <details data-bh=format!("rule.row.{ri}") class="mrule"
                                    open=move || om.get() == Some(ri)>
                                    <summary class="mr-sum"
                                        on:click=move |ev| {
                                        // Единственный источник истины — сигнал `open_rule`.
                                        // Нативный toggle браузера гасим: иначе он и реактивный
                                        // эффект тянут `open` по очереди и нужен двойной-тройной клик.
                                        ev.prevent_default();
                                        if om.get() == Some(ri) {
                                            om.set(None);
                                        } else {
                                            sr2(ri);
                                            om.set(Some(ri));
                                        }
                                    }>
                                        <svg class="mr-chev" aria-hidden="true" width="14" height="14" viewBox="0 0 16 16" fill="none">
                                            <path d="M6 3.5 10.5 8 6 12.5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>
                                        </svg>
                                        <span class="mr-title">
                                            <span class="mr-method">{move || rfield(ri, "method").and_then(|x| x.as_str().map(String::from)).unwrap_or_else(|| "*".to_string())}</span>
                                            <span class="mr-path">{move || rfield(ri, "path").and_then(|x| x.as_str().map(String::from)).unwrap_or_else(|| "/".to_string())}</span>
                                            <span class="mr-sep" aria-hidden="true">"|"</span>
                                            <span class="mr-variants-count">{move || {
                                                let n = rfield(ri, "variants").and_then(|x| x.as_array().map(|a| a.len())).unwrap_or(0);
                                                format!("вариантов ответа — {n}")
                                            }}</span>
                                        </span>
                                        <span class="mr-tail">{move || {
                                            let rs = rules.get();
                                            let Some(r) = rs.get(ri) else { return view! { <div></div> }.into_any() };
                                            let enabled = r.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
                                            let vars = r.get("variants").and_then(|x| x.as_array()).cloned().unwrap_or_default();
                                            if enabled && !vars.iter().any(var_bad) {
                                                return view! { <div></div> }.into_any();
                                            }
                                            view! { <span class="mnav-dot" title="Правило выключено или вариант с битыми данными"/> }.into_any()
                                        }}</span>
                                    </summary>
                                    <div class="mr-body">
                                        <div class="px-grid2">
                                            <div class="px-field">
                                                <label class="px-label">"Method"</label>
                                                <select data-bh=format!("rule.method.{ri}") class="px-input"
                                                    prop:value=move || rfield(ri, "method").and_then(|x| x.as_str().map(String::from)).unwrap_or_else(|| "*".to_string())
                                                    on:change=move |ev| on_rule_method_change(ri, ev)>
                                                    <option value="*" selected=move || opt_is(ri, "method", "*")>"*"</option>
                                                    <option value="GET" selected=move || opt_is(ri, "method", "GET")>"GET"</option>
                                                    <option value="POST" selected=move || opt_is(ri, "method", "POST")>"POST"</option>
                                                    <option value="PUT" selected=move || opt_is(ri, "method", "PUT")>"PUT"</option>
                                                    <option value="PATCH" selected=move || opt_is(ri, "method", "PATCH")>"PATCH"</option>
                                                    <option value="DELETE" selected=move || opt_is(ri, "method", "DELETE")>"DELETE"</option>
                                                </select>
                                            </div>
                                            <div class="px-field">
                                                <label class="px-label">"Match"</label>
                                                <select data-bh=format!("rule.match.{ri}") class="px-input"
                                                    prop:value=move || rfield(ri, "match").and_then(|x| x.as_str().map(String::from)).unwrap_or_else(|| "prefix".to_string())
                                                    on:change=move |ev| on_rule_match_change(ri, ev)>
                                                    <option value="exact" selected=move || opt_is(ri, "match", "exact")>"exact"</option>
                                                    <option value="prefix" selected=move || opt_is(ri, "match", "prefix")>"prefix"</option>
                                                </select>
                                            </div>
                                        </div>
                                        <div class="px-field">
                                            <label class="px-label">"Path"</label>
                                            <input data-bh=format!("rule.path.{ri}") class="px-input" type="text"
                                                prop:value=move || rfield(ri, "path").and_then(|x| x.as_str().map(String::from)).unwrap_or_default()
                                                on:input=move |ev| on_rule_path_change(ri, ev) />
                                        </div>
                                        <div class="mr-checks">
                                            <label class="px-check-label">
                                                <input data-bh=format!("rule.enabled.{ri}") type="checkbox" class="px-check"
                                                    prop:checked=move || rfield(ri, "enabled").and_then(|x| x.as_bool()).unwrap_or(true)
                                                    on:change=move |ev| on_rule_enabled_change(ri, ev) />
                                                "Включено"
                                            </label>
                                            <label class="px-check-label">
                                                <input data-bh=format!("rule.cycle.{ri}") type="checkbox" class="px-check"
                                                    prop:checked=move || rfield(ri, "cycle").and_then(|x| x.as_bool()).unwrap_or(false)
                                                    on:change=move |ev| on_rule_cycle_change(ri, ev) />
                                                "Циклически"
                                            </label>
                                        </div>
                                        <div class="px-h-row">
                                            <div class="px-h">"Варианты ответа"</div>
                                            <button data-bh=format!("rule.new_variant.{ri}") class="px-mini" title="Новый вариант" aria-label="Новый вариант"
                                                on:click=move |_| { navav2(ri); }><IconPlus /></button>
                                        </div>
                                        <Show when=move || {
                                            let rs = rules.get();
                                            rs.get(ri)
                                                .and_then(|r| r.get("variants").and_then(|x| x.as_array()))
                                                .map(|v| v.is_empty())
                                                .unwrap_or(true)
                                        } fallback=move || { view! { <div></div> } }>
                                            <div class="mnav-hint">"Нажми «+», чтобы создать вариант ответа."</div>
                                        </Show>
                                        <div class="mvars">
                                            <For
                                                each=move || {
                                                    let rs = rules.get();
                                                    rs.get(ri)
                                                        .and_then(|r| r.get("variants").and_then(|x| x.as_array()).cloned())
                                                        .unwrap_or_default()
                                                        .into_iter()
                                                        .enumerate()
                                                        .collect::<Vec<(usize, Value)>>()
                                                }
                                                key=|(_, v)| v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string()
                                                children=move |(vi, _v)| {
                                                    let sr3 = select_rule;
                                                    let sv2 = select_variant;
                                                    let dv2 = delete_variant;
                                                    view! {
                                                        <details data-bh=format!("variant.row.{ri}.{vi}") class="mvar"
                                                            open=move || selected_rule_idx.get() == Some(ri) && selected_variant_idx.get() == Some(vi)>
                                                            <summary class="mv-sum"
                                                                on:click=move |ev| {
                                                                let Some(t) = ev.target() else { return };
                                                                let Some(el) = t.dyn_ref::<web_sys::Element>() else { return };
                                                                // Кнопка-переключатель внутри строки не должна схлопывать редактор.
                                                                if let Ok(Some(_)) = el.closest("button") { return; }
                                                                let Ok(Some(sum)) = el.closest("summary") else { return };
                                                                let det = match sum.parent_element()
                                                                    .and_then(|p| p.dyn_into::<web_sys::HtmlDetailsElement>().ok())
                                                                {
                                                                    Some(d) => d,
                                                                    None => return,
                                                                };
                                                                if det.open() {
                                                                    if selected_variant_idx.get() == Some(vi) {
                                                                        selected_variant_idx.set(None);
                                                                    }
                                                                } else {
                                                                    sr3(ri);
                                                                    sv2(vi);
                                                                }
                                                            }>
                                                                <svg class="mr-chev" aria-hidden="true" width="12" height="12" viewBox="0 0 16 16" fill="none">
                                                                    <path d="M6 3.5 10.5 8 6 12.5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>
                                                                </svg>
                                                                <span class="mv-label">{move || {
                                                                    let lab = vfield(ri, vi, "label").and_then(|x| x.as_str().map(String::from)).unwrap_or_default();
                                                                    if !lab.is_empty() { lab }
                                                                    else { vfield(ri, vi, "id").and_then(|x| x.as_str().map(String::from)).unwrap_or_else(|| "вариант".to_string()) }
                                                                }}</span>
                                                                <span class="mv-tail">
                                                                    {move || {
                                                                        let rs = rules.get();
                                                                        let act = rs.get(ri)
                                                                            .and_then(|r| r.get("active_variant").and_then(|x| x.as_str()).map(String::from))
                                                                            .unwrap_or_default();
                                                                        let vid = vfield(ri, vi, "id").and_then(|x| x.as_str().map(String::from)).unwrap_or_default();
                                                                        let on = !act.is_empty() && act == vid;
                                                                        view! {
                                                                            <button data-bh=format!("variant.activate.{ri}.{vi}") class={if on { "mv-active on" } else { "mv-active off" }}
                                                                                on:click={move |e| {
                                                                                    e.prevent_default();
                                                                                    e.stop_propagation();
                                                                                    let mut r = rules.get();
                                                                                    if let Some(ro) = r.get_mut(ri).and_then(|x| x.as_object_mut()) {
                                                                                        let nv = if on { String::new() } else { vid.clone() };
                                                                                        ro.insert("active_variant".into(), Value::String(nv));
                                                                                    }
                                                                                    rules.set(r.clone());
                                                                                    update_profile_rules(r);
                                                                                }}>{if on { "активный" } else { "активировать" }}</button>
                                                                        }.into_any()
                                                                    }}
                                                                </span>
                                                            </summary>
                                                            <div class="mv-body">
                                                                <div class="px-ctl-row">
                                                                    <div class="px-field">
                                                                        <label class="px-label">"Status"</label>
                                                                        <select data-bh=format!("variant.status.{ri}.{vi}") class="px-input"
                                                                            on:change=move |ev| on_variant_status_change(vi, ev)>
                                                                            {STATUS_CLASSES
                                                                                .iter()
                                                                                .map(|cls| {
                                                                                    view! { <optgroup label=cls.title>{
                                                                                        cls.codes
                                                                                            .iter()
                                                                                            .map(|(code, _phrase)| {
                                                                                                let cur = code.to_string();
                                                                                                view! {
                                                                                                    <option
                                                                                                        value=cur.clone()
                                                                                                        selected=move || {
                                                                                                            vfield(ri, vi, "status").and_then(|x| x.as_u64()).map(|s| s.to_string()) == Some(cur.clone())
                                                                                                        }
                                                                                                    >{{code.to_string()}}</option>
                                                                                                }
                                                                                            })
                                                                                            .collect::<Vec<_>>()
                                                                                    }</optgroup> }
                                                                                })
                                                                                .collect::<Vec<_>>()}
                                                                        </select>
                                                                    </div>
                                                                    <div class="px-field">
                                                                        <label class="px-label">"Delay ms"</label>
                                                                        <div class="px-stepper">
                                                                         <button data-bh=format!("variant.delay_down.{ri}.{vi}") class="px-step-btn" type="button" aria-label="Меньше"
                                                                             on:click=move |_| {
    let cur = vfield(ri, vi, "delay_ms").and_then(|x| x.as_u64()).unwrap_or(0);
    update_variant_field(vi, "delay_ms", Value::Number(cur.saturating_sub(100).into()));
}>
<svg aria-hidden="true" width="12" height="12" viewBox="0 0 16 16" fill="none"><path d="M3.5 8h9" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
</button>
                                                                         <input data-bh=format!("variant.delay.{ri}.{vi}") class="px-step-val" type="number" min="0" max="600000" step="100"
                                                                             title="Кликни по числу и впиши сам"
                                                                             prop:value=move || vfield(ri, vi, "delay_ms").and_then(|x| x.as_u64()).unwrap_or(0).to_string()
                                                                             on:input=move |ev| on_variant_delay_change(vi, ev) />
                                                                         <button data-bh=format!("variant.delay_up.{ri}.{vi}") class="px-step-btn" type="button" aria-label="Больше"
                                                                             on:click=move |_| {
    let cur = vfield(ri, vi, "delay_ms").and_then(|x| x.as_u64()).unwrap_or(0);
    update_variant_field(vi, "delay_ms", Value::Number(cur.saturating_add(100).min(600_000).into()));
}>
<svg aria-hidden="true" width="12" height="12" viewBox="0 0 16 16" fill="none"><path d="M8 3.5v9M3.5 8h9" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
</button>
                                                                        </div>
                                                                    </div>
                                                                </div>
                                                                <div class="px-field px-check-field">
                                                                    <label class="px-check-label">
                                                                        <input data-bh=format!("variant.randomize.{ri}.{vi}") type="checkbox" class="px-check"
                                                                            prop:checked=move || vfield(ri, vi, "randomize").and_then(|x| x.as_bool()).unwrap_or(false)
                                                                            on:change=move |ev| on_variant_randomize_change(vi, ev) />
                                                                        "Рандомизировать {{...}}"
                                                                    </label>
                                                                </div>
<div class="px-field">
                                                                    <div class="px-json-head"><label class="px-label">"JSON Body"</label><details class="px-help">
                                                                    <summary>"Синтаксис {{токенов}}"</summary>
                                                                    <pre class="px-help-list">{r#"{{guid}}  {{bool}}  {{int:MIN:MAX}}  {{float:MIN:MAX:DEC}}
{{string:N}}  {{hex:N}}  {{email}}  {{phone}}  {{inn}}  {{snils}}
{{name}}  {{date:ДНЕЙ_СДВИГ}}  {{datetime:ДНЕЙ_СДВИГ}}  {{pick:A|B|C}}"#}</pre>
                                                                </details></div>
                                                                    <textarea data-bh=format!("variant.json.{ri}.{vi}") class="px-json" rows="14"
                                                                        prop:value=move || vfield(ri, vi, "json").and_then(|x| x.as_str().map(String::from)).unwrap_or_else(|| "{}".to_string())
                                                                        on:input=move |ev| on_variant_json_change(vi, ev) />
                                                                </div>
                                                                <div class="px-btn-row">
                                                                    <button data-bh=format!("variant.make_active.{ri}.{vi}") class="px-btn" on:click=move |_| { sv2(vi); }>"Сделать активным"</button><button data-bh=format!("variant.delete.{ri}.{vi}") class="px-btn" on:click=move |_| { dv2(vi); }>"Удалить вариант"</button></div>
                                                            </div>
                                                        </details>
                                                    }.into_any()
                                                }
                                            />
                                        </div>
                                        <div class="px-grid2">
                                            <button data-bh="rule.save" class="px-btn primary" on:click=move |_| { sp2(); }>"Сохранить профиль"</button>
                                            <button data-bh=format!("rule.delete.{ri}") class="px-btn" on:click=move |_| { dr2(ri); }>"Удалить правило"</button>
                                        </div>
                                    </div>
                                </details>
                            }.into_any()
                        }
                    />
                </div>
            </div>
        }.into_any()
    };

    view! {
        <div class="px-layout">
            <div class="px-side">
                <div class="px-h-row">
                    <div class="px-h">Профили</div>
                    <div class="px-h-tools">
                        <button data-bh="sidebar.sort_desc" class="px-mini tip-right"
                            data-tip=move || if sort_desc.get() { "По возрастанию" } else { "По убыванию" }
                            aria-label="Направление порядка"
                            on:click=move |_| {
                                sort_desc.update(|d| *d = !*d);
                                load_profiles();
                            }>
                            {move || if sort_desc.get() {
                                view! { <IconArrowUp /> }.into_any()
                            } else {
                                view! { <IconArrowDown /> }.into_any()
                            }}
                        </button>
                        <button data-bh="sidebar.sort_alpha" class=move || if sort_alpha.get() { "px-mini tip-right on" } else { "px-mini tip-right" }
                            data-tip=move || if sort_alpha.get() { "Порядок истории создания" } else { "Порядок по алфавиту" }
                            aria-label="Порядок списка"
                            on:click=move |_| {
                                sort_alpha.update(|a| *a = !*a);
                                load_profiles();
                            }>
                            <IconAlpha />
                        </button>
                        <button data-bh="sidebar.new_profile" class="px-mini tip-right" data-tip="Создать профиль" aria-label="Создать профиль"
                            on:click=move |_| open_new_profile()>
                            <IconPlus />
                        </button>
                    </div>
                </div>
                <Show when=move || new_profile_open.get()>
                    <div class="px-field">
                        <input data-bh="profile.new_name" class="px-input" type="text" placeholder="profile-name"
                            autofocus=true
                            prop:value=move || new_profile_name.get()
                            on:input=move |ev| new_profile_name.set(input_value(ev))
                            on:keydown=move |ev| {
                                if ev.key() == "Enter" { create_profile(); }
                            } />
                        <div style="display:flex;gap:6px;margin-top:6px">
                            <button data-bh="profile.create" class="px-btn" on:click=move |_| create_profile()>"Создать"</button>
                            <button data-bh="profile.create_cancel" class="px-btn" on:click=move |_| new_profile_open.set(false)>"Отмена"</button>
                        </div>
                    </div>
                </Show>
                <div class="px-rules">
                    <For
                        each=move || profiles.get()
                        key=|p| {
                            let name = p.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                            let running = p.get("running").and_then(|x| x.as_bool()).unwrap_or(false);
                            let cnt = rules_count_of(p);
                            format!("{name}:{running}:{cnt}")
                        }
                        children=move |p| {
                            let name = p.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                            let running = p.get("running").and_then(|x| x.as_bool()).unwrap_or(false);
                            let is_active = selected_profile.get().as_deref() == Some(&name);
                            let mut item_cls = String::from("px-item");
                            if is_active {
                                item_cls.push_str(" active");
                            }
                            if running {
                                item_cls.push_str(" running");
                            }
                            let dot_cls = if running { "px-dot on" } else { "px-dot" };
                            let pp_cls = if running { "px-pp on" } else { "px-pp" };
                            let pp_title = if running { "Остановить" } else { "Запустить" };
                            let name_click = name.clone();
                            let name_del = name.clone();
                            let name_pp = name.clone();
                            let bh_row = format!("profile.row.{name}");
                            let bh_pp = format!("profile.start_stop.{name}");
                            let bh_del = format!("profile.delete.{name}");
                            view! {
                                <div data-bh=bh_row class=item_cls
                                    on:click=move |_| load_profile(name_click.clone())>
                                    <span class=dot_cls></span>
                                    <span class="px-name">{name}</span>
                                    <button data-bh=bh_pp class=pp_cls title=pp_title
                                        on:click=move |e| {
                                            e.stop_propagation();
                                            if running {
                                                stop_profile(name_pp.clone());
                                            } else {
                                                start_profile(name_pp.clone());
                                            }
                                        }>
                                        <svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true">
                                            {if running {
                                                view! { <rect x="4.5" y="4.5" width="7" height="7" rx="1" fill="none" stroke="currentColor" stroke-width="1.6"/> }.into_any()
                                            } else {
                                                view! { <path d="M5.5 3.8v8.4L13 8z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"/> }.into_any()
                                            }}
                                        </svg>
                                    </button>
                                    <button data-bh=bh_del class="px-del" title="Удалить профиль"
                                        on:click=move |e| {
                                            e.stop_propagation();
                                            del_profile_confirm.set(Some(name_del.clone()));
                                        }>"✕"</button>
                                </div>
                            }
                        }
                    />
                </div>
                <Show when=move || del_profile_confirm.get().is_some()>
                    <div class="modal-overlay"
                        on:click=move |ev| {
                            let on_overlay = ev
                                .target()
                                .and_then(|t| t.dyn_ref::<web_sys::HtmlElement>().cloned())
                                .map(|t| t.class_list().contains("modal-overlay"))
                                .unwrap_or(false);
                            if on_overlay {
                                del_profile_confirm.set(None);
                            }
                        }>
                        <div class="modal">
                            <div class="modal-title">{move || {
                                let name = del_profile_confirm.get().unwrap_or_default();
                                format!("Удалить профиль «{name}»?")
                            }}</div>
                            <div class="modal-actions">
                                <button data-bh="modal.del_profile_confirm" class="modal-save danger" on:click=move |_| {
                                    if let Some(name) = del_profile_confirm.get() {
                                        delete_profile(name);
                                    }
                                    del_profile_confirm.set(None);
                                }>"Удалить"</button>
                                <button data-bh="modal.del_profile_cancel" class="modal-cancel" on:click=move |_| del_profile_confirm.set(None)>"Отмена"</button>
                            </div>
                        </div>
                    </div>
                </Show>
                // Модалка нового правила: название обязательно, тип совпадения и
                // код ответа как в редакторе правил (просьба пользователя 17.11.2026).
                <Show when=move || rule_name_open.get()>
                    <div class="modal-overlay"
                        on:click=move |ev| {
                            let on_overlay = ev
                                .target()
                                .and_then(|t| t.dyn_ref::<web_sys::HtmlElement>().cloned())
                                .map(|t| t.class_list().contains("modal-overlay"))
                                .unwrap_or(false);
                            if on_overlay {
                                rule_name_open.set(false);
                            }
                        }>
                        <div class="modal">
                            <div class="modal-title">"Новое правило"</div>
                            <div class="modal-hint">"Метод запроса"</div>
                            <select data-bh="modal.rule_method" class="modal-input"
                                on:change=move |ev| {
                                    rule_method_draft.set(select_value(ev));
                                }>
                                <option value="*" selected=move || rule_method_draft.get() == "*">"* (любой метод)"</option>
                                <option value="GET" selected=move || rule_method_draft.get() == "GET">"GET"</option>
                                <option value="POST" selected=move || rule_method_draft.get() == "POST">"POST"</option>
                                <option value="PUT" selected=move || rule_method_draft.get() == "PUT">"PUT"</option>
                                <option value="PATCH" selected=move || rule_method_draft.get() == "PATCH">"PATCH"</option>
                                <option value="DELETE" selected=move || rule_method_draft.get() == "DELETE">"DELETE"</option>
                            </select>
                            <div class="modal-hint">"Тип совпадения"</div>
                            <select data-bh="modal.rule_match" class="modal-input"
                                on:change=move |ev| rule_match_draft.set(select_value(ev))>
                                <option value="exact" selected=move || rule_match_draft.get() == "exact">"exact (точное совпадение пути)"</option>
                                <option value="prefix" selected=move || rule_match_draft.get() == "prefix">"prefix (по префиксу пути)"</option>
                            </select>
                            <div class="modal-hint">"Код ответа — название правила подставится автоматически"</div>
                            <select data-bh="modal.rule_status" class="modal-input"
                                on:change=move |ev| rule_status_draft.set(select_value(ev))>
                                {STATUS_CLASSES
                                    .iter()
                                    .map(|cls| {
                                        view! { <optgroup label=cls.title>{
                                            cls.codes
                                                .iter()
                                                .map(|(code, _phrase)| {
                                                    let cur = code.to_string();
                                                    view! {
                                                        <option
                                                            value=cur.clone()
                                                            selected=move || rule_status_draft.get() == cur
                                                        >{{code.to_string()}}</option>
                                                    }
                                                })
                                                .collect::<Vec<_>>()
                                        }</optgroup> }
                                    })
                                    .collect::<Vec<_>>()}
                            </select>
                            <div class="modal-actions">
                                <button data-bh="modal.rule_add" class="modal-save" on:click=move |_| save_rule_draft()>"Добавить"</button>
                                <button data-bh="modal.rule_cancel" class="modal-cancel" on:click=move |_| rule_name_open.set(false)>"Отмена"</button>
                            </div>
                        </div>
                    </div>
                </Show>
            </div>

            <div class="px-main">
                <Show when=move || selected_profile.get().is_some() fallback=move || { view! { <div class="px-empty">Выберите или создайте профиль слева</div> } }>
                    // Центральная колонка: настройки соединения или список
                    // правил-спойлеров; раздел правил открыт по умолчанию.
                    {move || match nav_sel.get().map(|s| s.kind) {
                        Some(NavKind::Connect) => connect_block(),
                        _ => rules_panel(),
                    }}
                </Show>
                <Show when=move || selected_profile.get().is_some() && !status_msg.get().is_empty()>
                    <div class=move || if status_err.get() { "px-status err" } else { "px-status ok" }>{move || status_msg.get()}</div>
                </Show>
            </div>
            <div class="px-nav-col">
                <MockNav
                    profile=profile_data
                    current=nav_sel
                    pick=pick_cb
                />
            </div>
        </div>
    }
}
