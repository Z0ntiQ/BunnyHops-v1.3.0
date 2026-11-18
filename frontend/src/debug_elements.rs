//! Библиотека элементов: DOM-объекты с атрибутом `data-bh` как управляемые сущности.
//!
//! Каждый мутабельный элемент размечен `data-bh="<домен>.<элемент>"` — имя совпадает
//! с путью сигнала в `debug_bridge`. Мост отдаёт библиотеку по `GET /debug/elements`
//! и выполняет действия над элементом по его id: клик — настоящий DOM-клик через
//! событие (полный путь обработчиков, как в Selenium), значение — установка value с
//! событиями `input`/`change`.
//!
//! Источник истины — живой DOM: библиотека собирается по запросу, без реестра в коде.

use serde_json::{json, Value};
use wasm_bindgen::JsCast;
use web_sys::{
    Document, Event, EventInit, HtmlElement, HtmlInputElement, HtmlSelectElement,
    HtmlTextAreaElement, MouseEvent, MouseEventInit,
};

/// Найти документ (wasm живёт в окне браузера).
fn doc() -> Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Элемент по имени библиотеки (`data-bh`).
fn find(id: &str) -> Option<HtmlElement> {
    doc()
        .query_selector(&format!("[data-bh=\"{id}\"]"))
        .ok()
        .flatten()
        .and_then(|el| el.dyn_into::<HtmlElement>().ok())
}

/// Текст элемента для подписи (обрезан, чтобы библиотека не разрасталась).
fn short_text(el: &HtmlElement) -> String {
    let mut t = el
        .get_attribute("aria-label")
        .or_else(|| el.get_attribute("title"))
        .unwrap_or_default();
    if t.is_empty() {
        t = el.text_content().unwrap_or_default();
    }
    t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.len() > 60 {
        t.truncate(60);
        t.push('…');
    }
    t
}

/// Событие с всплытием: Dioxus слушает делегированно от корня документа.
fn dispatch(el: &HtmlElement, kind: &str) {
    let init = EventInit::new();
    init.set_bubbles(true);
    if let Ok(ev) = Event::new_with_event_init_dict(kind, &init) {
        let _ = el.dispatch_event(&ev);
    }
}

/// Дескриптор одного элемента: id, тип, значение, текст, геометрия, видимость.
fn describe(el: &HtmlElement) -> Value {
    let rect = el.get_bounding_client_rect();
    let tag = el.tag_name().to_lowercase();
    let mut item = json!({
        "id": el.get_attribute("data-bh").unwrap_or_default(),
        "tag": tag,
        "visible": rect.width() > 0.0 && rect.height() > 0.0,
        "enabled": !el.has_attribute("disabled"),
        "rect": [rect.x(), rect.y(), rect.width(), rect.height()],
    });
    if let Some(input) = el.dyn_ref::<HtmlInputElement>() {
        let obj = item.as_object_mut().unwrap();
        obj.insert("type".into(), json!(input.type_()));
        if input.type_() == "checkbox" {
            obj.insert("value".into(), json!(input.checked()));
        } else {
            obj.insert("value".into(), json!(input.value()));
        }
    } else if let Some(area) = el.dyn_ref::<HtmlTextAreaElement>() {
        let obj = item.as_object_mut().unwrap();
        obj.insert("type".into(), json!("textarea"));
        obj.insert("value".into(), json!(area.value()));
    } else if let Some(select) = el.dyn_ref::<HtmlSelectElement>() {
        let obj = item.as_object_mut().unwrap();
        obj.insert("type".into(), json!("select"));
        obj.insert("value".into(), json!(select.value()));
    } else {
        let text = short_text(el);
        if !text.is_empty() {
            item.as_object_mut()
                .unwrap()
                .insert("text".into(), json!(text));
        }
    }
    item
}

/// Вся библиотека элементов страницы, отсортированная по id.
pub fn collect() -> Vec<Value> {
    let nodes = match doc().query_selector_all("[data-bh]") {
        Ok(n) => n,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<Value> = Vec::new();
    for i in 0..nodes.length() {
        let el = match nodes.item(i) {
            Some(el) => el,
            None => continue,
        };
        if let Some(html) = el.dyn_ref::<HtmlElement>() {
            out.push(describe(html));
        }
    }
    out.sort_by(|a, b| {
        a.get("id")
            .and_then(Value::as_str)
            .cmp(&b.get("id").and_then(Value::as_str))
    });
    out
}

/// Интерактивные элементы без `data-bh`: гейт полноты разметки.
/// Элемент внутри уже размеченного родителя не считается — его покрывает id родителя.
pub fn unmarked() -> Vec<Value> {
    let nodes = match doc()
        .query_selector_all("button,input,select,textarea,a,[role=button]")
    {
        Ok(n) => n,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<Value> = Vec::new();
    for i in 0..nodes.length() {
        let el = match nodes.item(i) {
            Some(el) => el,
            None => continue,
        };
        let Some(html) = el.dyn_ref::<HtmlElement>() else {
            continue;
        };
        if html.has_attribute("data-bh") {
            continue;
        }
        if html.closest("[data-bh]").ok().flatten().is_some() {
            continue;
        }
        out.push(json!({
            "tag": html.tag_name().to_lowercase(),
            "class": html.class_name(),
            "text": short_text(html),
        }));
    }
    out
}

/// Дескриптор одного элемента по id (для точечной проверки).
pub fn query(id: &str) -> Option<Value> {
    find(id).map(|el| describe(&el))
}

/// Настоящий DOM-клик по элементу: событие проходит весь путь обработчиков UI.
pub fn click(id: &str) -> bool {
    match find(id) {
        Some(el) => {
            // Проверено в WKWebView (18.11.2026): родной `el.click()` доходит до
            // делегированных обработчиков Leptos и на button, и на div; явный
            // `dispatchEvent(new MouseEvent)` WebKit до них не доносит.
            el.click();
            true
        }
        None => false,
    }
}

/// Выполнить JS в окне и вернуть результат строкой. Точечный инструмент,
/// когда готового действия нет: проверка гипотез про обработчики и состояние.
pub fn eval(expr: &str) -> String {
    match js_sys::eval(expr) {
        Ok(v) => {
            let s = match js_sys::JSON::stringify(&v) {
                Ok(s) => String::from(s),
                Err(_) => format!("{:?}", v),
            };
            if s.chars().count() > 4000 {
                return format!("{}…(обрезано)", s.chars().take(4000).collect::<String>());
            }
            s
        }
        Err(e) => format!("ошибка eval: {:?}", e),
    }
}

/// Установить значение элемента и известить UI событиями `input`/`change`.
pub fn set_value(id: &str, value: &Value) -> bool {
    let Some(el) = find(id) else {
        return false;
    };
    if let Some(input) = el.dyn_ref::<HtmlInputElement>() {
        if input.type_() == "checkbox" {
            input.set_checked(value.as_bool().unwrap_or(false));
        } else {
            input.set_value(&value.as_str().map(String::from).unwrap_or_else(|| value.to_string()));
        }
        dispatch(&el, "input");
        dispatch(&el, "change");
        return true;
    }
    if let Some(area) = el.dyn_ref::<HtmlTextAreaElement>() {
        area.set_value(&value.as_str().unwrap_or(""));
        dispatch(&el, "input");
        return true;
    }
    if let Some(select) = el.dyn_ref::<HtmlSelectElement>() {
        select.set_value(&value.as_str().unwrap_or(""));
        dispatch(&el, "change");
        return true;
    }
    false
}
