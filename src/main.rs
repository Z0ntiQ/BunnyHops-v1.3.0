// Bunny Hops — тонкий headless-бинарь (API-движок: /debug/*, минимальный API /api/status +
// /api/debug-mode); env-контракт BUNNY_HOPS_DIST/PORT; UI — только окно Tauri (app/desktop/),
// браузер не открывается.
// Логика сервера — в lib.rs (run_server), оттуда же её берёт Tauri-оболочка desktop/.
use bunny_hops_server::run_server;
use std::env;
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    // dist рядом с бинарником; переопределение — BUNNY_HOPS_DIST (старый HOP_LITE_DIST тоже принимается)
    let dist = match env::var("BUNNY_HOPS_DIST").or_else(|_| env::var("HOP_LITE_DIST")) {
        Ok(d) => PathBuf::from(d),
        Err(_) => {
            let exe = env::current_exe().unwrap_or_default();
            exe.parent()
                .map(|p| p.join("dist"))
                .unwrap_or_else(|| PathBuf::from("dist"))
        }
    };
    // PORT не задан — порт рандомизируется ядром; задан — строгий bind на него.
    let fixed_port: Option<u16> = env::var("PORT").ok().and_then(|p| p.parse().ok());

    match run_server(dist, fixed_port, None).await {
        Ok(_port) => {
            // Сервер живёт на spawned-задаче; держим процесс.
            std::future::pending::<()>().await;
        }
        Err(e) => {
            eprintln!("Ошибка: {e}");
            std::process::exit(1);
        }
    }
}
