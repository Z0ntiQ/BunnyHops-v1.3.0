# SPEC — BunnyProx: mock-шлюз внутри Bunny Hops

## Что строим
В существующем приложении Bunny Hops (`/Users/alexandrcherenkov/.hermes/skills/bunny-hops/app`) добавляем режим «Mock» (BunnyProx): встроенный mock-сервер, который слушает указанный порт, перехватывает запросы по правилу (метод + путь) и отдаёт сохранённый JSON из профиля. JSON поддерживает плейсхолдеры рандомизации `{{...}}` — при каждом ответе значения подставляются заново. Профили хранятся на сервере в `~/.bunny_hops/mock/<имя>.json`. UI — новая вкладка «Proxy» в стиле Postman: слева профили, в центре правила/варианты ответа, справа живой трафик.

## Жёсткие правила для всех частей
1. Никаких новых зависимостей. Сервер: только std + axum 0.7 + tokio + reqwest 0.12 (json) + serde/serde_json + futures-util (уже в Cargo.toml). Фронтенд: только leptos 0.7 (csr) + wasm-bindgen + wasm-bindgen-futures + js-sys + web-sys (features в Cargo.toml) + serde/serde_json.
2. Выдай ОДИН fenced-блок ```rust (или ```python / ```css) с ПОЛНЫМ содержимым файла. Без пояснений, без diff, без второго блока.
3. Сигнатуры контракта — дословно, имена файлов — дословно.
4. Комментарии в коде — на русском, короткие (ASD-STE100: одно предложение — одна мысль).
5. Стиль — как в приложенных фрагментах существующего кода (он эталон API).
6. Код должен компилироваться: не выдумывай методы, которых нет во фрагментах и в стандартных библиотеках.

## Контракт A — `src/mock_random.rs` (crate сервера, edition 2024)
```rust
/// Заменяет все токены {{...}} в тексте. Неизвестный токен остаётся как есть.
pub fn randomize_text(src: &str) -> String
```
- Токен: `{{` ... `}}`, без вложенности, регистронезависимое имя, пробелы внутри скобок допустимы.
- Грамматика токенов (строго этот набор):
  - `{{int:MIN:MAX}}` — целое включительно в диапазоне; MIN>MAX → поменять местами
  - `{{float:MIN:MAX:DEC}}` — float с DEC знаками после точки (0..9)
  - `{{bool}}` → `true`/`false`
  - `{{guid}}` → формат 8-4-4-4-12 hex, версия 4 (биты variant/version корректные)
  - `{{hex:N}}` → N строчных hex-символов
  - `{{string:N}}` → N символов `[a-z0-9]`
  - `{{email}}` → вида `name123@example.com`
  - `{{phone}}` → `+7 9XX XXX-XX-XX`
  - `{{inn}}` → 10 цифр, первая не 0
  - `{{snils}}` → `XXX-XXX-XX XX` (11 цифр)
  - `{{name}}` → случайные ФИО из встроенного списка ≥8 вариантов (русские)
  - `{{date:OFFSET}}` → `YYYY-MM-DD` сегодня + OFFSET дней (int со знаком)
  - `{{datetime:OFFSET}}` → `YYYY-MM-DDTHH:MM:SS` сегодня+OFFSET дней, время случайное
  - `{{pick:A|B|C}}` → один из вариантов по `|` (внутри могут быть пробелы, не тримить содержимое)
- PRNG: свой xorshift64*, сид из nanos SystemTime. Без крейтов rand/getrandom.
- Обязателен модуль `#[cfg(test)] mod tests` (≥8 тестов: границы int, unknown-токен сохраняется, guid regex-подобная проверка вручную, длина string, pick из списка, float формат, date сдвиг, empty input).
- Никаких других pub-ссылок; файл автономный.

## Контракт B — `src/mock_server.rs` (crate сервера)
Хранение профиля: `~/.bunny_hops/mock/<name>.json`, имя sanitized `[a-z0-9_-]` (иное → ошибка). Схема JSON (serde struct'ы с этими полями camelCase НЕ нужны, snake_case как в примере):
```json
{
  "name": "one-c",
  "listen_port": 3100,
  "upstream": "http://127.0.0.1:8080",
  "rules": [
    {
      "id": "r1", "label": "persondata", "method": "*", "match": "prefix",
      "path": "/api/personal-data", "enabled": true,
      "active_variant": "v1", "cycle": false,
      "variants": [
        {"id": "v1", "label": "ok", "status": 200, "json": "{\"a\":1}", "randomize": true, "delay_ms": 0}
      ]
    }
  ]
}
```
`upstream` — Option<String>. `match` — `"exact"` или `"prefix"`. `method` — `"*"` или HTTP-глагол.

Публичный API файла (дословно):
```rust
pub fn control_router() -> axum::Router   // все /api/mock/* маршруты
```
Контрольные маршруты (ответы — JSON, ошибки — `{"error": "..."}`):
- `GET /api/mock/profiles` → `{"profiles":[{"name":str,"listen_port":u16,"running":bool,"rules":usize}]}` (обход каталога)
- `GET /api/mock/profile?name=X` → профиль JSON; нет → 404
- `POST /api/mock/profile` (тело=профиль) → сохраняет, `{"ok":true}`; имя невалидное → 400
- `POST /api/mock/profile/delete` `{"name":X}` → остановить если запущен, удалить файл, `{"ok":true}`
- `POST /api/mock/start` `{"name":X}` → `{"ok":true,"port":N}`; порт занят/нет файла → 400 `{"error":...}`
- `POST /api/mock/stop` `{"name":X}` → `{"ok":true}` (не запущен → тоже ok)
- `GET /api/mock/status` → `{"running":[{"name":...,"port":...}]}`
- `GET /api/mock/traffic?name=X` → `{"traffic":[...до 50 последних...]}`, элемент: `{"ts":f64,"method":str,"path":str,"status":u16,"ms":u64,"rule":String|null,"variant":String|null}`
- `POST /api/mock/preview` `{"json":str,"randomize":bool}` → `{"text":str}` (один рендер через mock_random)

Шлюз (запускается на `listen_port`, любой метод/путь):
1. Каждый запрос перечитывает профиль с диска (горячая правка из UI без рестарта).
2. Первое включённое правило: метод `*` или совпал; путь по `match`.
3. Вариант: `active_variant`; если `cycle=true` — ротация по кругу (счётчик на правило, глобальная static-карта).
4. `delay_ms` → tokio::sleep. Тело: `randomize=true` → `mock_random::randomize_text(json)`.
5. Ответ: status варианта, `Content-Type: application/json`.
6. Нет правила: `upstream` задан → прозрачная проксировка reqwest (метод, path+query к upstream+path, заголовки без hop-by-hop и host, тело как есть; вернуть статус/тело/content-type; ошибка сети → 502 `{"error":"upstream: ..."}`). Нет upstream → 502 `{"error":"no matching rule"}`.
7. Каждый запрос шлюза — запись в traffic-кольцо профиля (Mutex<HashMap<String, VecDeque>>, cap 50).
- Запуск: `std::net::TcpListener::bind(("127.0.0.1", port))`, `tokio::spawn(axum::serve(...).into_future())`, handle в глобальной static-карте; stop → abort. Ошибка бинда → Err(text).
- Фолбэк-роутер движка: `Router::new().fallback(any(gateway_handler))` — никаких wildcard-синтаксисов.
- В файле `#[cfg(test)] mod tests` не нужен.

## Контракт C — `frontend/src/proxy.rs` (leptos 0.7 csr, edition 2021)
```rust
#[component]
pub fn ProxyView() -> impl IntoView { ... }
```
Компонент виден как `use crate::proxy::ProxyView;`. Работает ТОЛЬКО с контрольным API раздела «Контракт B» через два помощника (включи их в файл дословно):
```rust
async fn fetch_json_get(url: &str) -> Result<serde_json::Value, String> {
    let win = web_sys::window().unwrap();
    let resp_val = wasm_bindgen_futures::JsFuture::from(win.fetch_with_str(url)).await
        .map_err(|e| format!("{e:?}"))?;
    let resp: web_sys::Response = resp_val.dyn_into()?;
    if !resp.ok() { return Err(format!("HTTP {}", resp.status())); }
    let txt = resp.text().unwrap();
    let val = wasm_bindgen_futures::JsFuture::from(txt).await.map_err(|e| format!("{e:?}"))?;
    let s = val.as_string().unwrap_or_default();
    serde_json::from_str(&s).map_err(|e| e.to_string())
}
// fetch_json_post — тот же паттерн, но RequestInit: method POST, headers content-type application/json, body
```
(реализуй post по образцу stream.rs: `Request::new_with_str_and_init` + `RequestInit` + `Headers` — см. приложенный фрагмент).

Данные — `serde_json::Value` (не типизировать struct'ами — профиль правится точечно), сигналы `create_rw_signal`.

Раскладка (Postman-style, три колонки в `div.px-layout`):
- Левая `px-side` (220px): заголовок «Профили», список профилей (`px-item`, активный `.active`, точка running), кнопка `px-btn` «Новый профиль» (prompt-имя через `window().prompt`), удаление активного (confirm).
- Центр `px-main`: карточка профиля: поля name (readonly), listen_port (number), upstream (text); кнопки `px-btn primary` «Сохранить» (POST profile) и «Запустить»/«Остановить» (start/stop, статус-строка `px-status`); ниже список правил (`px-rules`): выбор правила, «+ правило»; редактор правила: label, method (`select`: *, GET, POST, PUT, PATCH, DELETE), match (`select`: exact/prefix), path, enabled (checkbox), cycle (checkbox); ниже чипы вариантов `px-chip` (активный `.active`, «+ вариант», удаление), редактор варианта: label, status (number), delay_ms (number), randomize (checkbox с подписью «Рандомизировать {{...}}»), textarea `px-json` (rows=14, monospace) с кнопкой «Предпросмотр» (POST preview → показать результат под textarea).
- Правая `px-monitor` (300px): «Трафик» — таблица `px-row`: method, path (truncate), status (`.ok` <400, `.err` ≥400), ms. Пока профиль запущен — опрос `traffic` каждые 2000 мс (`w.set_timeout_with_callback_and_timeout_and_arguments_0` — паттерн из app.rs, приложен).
- Всё состояние — внутри компонента; при смене выбранного профиля/правила — точечные правки `Value` и сохранение целиком.
- Ошибки API — красная строка в шапке (`px-status.err`), без alert.
- Используй хелперы `input_value/input_checked/select_value` из `crate::ui` (сигнатуры в приложенном фрагменте).
- Никаких новых CSS-классов кроме `px-`-префикса из контракта D.

## Контракт D — `frontend/styles-proxy.css`
Только эти классы (их использует proxy.rs):
`px-layout px-side px-main px-monitor px-h px-item (.active) px-dot px-dot.on px-btn (.primary) px-field px-label px-input px-check px-rules px-chip (.active) px-json px-table px-row px-status (.ok .err) px-empty px-grid2`
- Только CSS-переменные из приложенного `:root` (bg, bg-panel, bg-raised, border, text, text-dim, accent, accent-dim и т.п.), новых цветов не вводить.
- Тёмная тема, компактно, радиусы 8px, моноширинный шрифт для `px-json`, sticky-заголовки колонок, скроллбары как в приложенном стиле.
- `px-layout { display:grid; grid-template-columns: 220px 1fr 300px; height:100%; }` — каркас зафиксирован, остальное на тебе.

## Контракт E — `tests/api_matrix_mock.py`
Python 3, только stdlib. Стиль и вывод — как приложенный api_matrix.py (тот же формат PASS/FAIL строк и exit code). Тесты (каждый — отдельная функция, нумерованные имена T1..):
- T1 CRUD: создать профиль `pytest-mock` (listen_port 3199, одно правило prefix `/api/test`, вариант status 200, json `{"v":1,"id":"{{guid}}"}`, randomize true), получить, в списке, удалить потом в конце
- T2 start → порт слушается (connect ok), stop → connect refused
- T3 randomize: два GET `http://127.0.0.1:3199/api/test/x` → json валиден, `id` различается
- T4 static: randomize=false → два ответа побайтово равны
- T5 no-rule: GET другого пути, upstream=null → 502 + `{"error"`
- T6 passthrough: локальный echo-сервер на 3198 (threading+http.server), upstream=echo, запрос с телом → тело и статус вернулись как от echo
- T7 traffic: после T3 GET /api/mock/traffic?name=... содержит путь /api/test/x со статусом 200
- T8 preview: POST /api/mock/preview `{"json":"{{int:5:5}}","randomize":true}` → text=="5"
BASE берётся из env `HOP_URL` (default `http://127.0.0.1:3000`). Таймауты HTTP 5 с.

## Что НЕ делать
- Не трогать main.rs, lib.rs, app.rs, index.html, styles.css — интеграцию делает заказчик.
- Не запускать cargo/trunk/python — сборка у заказчика; код должен пройти `cargo check`/`node --check`-эквивалент с первого раза.
