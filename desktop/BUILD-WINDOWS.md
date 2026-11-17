# Сборка под Windows (WSL или нативно)

Tauri под Windows собирается **на Windows** (нужны WebView2 + MSVC link).
Целевая машина в контуре Александра — сервер 192.168.1.85 (x86_64-pc-windows-msvc).

## Шаги на Windows-машине

1. Репозиторий скилла: `~/.hermes/skills/bunny-hops/app` (скопировать/склонировать).
2. Инструменты (однократно):
   - Rust MSVC: https://rustup.rs (target `x86_64-pc-windows-msvc`)
   - `cargo install tauri-cli --version "^2" --locked`
   - Trunk: `cargo install trunk` + `rustup target add wasm32-unknown-unknown`
   - WebView2 Runtime (обычно уже есть в Win10/11; иначе — Evergreen installer от Microsoft)
3. Сборка:
   ```powershell
   cd app\frontend; trunk build --release
   cd ..\desktop;  cargo tauri build
   ```
4. Артефакт: `desktop\target\release\bundle\msi\Bunny Hops_0.1.0_x64_en-US.msi`
   (или `bundle\nsis\*.exe`, если добавить feature nsis в tauri.conf).

## Проверка

Запустить собранный exe: окно 1280×800, вкладка Mock, `http://127.0.0.1:<порт>` внутри окна.
Headless-режим не затронут: `bunny-hops-server.exe` собирается как раньше (`cargo build --release` в app/).
