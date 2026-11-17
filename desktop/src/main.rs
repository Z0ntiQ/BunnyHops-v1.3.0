// Bunny Hops — оконная оболочка (Tauri v2).
// Один процесс: в setup поднимается то же ядро (bunny_hops_server::run_server) на отдельном
// потоке со своим tokio-runtime, затем окно открывается на http://127.0.0.1:<занятый порт>.
// Дистрибутив SPA: BUNNY_HOPS_DIST → ресурсы собранного приложения → ../frontend/dist (dev).
use bunny_hops_server::run_server;
use std::path::PathBuf;
use std::sync::mpsc;
use tauri::Manager;

fn resolve_dist(app: &tauri::App) -> PathBuf {
    // 1. явное переопределение
    if let Ok(d) = std::env::var("BUNNY_HOPS_DIST") {
        let p = PathBuf::from(d);
        if p.join("index.html").is_file() {
            return p;
        }
    }
    // 2. собранные ресурсы (.app/Contents/Resources/...)
    if let Ok(res) = app.path().resolve("dist", tauri::path::BaseDirectory::Resource) {
        if res.join("index.html").is_file() {
            return res;
        }
    }
    // 3. dev-режим: cargo tauri dev запускается из desktop/, dist — рядом
    PathBuf::from("../frontend/dist")
}

fn main() {
    tauri::Builder::default()
        // Обещание баннера: закрытие окна останавливает сервер и процесс
        // (дефолт Tauri v2 — Wait: процесс висит без окна).
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                if window.webview_windows().is_empty() {
                    window.app_handle().exit(0);
                }
            }
        })
        .setup(move |app| {
            let dist = resolve_dist(app);

            let (tx, rx) = mpsc::channel::<Result<u16, String>>();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let res = rt.block_on(run_server(dist, None)); // порт всегда рандомизируется (правило пользователя 17.11.2026)
                let _ = tx.send(res);
                // держим runtime живым: serve-задача крутится на нём;
                // поток умрёт вместе с процессом (закрытие окна)
                loop {
                    std::thread::park();
                }
            });

            let port = match rx.recv().expect("сервер не ответил") {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Ошибка запуска сервера: {e}");
                    std::process::exit(1);
                }
            };

            let url = format!("http://127.0.0.1:{port}");
            let _win = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::External(url.parse().unwrap()),
            )
            .title(format!("Bunny Hops \u{2014} http://127.0.0.1:{port}")) // полный адрес в title-bar (правило пользователя 17.11.2026)
            .inner_size(1280.0, 800.0)
            .min_inner_size(900.0, 600.0)
            .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("ошибка запуска Bunny Hops");
}
