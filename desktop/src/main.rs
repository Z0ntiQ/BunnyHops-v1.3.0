// Bunny Hops — оконная оболочка (Tauri v2).
// Один процесс: в setup поднимается то же ядро (bunny_hops_server::run_server) на отдельном
// потоке со своим tokio-runtime, затем окно открывается на http://127.0.0.1:<занятый порт>.
// Дистрибутив SPA: BUNNY_HOPS_DIST → ресурсы собранного приложения → ../frontend/dist (dev).
// Релиз под Windows — оконная подсистема: консольное окно при запуске не нужно (правило пользователя 19.11.2026).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
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
    // 2. собранные ресурсы: "frontend/dist" кладётся в .app как
    //    Contents/Resources/_up_/frontend/dist — резолвим и короткое имя, и
    //    фактический путь с _up_ (иначе запуск из Finder/cwd≠build-дерева падает).
    for rel in ["dist", "_up_/frontend/dist"] {
        if let Ok(res) = app.path().resolve(rel, tauri::path::BaseDirectory::Resource) {
            if res.join("index.html").is_file() {
                return res;
            }
        }
    }
    // 3. рядом с бинарём внутри бандла: .../Contents/MacOS → .../Contents/Resources/_up_/frontend/dist
    if let Ok(exe) = std::env::current_exe() {
        if let Some(res) = exe.ancestors().nth(2).map(|p| p.join("Resources/_up_/frontend/dist")) {
            if res.join("index.html").is_file() {
                return res;
            }
        }
    }
    // 4. dev-режим: cargo tauri dev запускается из desktop/, dist — рядом
    PathBuf::from("../frontend/dist")
}

fn main() {
    // Создаём oneshot-канал для активации окна (показать/восстановить/сфокусировать).
    let (window_activate_tx, window_activate_rx) = tokio::sync::oneshot::channel::<()>();

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
            // Передаём sender в run_server для активации окна.
            let window_tx = window_activate_tx;
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let res = rt.block_on(run_server(dist, None, Some(window_tx)));
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
            let mut winb = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::External(url.parse().unwrap()),
            )
            .title(format!("Bunny Hops \u{2014} http://127.0.0.1:{port}")) // полный адрес в title-bar (правило пользователя 17.11.2026)
            .inner_size(1280.0, 800.0)
            .min_inner_size(900.0, 600.0);
            // CDP на 9222 для /screenshot: WebView2 слушает remote debugging только
            // когда флаг передан кодом (env WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
            // до процесса не доходит — проверено 18.11.2026 на TB49).
            // Аргументы заменяют дефолтные wry — дефолт повторяем и дополняем.
            #[cfg(target_os = "windows")]
            {
                winb = winb.additional_browser_args(
                    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port=9222",
                );
            }
            let win = winb.build()?;

            // Слушаем команды активации окна и применяем их на главном потоке Tauri.
            let window = win.clone();
            tauri::async_runtime::spawn(async move {
                let _ = window_activate_rx.await;
                // Показать/восстановить/сфокусировать окно.
                let _ = window.show();
                let _ = window.set_focus();
                let _ = window.unminimize();
            });

            Ok(())
        })
        // Последняя запись журнала — на RunEvent::Exit: он приходит на ЛЮБОМ
        // пути выхода (закрытие окна, Cmd+Q, osascript quit). Проверено
        // 17.11.2026: на quit-путь WindowEvent::Destroyed не приходит.
        .build(tauri::generate_context!())
        .expect("ошибка запуска Bunny Hops")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                use std::sync::atomic::{AtomicBool, Ordering};
                static STOP_LOGGED: AtomicBool = AtomicBool::new(false);
                if !STOP_LOGGED.swap(true, Ordering::SeqCst) {
                    bunny_hops_server::control::log_simple("server", "stop");
                }
            }
        });
}
