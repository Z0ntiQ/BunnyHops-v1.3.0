//! doStream — выполнение запроса через /proxy со стримингом. 1:1 с App.jsx.

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AbortSignal, Blob, BlobPropertyBag, ReadableStreamDefaultReader, Request, RequestInit,
    Response, TextDecodeOptions, TextDecoder,
};

use crate::ui::js_now;

#[derive(Clone)]
pub struct RespUpdate {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub content_type: String,
    pub final_url: String,
    pub is_textual: bool,
}

pub struct StreamOutcome {
    pub network_error: bool,
    pub status: u16,
    pub time_ms: f64,
    pub bytes: u64,
    pub text: String,
    pub image_blob_url: Option<String>,
}

pub fn is_abort_err(e: &JsValue) -> bool {
    js_sys::Reflect::get(e, &JsValue::from_str("name"))
        .map(|v| v.as_string().unwrap_or_default() == "AbortError")
        .unwrap_or(false)
}

pub fn err_message(e: &JsValue) -> String {
    if let Some(s) = e.as_string() {
        return s;
    }
    js_sys::Reflect::get(e, &JsValue::from_str("message"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| format!("{e:?}"))
}

/// Выполняет запрос; head — вызывается при получении заголовков ответа
/// (в React это был setResponse({... streaming: true})), on_chunk — flush тела.
pub async fn do_stream(
    target: &str,
    method: &str,
    headers_obj: &[(String, String)],
    body_text: &str,
    signal: &AbortSignal,
    head: impl FnOnce(RespUpdate),
    mut on_chunk: impl FnMut(u64, String),
) -> Result<StreamOutcome, JsValue> {
    let start = js_now();
    let win = web_sys::window().unwrap();

    let mut h = Vec::new();
    for (k, v) in headers_obj {
        if !k.trim().is_empty() {
            h.push((k.clone(), v.clone()));
        }
    }
    let proxy_url = format!(
        "/debug/proxy?url={}",
        crate::logic::js_encode_uri_component(target)
    );
    let init = RequestInit::new();
    init.set_method(method);
    let hobj = js_sys::Object::new();
    let headers_json = serde_json::to_string(
        &h.iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect::<serde_json::Map<_, _>>(),
    )
    .unwrap();
    // порядок ключей в JSON сохраняем вручную: serde_json::Map сортирует только с feature "preserve_order";
    // для контракта прокси порядок не значим.
    js_sys::Reflect::set(
        &hobj,
        &JsValue::from_str("x-proxy-headers"),
        &JsValue::from_str(&headers_json),
    )
    .unwrap();
    // запросы из интерфейса не попадают в лог внешнего API
    let _ = js_sys::Reflect::set(
        &hobj,
        &JsValue::from_str("x-hop-ui"),
        &JsValue::from_str("1"),
    );
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
        let text = JsFuture::from(resp.text()?)
            .await?
            .as_string()
            .unwrap_or_default();
        return Ok(StreamOutcome {
            network_error: true,
            status: resp.status() as u16,
            time_ms: 0.0,
            bytes: 0,
            text: if text.is_empty() {
                "Не удалось подключиться".to_string()
            } else {
                text
            },
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
        .iter()
        .find(|(k, _)| k == "content-type")
        .map(|(_, v)| v.to_lowercase())
        .unwrap_or_default();
    let is_textual = !content_type.starts_with("image/") && !content_type.contains("octet-stream");
    let final_url = headers
        .iter()
        .find(|(k, _)| k == "x-proxy-final-url")
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| target.to_string());

    head(RespUpdate {
        status: resp.status() as u16,
        status_text: resp.status_text(),
        headers,
        content_type: content_type.clone(),
        final_url,
        is_textual,
    });

    // чтение потока
    let reader: ReadableStreamDefaultReader =
        resp.body().expect("stream body").get_reader().dyn_into()?;
    let decoder = TextDecoder::new().unwrap();
    let opts = TextDecodeOptions::new();
    opts.set_stream(true);

    let mut bytes: u64 = 0;
    let mut text = String::new();
    let binary_chunks = js_sys::Array::new();

    loop {
        let res = JsFuture::from(reader.read()).await?;
        let done = js_sys::Reflect::get(&res, &JsValue::from_str("done"))
            .unwrap()
            .is_truthy();
        if done {
            break;
        }
        let value: js_sys::Uint8Array = js_sys::Reflect::get(&res, &JsValue::from_str("value"))
            .unwrap()
            .dyn_into()
            .unwrap();
        let len = value.length() as u64;
        if is_textual {
            let bytes_slice = value.to_vec();
            let part = decoder
                .decode_with_u8_array_and_options(&bytes_slice, &opts)
                .unwrap();
            text.push_str(&part);
        } else {
            binary_chunks.push(&JsValue::from(value));
        }
        bytes += len;
        on_chunk(bytes, text.clone());
    }

    if is_textual {
        // финальный decode()
        opts.set_stream(false);
        let tail = decoder
            .decode_with_u8_array_and_options(&[], &opts)
            .unwrap();
        text.push_str(&tail);
    }

    let time_ms = (js_now() - start).round();
    let mut image_blob_url = None;
    if !is_textual && content_type.starts_with("image/") {
        let initb = BlobPropertyBag::new();
        initb.set_type(if content_type.is_empty() {
            "application/octet-stream"
        } else {
            &content_type
        });
        let blob = Blob::new_with_u8_array_sequence_and_options(&binary_chunks, &initb).unwrap();
        image_blob_url = Some(web_sys::Url::create_object_url_with_blob(&blob).unwrap());
    }

    Ok(StreamOutcome {
        network_error: false,
        status: resp.status() as u16,
        time_ms,
        bytes,
        text: if is_textual { text } else { String::new() },
        image_blob_url,
    })
}
