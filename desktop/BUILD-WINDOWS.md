# Сборка релиза под Windows — проверенный хэппи-пас

Проверено фактом 19.11.2026 на TB49 (192.168.1.85, Xeon, Windows): оба бинарника v1.1.0 собраны офлайн,
десктоп запущен, `GET /api/status` → `{"ok":true,"version":"1.1.0"}`.

Принцип: **сборка только на Windows-машине, интернет на ней не нужен**. Все crates и инструменты
готовит mac (кэш cargo + `cargo vendor`) и привозит архивами.

## 0. Одноразовая подготовка машины (один раз, не в релизный прогон)
- Rust MSVC (rustup, target `x86_64-pc-windows-msvc`) + `link.exe` из VS Build Tools.
- Node LTS (для trunk из crates.io), WebView2 Runtime (в Win10/11 обычно есть).
- Инструменты ставятся офлайн из локального вендора: `cargo install --path vendor/trunk --locked`,
  так же `tauri-cli`, `cargo-xwin`, `cbindgen`. Каталог вендора — `vendor_desktop` (имя — как в config.toml).

## 1. На mac: привезти исходники и вендор
```bash
cd ~/.hermes/skills/bunny-hops/app
cargo vendor --version-interval ">=0.0.0" desktop > vendor-config-desktop.toml
# архивы: исходники (tar), вендор сервера, вендор десктопа — COPYFILE_DISABLE=1 tar czf
scp -i ~/.ssh/id_ed25519 ... user@100.66.85.11:bh_build/
```
`--version-interval ">=0.0.0"` обязателен: по умолчанию cargo vendor берёт только newest-version-per-name
и роняет дубли версий из lock (проверено 19.11.2026: без флага не хватало `home 0.511.3/0.59.2`).

## 2. На Windows: распаковать, конфиг вендора
- `.cargo/config.toml`: `directory = "<точное имя каталога вендора>"` — имя каталога сверить `dir /b /ad`,
  путь из догадки даёт `os error 3` (проверено 19.11.2026).
- Писать конфиг без BOM: `echo ... > file` в cmd даёт UTF-8 без BOM; `Set-Content` PowerShell добавляет BOM
  и `toml` падает `unicode escape sequence error` на строке с кириллицей. Правка BOM — байтовым срезом.

## 3. Серверный бинарник
```bat
cd %USERPROFILE%\bh_build\bh-src
cargo build --release --no-default-features --bin bunny-hops-server   # ~10 мин, target\release\
```

## 4. Фронтенд
```bat
cd %USERPROFILE%\bh_build\bh-src\frontend
trunk build --release
```
`dist` обязан лежать в `bh-src\frontend\dist` — это относительный путь `../frontend/dist` из
`desktop/tauri.conf.json`. До его появления `tauri build` падает `build.rs: The "dist" path does not exist`.

## 5. Десктопный бинарник (tauri-cli на машине НЕ нужен)
`tauri build` без бандла = компиляция крейта + раскладка ресурсов руками:
```bat
cd %USERPROFILE%\bh_build\bh-src\desktop
set CARGO_TARGET_DIR=E:\bh\target-desktop
set BH_VENDOR=E:\bh\vendor-desktop
cargo build --release --bin bunny-hops-desktop
```
Затем разложить dist рядом с exe в `_up_\frontend\dist` (путь `../` из tauri.conf сохраняется как `_up_`):
```bat
xcopy /e /y %USERPROFILE%\bh_build\bh-src\frontend\dist E:\bh\target-desktop\release\_up_\frontend\dist\
```
Первая полная компиляция графа (tao/wry/webview2-com) — 10–15 минут на Xeon: приложение маленькое,
тяжёлое — граф WebView. Повторные сборки — минуты (кэш target-desktop, проверено: 3 м 17 с).
Артефакт: `E:\bh\target-desktop\release\bunny-hops-desktop.exe` (~14 МБ).
Консоль при запуске не нужна (правило пользователя 19.11.2026): в `desktop/src/main.rs` стоит
`#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` — релиз оконной подсистемы,
dev-сборка консоль остаётся.

## 6. Smoke-запуск и приёмка
Порт десктопа всегда эфемерный (bind(0)) — проверять НЕ 3000.
Окно на рабочем столе даёт только интерактивная сессия: обычный старт из-под ssh умирает с сессией,
CIM Create даёт сессию 0 без окна. Рабочий путь — задача планировщика (создаётся один раз):
```bat
schtasks /create /tn BunnyHopsTest /tr "E:\bh\target-desktop\release\bunny-hops-desktop.exe" /sc once /st 23:59 /f
schtasks /run /tn BunnyHopsTest
```
Порт — по PID процесса (action_log пишется в cwd запуска и может быть старым):
```bat
powershell "$p=Get-CimInstance Win32_Process -Filter \"Name='bunny-hops-desktop.exe'\"; $port=(Get-NetTCPConnection -OwningProcess $p.ProcessId -State Listen).LocalPort; (New-Object Net.WebClient).DownloadString('http://127.0.0.1:'+$port+'/api/status')"
```
Гейты приёмки: `SessionId=1` (окно видно пользователю) + LISTENING-сокет процесса + `/api/status` `ok:true` + версия.

## Ямы (все наступали 19.11.2026)
1. BOM от PowerShell ломает парсинг Cargo.toml/config.toml — писать без BOM, править байтами.
2. Имя каталога вендора не угадывать — брать из `dir /b /ad`.
3. `dist` не там, где ждёт tauri.conf — класть в `bh-src\frontend\dist` до `tauri build`.
4. Порт не 3000 — читать из `action_log.json` или title-bar окна.
5. Процесс, запущенный из ssh-сессии, умирает вместе с ней. CIM Create даёт сессию 0 — окно не видно.
   Для окна на рабочем столе TB49: `schtasks /create /tn ... /tr <exe> /sc once /st <hh:mm> /f` + `/run` → SessionId=1 (проверено 19.11.2026).
6. cargo vendor без `--version-interval ">=0.0.0"` даёт неполный вендор.
7. `action_log.json` пишется в каталог ЗАПУСКА (cwd задачи), а не рядом с бинарём, и запись там может быть
   старой. Живой источник порта: `netstat -ano | findstr <PID>` → строка LISTENING 127.0.0.1:<порт>
   (плюс ESTABLISHED от webview = окно рендерится). Проверено 19.11.2026.
