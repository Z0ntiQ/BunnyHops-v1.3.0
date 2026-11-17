// Hop Lite — Rust-порт server.js + proxy-middleware.js, один в один по контракту.
// Раздаёт SPA из dist/ и обрабатывает /proxy (стриминговый обход CORS).
// Пути резолвятся от папки бинарника (или HOP_LITE_DIST), автоподбор порта.
// UI — только окно Tauri (app/desktop/); автооткрытия браузера нет (веб-версия удалена 17.11.2026).

use axum::body::{to_bytes, Body};
use axum::extract::Request;
use axum::http::{header::HeaderName, HeaderValue, Method, StatusCode};
use axum::response::Response;
use axum::routing::{any, get};
use axum::Router;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const HOP_BY_HOP: &[&str] = &[
    "host",
    "connection",
    "keep-alive",
    "transfer-encoding",
    "upgrade",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "expect",
];

// Эти нельзя копировать как есть: тело приходит уже распакованным,
// а браузер попытался бы распаковать его ещё раз по content-encoding.
const STRIP: &[&str] = &["content-length", "content-encoding", "transfer-encoding"];
/// Максимальный размер тела входящего запроса в /proxy (32 MiB).
const MAX_PROXY_BODY: usize = 32 * 1024 * 1024;

fn mime_for(ext: &str) -> &'static str {
    match ext {
        ".html" => "text/html; charset=utf-8",
        ".js" => "text/javascript",
        ".css" => "text/css",
        ".svg" => "image/svg+xml",
        ".png" => "image/png",
        ".jpg" => "image/jpeg",
        ".gif" => "image/gif",
        ".webp" => "image/webp",
        ".json" => "application/json",
        ".ico" => "image/x-icon",
        ".txt" => "text/plain; charset=utf-8",
        ".map" => "application/json",
        ".wasm" => "application/wasm",
        ".woff" => "font/woff",
        ".woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn is_skipped(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    HOP_BY_HOP.contains(&n.as_str()) || STRIP.contains(&n.as_str())
}

/// Разрешает путь запроса внутри dist/ с защитой от выхода за root
/// (аналог join+normalize+startsWith из server.js).
fn resolve_static(root: &Path, raw_path: &str) -> Option<PathBuf> {
    let decoded = percent_encoding::percent_decode_str(raw_path).decode_utf8_lossy().into_owned();
    let clean = if decoded == "/" { "/index.html".to_string() } else { decoded };
    let mut file = root.to_path_buf();
    for c in Path::new(&clean).components() {
        match c {
            Component::Normal(seg) => file.push(seg),
            Component::ParentDir => {
                file.pop();
            }
            Component::RootDir | Component::CurDir => {}
            _ => return None,
        }
    }
    if !file.starts_with(root) {
        return None;
    }
    Some(file)
}

async fn serve_static_or_spa(root: &Path, req: Request) -> Response {
    let raw_path = req.uri().path();
    let mut file = match resolve_static(root, raw_path) {
        Some(f) => f,
        None => return spa_fallback(root).await,
    };

    // директория -> index.html, как в оригинале
    if file.is_dir() {
        file = file.join("index.html");
    }
    match tokio::fs::read(&file).await {
        Ok(data) => {
            let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
            let ct = mime_for(&format!(".{ext}"));
            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", ct)
                .body(Body::from(data))
                .unwrap()
        }
        Err(_) => spa_fallback(root).await, // SPA fallback
    }
}

async fn spa_fallback(root: &Path) -> Response {
    match tokio::fs::read(root.join("index.html")).await {
        Ok(data) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/html; charset=utf-8")
            .body(Body::from(data))
            .unwrap(),
        Err(_) => Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("Server error"))
            .unwrap(),
    }
}

#[derive(Clone)]
pub struct AppState {
    client: reqwest::Client,
    api_log: Arc<Mutex<Vec<serde_json::Value>>>,
}

/// Вызовы /proxy из внешнего API (без заголовка x-hop-ui) складываем в лог
/// для технической вкладки automation_rules.
fn log_api_call(
    state: &AppState,
    method: &Method,
    target: &str,
    supplied: &HashMap<String, serde_json::Value>,
    body: &[u8],
    status: u16,
) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let headers: Vec<serde_json::Value> = supplied
        .iter()
        .map(|(k, v)| {
            serde_json::json!({
                "key": k,
                "value": v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()),
                "on": true,
            })
        })
        .collect();
    let rec = serde_json::json!({
        "id": format!("a{}", now.as_nanos()),
        "method": method.as_str(),
        "url": target,
        "headers": headers,
        "body": String::from_utf8_lossy(body),
        "status": status,
        "ts": now.as_secs(),
    });
    let mut log = state.api_log.lock().unwrap();
    log.push(rec);
    if log.len() > 50 {
        let excess = log.len() - 50;
        log.drain(0..excess);
    }
}

async fn api_rules_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Response {
    let log = state.api_log.lock().map(|l| l.clone()).unwrap_or_default();
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json; charset=utf-8")
        .header("Cache-Control", "no-store")
        .body(Body::from(serde_json::to_vec(&log).unwrap_or_default()))
        .unwrap()
}

/// Стриминговый прокси: тело ответа передаётся по мере поступления.
/// Заголовки цели копируются 1:1 (без hop-by-hop), ставится X-Proxy-Final-Url.
/// Ошибка подключения -> 502 + X-Proxy-Error: connection-refused.
async fn proxy_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: Request,
) -> Response {
    let client = state.client.clone();
    let target = req
        .uri()
        .query()
        .and_then(|q| url_query_param(q, "url"));

    let mut builder = Response::builder()
        .header("Cache-Control", "no-store")
        .header("X-Accel-Buffering", "no");

    let target = match target {
        Some(t) => t,
        None => {
            return builder
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json; charset=utf-8")
                .body(Body::from(r#"{"error":"Missing ?url= parameter"}"#))
                .unwrap();
        }
    };

    // валидация URL до запроса: невалидный — это 400, а не «подключение не удалось»
    if let Err(e) = reqwest::Url::parse(&target) {
        return builder
            .status(StatusCode::BAD_REQUEST)
            .header("Content-Type", "text/plain; charset=utf-8")
            .header("X-Proxy-Error", "invalid-url")
            .body(Body::from(format!("INVALID_URL — {e}")))
            .unwrap();
    }

    let method = req.method().clone();
    let supplied: HashMap<String, serde_json::Value> = req
        .headers()
        .get("x-proxy-headers")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    let from_ui = req.headers().get("x-hop-ui").is_some();
    // Лимит тела: перенос larger-than-memory ответа через прокси не должен ронять процесс.
    let body_bytes = match to_bytes(req.into_body(), MAX_PROXY_BODY).await {
        Ok(b) => b,
        Err(_) => {
            return builder
                .status(StatusCode::PAYLOAD_TOO_LARGE)
                .header("Content-Type", "application/json; charset=utf-8")
                .body(Body::from(format!(
                    r#"{{"error":"body exceeds {} bytes"}}"#,
                    MAX_PROXY_BODY
                )))
                .unwrap();
        }
    };
    let body_log = body_bytes.clone();

    let mut rb = client.request(method.clone(), &target);
    for (k, v) in &supplied {
        let lk = k.to_ascii_lowercase();
        if HOP_BY_HOP.contains(&lk.as_str()) || lk == "content-length" {
            continue;
        }
        if let (Ok(hn), Ok(hv)) = (
            HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v.as_str().unwrap_or(&v.to_string())),
        ) {
            rb = rb.header(hn, hv);
        }
    }
    if !body_bytes.is_empty() && method != Method::GET && method != Method::HEAD {
        rb = rb.body(body_bytes);
    }

    match rb.send().await {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
            if !from_ui {
                log_api_call(&state, &method, &target, &supplied, &body_log, status.as_u16());
            }
            control::log_action("request", "proxy", serde_json::json!({
                "method": method.as_str(),
                "url": &target,
                "status": status.as_u16(),
                "from_ui": from_ui,
            }));
            let final_url = resp.url().to_string();
            let mut headers: Vec<(HeaderName, HeaderValue)> = Vec::new();
            for (k, v) in resp.headers().iter() {
                if !is_skipped(k.as_str()) {
                    headers.push((k.clone(), v.clone()));
                }
            }
            let mut b = builder
                .status(status)
                .header("X-Proxy-Final-Url", final_url.as_str());
            for (k, v) in headers {
                b = b.header(k, v);
            }
            let stream = resp.bytes_stream().map(|item| {
                item.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })
            });
            b.body(Body::from_stream(stream)).unwrap()
        }
        Err(e) => {
            eprintln!("proxy error: {e:?}");
            if !from_ui {
                log_api_call(&state, &method, &target, &supplied, &body_log, 502);
            }
            control::log_action("request", "proxy", serde_json::json!({
                "method": method.as_str(),
                "url": &target,
                "status": 502,
                "from_ui": from_ui,
                "error": format!("{e}"),
            }));
            let host = target
                .parse::<reqwest::Url>()
                .map(|u| u.host_str().unwrap_or(&target).to_string())
                .unwrap_or_else(|_| target.clone());
            let (kind, verb) = if e.is_timeout() {
                ("timeout", "превышен таймаут запроса к")
            } else if short_error(&e) == "ENOTFOUND" {
                ("dns-not-found", "не найдено имя хоста")
            } else {
                ("connection-refused", "не удалось подключиться к")
            };
            let msg = format!("{} — {} {}", short_error(&e), verb, host);
            builder
                .status(StatusCode::BAD_GATEWAY)
                .header("X-Proxy-Error", kind)
                .header("Content-Type", "text/plain; charset=utf-8")
                .body(Body::from(msg))
                .unwrap()
        }
    }
}

fn short_error(e: &reqwest::Error) -> String {
    // Приближение node-овского e.cause.code (ECONNREFUSED/ENOTFOUND):
    if e.is_timeout() {
        "ETIMEDOUT".to_string()
    } else if e.is_connect() {
        // DNS-ошибка у reqwest тоже попадает в is_connect — различаем по цепочке
        // источников: в Display её нет, только в Debug ("dns error: ...").
        if format!("{e:?}").to_ascii_lowercase().contains("dns error") {
            "ENOTFOUND".to_string()
        } else {
            "ECONNREFUSED".to_string()
        }
    } else if e.is_builder() {
        "INVALID_URL".to_string()
    } else {
        e.to_string()
    }
}

fn url_query_param(q: &str, key: &str) -> Option<String> {
    for pair in q.split('&') {
        let mut it = pair.splitn(2, '=');
        if it.next() == Some(key) {
            let v = it.next().unwrap_or("");
            return Some(
                percent_encoding::percent_decode_str(v)
                    .decode_utf8_lossy()
                    .into_owned(),
            );
        }
    }
    None
}

pub mod screenshot;
pub mod eyes;
pub mod control;
pub mod mock_server;
pub mod mock_random;
use screenshot::screenshot_handler;
use eyes::eyes_handler;

/// Запускает сервер; возвращает занявший порт.
/// `fixed_port: None` — порт всегда рандомизируется (ОС выдаёт свободный, правило пользователя 17.11.2026);
/// `Some(p)` — строгий bind на p (явный запрос через env PORT), ошибка если занят.
pub async fn run_server(dist: PathBuf, fixed_port: Option<u16>) -> Result<u16, String> {
    if !dist.join("index.html").is_file() {
        return Err("не найдена папка dist/ рядом с сервером; сборка отсутствует".to_string());
    }

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        // Как браузер: висящее соединение не должно держать лоадер вечно.
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap();

    control::STARTED.set(std::time::Instant::now()).ok();
    let app = Router::new()
        .route("/proxy", any(proxy_handler))
        .route("/screenshot", any(screenshot_handler))
        .route("/api/eyes", get(eyes_handler))
        .route("/api/automation-rules", get(api_rules_handler))
        .merge(mock_server::control_router())
        .merge(control::control_router())
        .fallback(move |req: Request| {
            let root = dist.clone();
            async move { serve_static_or_spa(&root, req).await }
        })
        .with_state(AppState {
            client,
            api_log: Arc::new(Mutex::new(Vec::new())),
        });

    // Порт: по умолчанию рандомизация на каждом старте; строгий bind только при явном PORT.
    // Bind только на loopback: /proxy — открытый прокси, наружу (LAN) торчать нечем (правило ревью 17.11.2026).
    let listener = match fixed_port {
        Some(p) => tokio::net::TcpListener::bind(("127.0.0.1", p))
            .await
            .map_err(|e| format!("порт {p} занят или недоступен: {e}"))?,
        None => tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| format!("не удалось занять порт: {e}"))?,
    };
    let port = listener
        .local_addr()
        .map_err(|e| format!("не удалось прочитать адрес: {e}"))?
        .port();

    let url = format!("http://localhost:{port}");
    println!("==========================================");
    println!("  Bunny Hops запущен! (Rust)");
    println!("  Адрес: {url}");
    println!("  Закройте это окно, чтобы остановить сервер.");
    println!("==========================================");

    // serve живёт на spawned-задаче: run_server возвращается портом, сервер крутится.
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Ok(port)
}