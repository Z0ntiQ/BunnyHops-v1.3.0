// /screenshot — склеенный PNG вкладки Bunny Hops: хедеры → запрос → ответ.
// Ищет вкладку приложения через CDP (DevTools-порт Chrome), кликает табы
// настоящими мышиными событиями (Input.dispatchMouseEvent — Leptos не видит
// синтетические JS-клики), снимает три области через Page.captureScreenshot
// и склеивает вертикально. Активный таб восстанавливается после съёмки.
//
// Порт CDP: HOP_CDP_PORT, иначе ~/.hermes/browser-profile/chrome/DevToolsActivePort,
// иначе 9222. Целевая вкладка: HOP_TARGET (подстрока URL), иначе "localhost:3000".

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

struct Cdp {
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
}

impl Cdp {
    async fn send(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.id += 1;
        let msg = json!({ "id": self.id, "method": method, "params": params });
        self.ws
            .send(Message::Text(msg.to_string()))
            .await
            .map_err(|e| format!("ws send: {e}"))?;
        loop {
            match self.ws.next().await {
                Some(Ok(Message::Text(t))) => {
                    let v: Value = serde_json::from_str(&t).map_err(|e| e.to_string())?;
                    if v.get("id").and_then(Value::as_u64) == Some(self.id) {
                        if let Some(err) = v.get("error") {
                            return Err(format!("cdp {}: {err}", method));
                        }
                        return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                    }
                    // события игнорируем
                }
                Some(Ok(Message::Close(_))) | None => return Err("cdp closed".into()),
                Some(Err(e)) => return Err(format!("ws: {e}")),
                Some(Ok(_)) => {}
            }
        }
    }

    /// Runtime.evaluate с returnByValue; ожидаем строку с JSON.
    async fn eval_json(&mut self, expr: &str) -> Result<Value, String> {
        let r = self
            .send(
                "Runtime.evaluate",
                json!({"expression": expr, "returnByValue": true}),
            )
            .await?;
        let s = r
            .get("result")
            .and_then(|o| o.get("value"))
            .and_then(Value::as_str)
            .ok_or_else(|| format!("eval no string: {r}"))?;
        serde_json::from_str(s).map_err(|e| format!("eval json: {e}"))
    }

    async fn click(&mut self, x: f64, y: f64) -> Result<(), String> {
        for kind in ["mousePressed", "mouseReleased"] {
            self.send(
                "Input.dispatchMouseEvent",
                json!({"type": kind, "x": x, "y": y, "button": "left", "clickCount": 1}),
            )
            .await?;
        }
        Ok(())
    }

    async fn shot(&mut self, rect: &Rect) -> Result<Vec<u8>, String> {
        let r = self
            .send(
                "Page.captureScreenshot",
                json!({"format": "png", "clip": {
                    "x": rect.x, "y": rect.y, "width": rect.w, "height": rect.h, "scale": 1.0
                }}),
            )
            .await?;
        let b64 = r
            .get("data")
            .and_then(Value::as_str)
            .ok_or("no screenshot data")?;
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| e.to_string())
    }
}

#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

fn rect_from(v: &Value) -> Option<Rect> {
    Some(Rect {
        x: v.get("x")?.as_f64()?,
        y: v.get("y")?.as_f64()?,
        w: v.get("w")?.as_f64()?,
        h: v.get("h")?.as_f64()?,
    })
}

fn cdp_candidates() -> Vec<u16> {
    let mut ports = Vec::new();
    if let Ok(v) = std::env::var("HOP_CDP_PORT") {
        if let Ok(p) = v.trim().parse::<u16>() {
            ports.push(p);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let f = std::path::Path::new(&home)
            .join(".hermes/browser-profile/chrome/DevToolsActivePort");
        if let Ok(s) = std::fs::read_to_string(&f) {
            if let Some(p) = s.lines().next().and_then(|l| l.trim().parse::<u16>().ok()) {
                ports.push(p);
            }
        }
    }
    ports.push(9222);
    ports.dedup();
    ports
}

/// Собираем геометрию страницы: urlbar, левая/правая панели, табы левой панели.
const GEOMETRY_JS: &str = r#"JSON.stringify((()=>{
  const box=(e)=>{if(!e)return null;const b=e.getBoundingClientRect();
    return {x:b.x,y:b.y,w:b.width,h:b.height};};
  const panes=[...document.querySelectorAll('main.panes > section.pane')];
  const left=panes[0], right=panes[1];
  const tabs=[...(left?left.querySelectorAll('nav.tabs .tab'):[])].map(e=>{
    const b=e.getBoundingClientRect();
    return {text:e.textContent.trim(), cx:b.x+b.width/2, cy:b.y+b.height/2,
            active:e.classList.contains('active')};});
  return {urlbar:box(document.querySelector('.urlbar')),
          left:box(left), right:box(right), tabs,
          activeTab:(left&&left.querySelector('nav.tabs .tab.active'))
            ?left.querySelector('nav.tabs .tab.active').textContent.trim():null};
})())"#;

fn find_tab<'a>(tabs: &'a [Value], needle: &str) -> Option<&'a Value> {
    tabs.iter().find(|t| {
        t.get("text")
            .and_then(Value::as_str)
            .map(|s| s.to_lowercase().contains(needle))
            .unwrap_or(false)
    })
}

/// Склеить PNG по вертикали: headers, request, response (тёмный зазор между).
fn stitch(imgs: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    let mut decoded = Vec::new();
    for b in imgs {
        decoded.push(
            image::load_from_memory(b)
                .map_err(|e| format!("decode: {e}"))?
                .to_rgba8(),
        );
    }
    let w: u32 = decoded.iter().map(|i| i.width()).max().unwrap();
    let gap: u32 = 12;
    let h: u32 = decoded.iter().map(|i| i.height()).sum::<u32>() + gap * (decoded.len() as u32 - 1);
    let mut out = image::RgbaImage::from_pixel(w, h, image::Rgba([15, 17, 21, 255]));
    let mut y = 0u32;
    for img in &decoded {
        image::imageops::overlay(&mut out, img, 0, y as i64);
        y += img.height() + gap;
    }
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(out)
        .write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| format!("encode: {e}"))?;
    Ok(buf.into_inner())
}

pub async fn screenshot_handler(Query(q): Query<HashMap<String, String>>) -> Response {
    let target = std::env::var("HOP_TARGET").unwrap_or_else(|_| "localhost:3000".into());
    let target = q.get("match").cloned().unwrap_or(target);
    let run = async {
        // 1. Найти CDP-порт с подходящей вкладкой.
        let mut ws_url: Option<String> = None;
        for port in cdp_candidates() {
            let Ok(resp) = reqwest::get(format!("http://127.0.0.1:{port}/json/list")).await else {
                continue;
            };
            let Ok(list) = resp.json::<Value>().await else {
                continue;
            };
            if let Some(t) = list.as_array().into_iter().flatten().find(|t| {
                t.get("type").and_then(Value::as_str) == Some("page")
                    && t.get("url")
                        .and_then(Value::as_str)
                        .map(|u| u.contains(&target))
                        .unwrap_or(false)
            }) {
                ws_url = t.get("webSocketDebuggerUrl").and_then(Value::as_str).map(String::from);
                if ws_url.is_some() {
                    break;
                }
            }
        }
        let ws_url = ws_url.ok_or("вкладка Bunny Hops не найдена (открой http://localhost:3000)")?;
        let (ws, _) = connect_async(&ws_url).await.map_err(|e| format!("ws: {e}"))?;
        let mut cdp = Cdp { ws, id: 0 };
        cdp.send("Page.enable", json!({})).await?;
        cdp.send("Page.bringToFront", json!({})).await?;

        // 2. Геометрия и активный таб.
        let geo = cdp.eval_json(GEOMETRY_JS).await?;
        let left = rect_from(geo.get("left").ok_or("no left pane")?)
            .ok_or("left pane not visible")?;
        let right = rect_from(geo.get("right").ok_or("no right pane")?)
            .ok_or("right pane not visible")?;
        let urlbar = geo.get("urlbar").and_then(rect_from);
        let tabs: Vec<Value> = geo
            .get("tabs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let restore = geo.get("activeTab").and_then(Value::as_str).map(String::from);

        let click_tab = |_cdp: &mut Cdp, name: &str| -> Option<(f64, f64)> {
            find_tab(&tabs, name).map(|t| {
                (
                    t.get("cx").and_then(Value::as_f64).unwrap(),
                    t.get("cy").and_then(Value::as_f64).unwrap(),
                )
            })
        };

        // 3. Хедеры: активировать таб Headers, снять левую панель.
        let mut shots: Vec<Vec<u8>> = Vec::new();
        if let Some((x, y)) = click_tab(&mut cdp, "head") {
            cdp.click(x, y).await?;
            tokio::time::sleep(std::time::Duration::from_millis(450)).await;
        }
        shots.push(cdp.shot(&left).await?);

        // 4. Запрос: таб Params (параметры) + urlbar+левая панель одним кадром.
        if let Some((x, y)) = click_tab(&mut cdp, "param") {
            cdp.click(x, y).await?;
            tokio::time::sleep(std::time::Duration::from_millis(450)).await;
        }
        let req_rect = match urlbar {
            Some(u) => Rect {
                x: left.x.min(u.x),
                y: u.y,
                w: left.w.max(u.w),
                h: (left.y + left.h) - u.y,
            },
            None => left,
        };
        shots.push(cdp.shot(&req_rect).await?);

        // 5. Вернуть исходный таб, снять правую панель (ответ).
        if let Some(name) = restore {
            if let Some((x, y)) = click_tab(&mut cdp, &name.to_lowercase()) {
                cdp.click(x, y).await.ok();
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
        }
        shots.push(cdp.shot(&right).await?);
        stitch(&shots)
    };
    match tokio::time::timeout(std::time::Duration::from_secs(25), run).await {
        Ok(Ok(png)) => (
            StatusCode::OK,
            [("content-type", "image/png")],
            png,
        )
            .into_response(),
        Ok(Err(e)) => (
            StatusCode::BAD_GATEWAY,
            [(
                "content-type",
                "application/json; charset=utf-8",
            )],
            json!({"error": e}).to_string(),
        )
            .into_response(),
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            [(
                "content-type",
                "application/json; charset=utf-8",
            )],
            json!({"error": "cdp timeout"}).to_string(),
        )
            .into_response(),
    }
}
