# Твоя задача (d2_server)

Напиши файл `src/mock_server.rs` полностью, по «Контракту B» из SPEC.
- Синтаксис axum 0.7 и reqwest бери дословно из фрагментов `main_head.txt` и `main_router.txt` (обработчики `async fn(req: Request) -> Response`, глобальные `static` через `OnceLock<Mutex<..>>`, hop-by-hop список).
- Рандомизация: `crate::mock_random::randomize_text(&str) -> String` (уже существует, просто вызывай).
- Никаких `mod` объявлений и `fn main` внутри файла.
- Ниже приложены фрагменты эталонного кода.

=== main_head.txt ===
// Hop Lite — Rust-порт server.js + proxy-middleware.js, один в один по контракту.
// Раздаёт SPA из dist/ и обрабатывает /proxy (стриминговый обход CORS).
// Пути резолвятся от папки бинарника (или HOP_LITE_DIST), автоподбор порта,
// автооткрытие браузера (отключается HOP_NO_BROWSER=1).

use axum::body::{to_bytes, Body};
use axum::extract::Request;
use axum::http::{header::HeaderName, HeaderValue, Method, StatusCode};
use axum::response::Response;
use axum::routing::{any, get};
use axum::Router;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::env;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
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


=== main_router.txt ===
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    Command::new("open").arg(url).spawn().ok();
    #[cfg(target_os = "windows")]
    Command::new("cmd").args(["/C", "start", "", url]).spawn().ok();
    #[cfg(all(unix, not(target_os = "macos")))]
    Command::new("xdg-open").arg(url).spawn().ok();
}

mod screenshot;
use screenshot::screenshot_handler;

#[tokio::main]
async fn main() {
    // dist рядом с бинарником; переопределение — BUNNY_HOPS_DIST (старый HOP_LITE_DIST тоже принимается)
    let dist = match env::var("BUNNY_HOPS_DIST").or_else(|_| env::var("HOP_LITE_DIST")) {
        Ok(d) => PathBuf::from(d),
        Err(_) => {
            let exe = env::current_exe().unwrap_or_default();
            exe.parent().map(|p| p.join("dist")).unwrap_or_else(|| PathBuf::from("dist"))
        }
    };
    if !dist.join("index.html").is_file() {
        eprintln!("Ошибка: не найдена папка dist/ рядом с сервером. Сборка отсутствует.");
        std::process::exit(1);
    }

    let start_port: u16 = env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3000);

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        // Как браузер: висящее соединение не должно держать лоадер вечно.
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap();

    let app = Router::new()
        .route("/proxy", any(proxy_handler))
        .route("/screenshot", any(screenshot_handler))
        .route("/api/automation-rules", get(api_rules_handler))
        .fallback(move |req: Request| {
            let root = dist.clone();
            async move { serve_static_or_spa(&root, req).await }
        })
        .with_state(AppState {
            client,
            api_log: Arc::new(Mutex::new(Vec::new())),
        });

    // автоподбор порта, как listen(port+1) в оригинале (до 10 попыток)
    let mut port = start_port;
    let listener = loop {
        match tokio::net::TcpListener::bind(("::", port)).await {
            Ok(l) => break l,
            Err(_) if port < 65535 => port += 1,
            Err(e) => {
                eprintln!("Не удалось занять порт (последняя ошибка: {e}).");
                std::process::exit(1);
            }
        }
    };

    let url = format!("http://localhost:{port}");
    println!("==========================================");
    println!("  Bunny Hops запущен! (Rust)");
    println!("  Адрес: {url}");
    println!("  Закройте это окно, чтобы остановить сервер.");
    println!("==========================================");
    if env::var("HOP_NO_BROWSER").is_err() {
        open_browser(&url);
    }

    axum::serve(listener, app).await.unwrap();
}
