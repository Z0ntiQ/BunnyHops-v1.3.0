//! JSON-RPC 2.0 endpoint `/rpc` с Bearer-токеном.
//!
//! Методы: `system.get_state`, `ui.inspect`, `ui.click`, `ui.type`, `ui.wait`, `app.execute`.
//! Коды ошибок: -32601 (нет метода), -32602 (параметры), -32603 (внутренняя, data.stacktrace).
//! Авторизация: `Authorization: Bearer *** обязателен на `/rpc`.

use axum::body::to_bytes;
use axum::extract::{Extension, Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::control;
use crate::debug_api;
use crate::AppState;

const MAX_BODY: usize = 10 * 1024 * 1024;

/// Путь lock-файла: ~/.config/bunny-hops/rpc.json. Внешний MCP-клиент читает
/// его перед каждым вызовом и проверяет живость pid (os.kill(pid, 0)).
pub fn lock_file_path() -> std::path::PathBuf {
    // Windows: HOME не существует (запуск через Планировщик задач), нужен USERPROFILE;
    // иначе путь уходит в CWD и lock не пишется (инцидент 18.11.2026, TB49).
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".config/bunny-hops/rpc.json")
}

/// Случайный Bearer-токен: 16 байт из /dev/urandom в hex; fallback — время+pid.
pub fn generate_token() -> String {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    match std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes).map(|_| bytes))
    {
        Ok(b) => b.iter().map(|x| format!("{x:02x}")).collect(),
        Err(_) => {
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0)
                ^ (std::process::id() as u64) << 32;
            format!("{t:016x}{t:016x}")
        }
    }
}

/// Публикует порт+токен+pid в lock-файл; ошибка не фатальна для сервера.
pub fn write_lock_file(port: u16, token: &str) -> Result<(), String> {
    let path = lock_file_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("не создана папка {}: {e}", dir.display()))?;
    }
    let body = json!({ "port": port, "token": token, "pid": std::process::id() });
    std::fs::write(&path, body.to_string())
        .map_err(|e| format!("не записан {}: {e}", path.display()))?;
    // Токен в файле = секрет: режим 0600, только владелец читает.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Убирает lock-файл при остановке; отсутствие файла — не ошибка.
pub fn remove_lock_file() {
    let _ = std::fs::remove_file(lock_file_path());
}

/// Входящий JSON-RPC 2.0 запрос.
#[derive(Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

/// JSON-RPC 2.0 ответ (ошибка).
#[derive(Serialize)]
struct JsonRpcError {
    jsonrpc: String,
    id: Value,
    error: JsonRpcErrorObject,
}

#[derive(Serialize)]
struct JsonRpcErrorObject {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

/// Создать RPC-роутер. Состояние AppState применяется главным роутером
/// через .with_state(); в тестах — через .with_state(test_app_state()).
/// Токен передаётся через Extension layer.
pub fn rpc_router(token: String) -> Router<AppState> {
    Router::<AppState>::new()
        .route("/rpc", post(rpc_handler))
        .layer(Extension(token))
}

/// Обработчик POST /rpc.
async fn rpc_handler(
    Extension(token): Extension<String>,
    State(app_state): State<AppState>,
    req: Request,
) -> Response {
    // 1. Проверка Bearer-токена
    let auth = req.headers().get("authorization");
    let token_ok = auth
        .and_then(|h| h.to_str().ok())
        .map(|h| h.strip_prefix("Bearer ").unwrap_or(h))
        .map(|t| t == token)
        .unwrap_or(false);

    if !token_ok {
        return json_rpc_error_response(
            None,
            -32600, // Invalid Request
            "Bearer token required",
            None,
            StatusCode::UNAUTHORIZED,
        );
    }

    // 2. Чтение тела
    let bytes = match to_bytes(req.into_body(), MAX_BODY).await {
        Ok(b) => b,
        Err(_) => {
            return json_rpc_error_response(
                None,
                -32700, // Parse error
                "Request body too large",
                None,
                StatusCode::PAYLOAD_TOO_LARGE,
            );
        }
    };

    if bytes.is_empty() {
        return json_rpc_error_response(
            None,
            -32700,
            "Empty request body",
            None,
            StatusCode::BAD_REQUEST,
        );
    }

    // 3. Парсинг JSON-RPC запроса
    let request: JsonRpcRequest = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(_) => {
            return json_rpc_error_response(
                None,
                -32700, // Parse error
                "Invalid JSON",
                None,
                StatusCode::BAD_REQUEST,
            );
        }
    };

    // 4. Проверка jsonrpc версии
    if request.jsonrpc != "2.0" {
        return json_rpc_error_response(
            request.id,
            -32600,
            "Invalid JSON-RPC version (expected \"2.0\")",
            None,
            StatusCode::BAD_REQUEST,
        );
    }

    let id = request.id.unwrap_or(Value::Null);

    // 5. Вызов метода через match
    let result = match request.method.as_str() {
        "system.get_state" => handle_system_get_state(&app_state, request.params).await,
        "ui.inspect" => handle_ui_inspect(&app_state, request.params).await,
        "ui.click" => handle_ui_click(&app_state, request.params).await,
        "ui.type" => handle_ui_type(&app_state, request.params).await,
        "ui.wait" => handle_ui_wait(&app_state, request.params).await,
        "app.execute" => handle_app_execute(&app_state, request.params).await,
        _ => {
            return json_rpc_error_response(
                Some(id),
                -32601, // Method not found
                &format!("Method '{}' not found", request.method),
                None,
                StatusCode::OK, // JSON-RPC ошибки возвращаются с 200
            );
        }
    };

    // 6. Формирование ответа
    match result {
        Ok(result) => {
            let resp = json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result
            });
            control::json_response(StatusCode::OK, &resp)
        }
        Err((code, message, data)) => {
            json_rpc_error_response(Some(id), code, &message, data, StatusCode::OK)
        }
    }
}

/// Вспомогательная: JSON-RPC ошибка с заданным HTTP-статусом.
fn json_rpc_error_response(
    id: Option<Value>,
    code: i32,
    message: &str,
    data: Option<Value>,
    http_status: StatusCode,
) -> Response {
    let err = JsonRpcError {
        jsonrpc: "2.0".to_string(),
        id: id.unwrap_or(Value::Null),
        error: JsonRpcErrorObject {
            code,
            message: message.to_string(),
            data,
        },
    };
    control::json_response(http_status, &serde_json::to_value(err).unwrap())
}

// ===== Обработчики методов =====

/// system.get_state — возвращает полное состояние приложения.
async fn handle_system_get_state(
    _state: &AppState,
    _params: Option<Value>,
) -> Result<Value, (i32, String, Option<Value>)> {
    // Используем существующую функцию debug_api::full_state()
    Ok(debug_api::full_state())
}

/// ui.inspect — возвращает BROOP-карту (как GET /debug/broop).
async fn handle_ui_inspect(
    _state: &AppState,
    _params: Option<Value>,
) -> Result<Value, (i32, String, Option<Value>)> {
    let doc = debug_api::broop_doc();
    let catalog: Vec<Value> = doc
        .get("objects")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let live = debug_api::live_elements();
    let mut objects = Vec::new();
    let mut missing = Vec::new();
    for obj in &catalog {
        let pattern = obj.get("id").and_then(Value::as_str).unwrap_or("");
        let hits: Vec<&Value> = live
            .iter()
            .filter(|el| debug_api::matches_catalog(pattern, debug_api::element_id(el)))
            .collect();
        if hits.is_empty() {
            missing.push(json!({"id": pattern, "reason": "не на экране (скрыт или не отрисован)"}));
        } else {
            for el in hits {
                let mut merged = obj.clone();
                if let Some(o) = merged.as_object_mut() {
                    o.insert(
                        "declared_as".into(),
                        obj.get("id").cloned().unwrap_or(Value::Null),
                    );
                    o.insert("id".into(), json!(debug_api::element_id(el)));
                    o.insert("state".into(), el.clone());
                }
                objects.push(merged);
            }
        }
    }
    let undeclared: Vec<&Value> = live
        .iter()
        .filter(|el| {
            !catalog.iter().any(|o| {
                debug_api::matches_catalog(
                    o.get("id").and_then(Value::as_str).unwrap_or(""),
                    debug_api::element_id(el),
                )
            })
        })
        .collect();
    let unmarked = debug_api::live_unmarked();
    Ok(json!({
        "ok": true,
        "screen": doc.get("screen").cloned().unwrap_or(Value::Null),
        "objects": objects,
        "missing_in_dom": missing,
        "undeclared_in_broop": undeclared,
        "unmarked_in_dom": unmarked,
    }))
}

/// Клик по элементу и ожидание RPC-эха.
async fn click_and_wait_rpc(id: &str) -> Value {
    let seq_before = debug_api::ui_seq();
    debug_api::publish(json!({"type": "ui_command", "path": "element.click", "value": {"id": id}}));
    wait_ui_echo_rpc(seq_before).await
}

/// JSON-RPC-ошибка на отказ гейта; after_refresh — что карта уже обновлялась round-trip'ом.
fn rpc_gate_err(
    id: &str,
    r: debug_api::GateReject,
    after_refresh: bool,
) -> (i32, String, Option<Value>) {
    let (msg, trace) = match r {
        debug_api::GateReject::NotFound => (
            if after_refresh {
                format!("Object '{id}' not on screen after map refresh; map — GET /debug/broop")
            } else {
                format!("Object '{id}' not on screen; map — GET /debug/broop")
            },
            "gate_check NotFound",
        ),
        debug_api::GateReject::Disabled => {
            (format!("Object '{id}' is disabled"), "gate_check Disabled")
        }
        debug_api::GateReject::Invisible => (
            format!("Object '{id}' is invisible"),
            "gate_check Invisible",
        ),
    };
    (-32603, msg, Some(json!({"stacktrace": trace})))
}

/// ui.click — клик по элементу BROOP по id.
async fn handle_ui_click(
    _state: &AppState,
    params: Option<Value>,
) -> Result<Value, (i32, String, Option<Value>)> {
    let id = params
        .as_ref()
        .and_then(|p| p.get("id"))
        .and_then(Value::as_str)
        .ok_or((-32602, "Missing required parameter: id".to_string(), None))?;

    match debug_api::gate_check(id) {
        Ok(_) => Ok(click_and_wait_rpc(id).await),
        Err(debug_api::GateReject::NotFound) => {
            debug_api::publish(json!({"type": "action", "action": "elements.collect"}));
            let rx = debug_api::register_elements_wait(id);
            if tokio::time::timeout(std::time::Duration::from_millis(2000), rx)
                .await
                .is_err()
            {
                return Err((
                    -32603,
                    format!(
                        "Object '{}' not on screen; map — GET /debug/broop (map refresh timeout)",
                        id
                    ),
                    Some(json!({"stacktrace": "gate_check NotFound after round-trip timeout"})),
                ));
            }
            match debug_api::gate_check(id) {
                Ok(_) => Ok(click_and_wait_rpc(id).await),
                Err(r) => Err(rpc_gate_err(id, r, true)),
            }
        }
        Err(r) => Err(rpc_gate_err(id, r, false)),
    }
}

/// ui.type — установка текста в элемент.
async fn handle_ui_type(
    _state: &AppState,
    params: Option<Value>,
) -> Result<Value, (i32, String, Option<Value>)> {
    let id = params
        .as_ref()
        .and_then(|p| p.get("id"))
        .and_then(Value::as_str)
        .ok_or((-32602, "Missing required parameter: id".to_string(), None))?;
    let text = params.as_ref().and_then(|p| p.get("text")).ok_or((
        -32602,
        "Missing required parameter: text".to_string(),
        None,
    ))?;

    if debug_api::live_elements()
        .into_iter()
        .all(|el| debug_api::element_id(&el) != id)
    {
        return Err((
            -32603,
            format!("Object '{}' not on screen; map — GET /debug/broop", id),
            Some(json!({"stacktrace": "element not found in live elements"})),
        ));
    }

    let seq_before = debug_api::ui_seq();
    debug_api::publish(json!({
        "type": "ui_command",
        "path": "element.set",
        "value": {"id": id, "value": text}
    }));
    let result = wait_ui_echo_rpc(seq_before).await;
    Ok(result)
}

/// ui.wait — ожидание условия через общее ядро debug_api::wait_for_predicate.
async fn handle_ui_wait(
    _state: &AppState,
    params: Option<Value>,
) -> Result<Value, (i32, String, Option<Value>)> {
    let predicate = params
        .as_ref()
        .and_then(|p| p.get("predicate"))
        .cloned()
        .ok_or((
            -32602,
            "Missing required parameter: predicate".to_string(),
            None,
        ))?;
    let timeout_ms = params
        .as_ref()
        .and_then(|p| p.get("timeout_ms"))
        .and_then(Value::as_u64)
        .unwrap_or(5000);
    Ok(debug_api::wait_for_predicate(&predicate, timeout_ms).await)
}

/// app.execute — выполнение действия из реестра debug_api (ui.set, ui.reset, ui.click, element.click, element.set, eval, traffic.clear, logs.clear).
async fn handle_app_execute(
    _state: &AppState,
    params: Option<Value>,
) -> Result<Value, (i32, String, Option<Value>)> {
    let action = params
        .as_ref()
        .and_then(|p| p.get("action"))
        .and_then(Value::as_str)
        .ok_or((
            -32602,
            "Missing required parameter: action".to_string(),
            None,
        ))?;
    let payload = params
        .as_ref()
        .and_then(|p| p.get("payload"))
        .cloned()
        .unwrap_or(json!({}));

    if !debug_api::is_known_action(action) {
        return Err((-32601, format!("Unknown action '{}'", action), None));
    }

    let (before, after, result) = debug_api::run_action(action, payload).await;
    let diff = debug_api::json_diff(&before, &after);
    match result {
        Ok(data) => Ok(
            json!({"ok": true, "status": "ok", "action": action, "result": data, "stateDiff": diff}),
        ),
        Err((status, message)) => Err((-32603, message, Some(json!({"status": status.as_u16()})))),
    }
}

/// Ждёт эхо от UI (макс 2с), возвращает результат как Value.
async fn wait_ui_echo_rpc(seq_before: u64) -> Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(2000);
    while std::time::Instant::now() < deadline {
        let seq = debug_api::ui_seq();
        if seq > seq_before {
            return json!({"status": "applied", "ui_seq": seq});
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    json!({"status": "pending", "error": "UI client did not confirm change within 2s: open app window"})
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{HeaderValue, Request, StatusCode};
    use axum::Router;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::Mutex;
    use tower::ServiceExt;

    // Минимальное AppState для тестов
    fn test_app_state() -> AppState {
        AppState {
            client: reqwest::Client::new(),
            api_log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn test_rpc_router(token: &str) -> Router {
        rpc_router(token.to_string()).with_state(test_app_state())
    }

    async fn rpc_call(
        router: &Router,
        token: Option<&str>,
        method: &str,
        params: Option<Value>,
        id: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req_body = json!({
            "jsonrpc": "2.0",
            "method": method,
        });
        if let Some(p) = params {
            req_body["params"] = p;
        }
        if let Some(i) = id {
            req_body["id"] = i;
        }

        let mut req = Request::builder()
            .method("POST")
            .uri("/rpc")
            .header("Content-Type", "application/json")
            .body(Body::from(req_body.to_string()))
            .unwrap();

        if let Some(t) = token {
            req.headers_mut().insert(
                "Authorization",
                HeaderValue::from_str(&format!("Bearer {}", t)).unwrap(),
            );
        }

        let response = router.clone().oneshot(req).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
        (status, json)
    }

    #[tokio::test]
    async fn rpc_requires_bearer() {
        let router = test_rpc_router("secret123");
        let (status, body) =
            rpc_call(&router, None, "system.get_state", None, Some(json!(1))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], -32600);
        assert!(body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Bearer token required"));
    }

    #[tokio::test]
    async fn rpc_unknown_method_is_32601() {
        let router = test_rpc_router("secret123");
        let (status, body) = rpc_call(
            &router,
            Some("secret123"),
            "unknown.method",
            None,
            Some(json!(1)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["error"]["code"], -32601);
        assert!(body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not found"));
    }

    #[tokio::test]
    async fn rpc_missing_params_is_32602() {
        let router = test_rpc_router("secret123");
        let (status, body) =
            rpc_call(&router, Some("secret123"), "ui.click", None, Some(json!(1))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["error"]["code"], -32602);
        assert!(body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Missing required parameter"));
    }

    #[tokio::test]
    async fn rpc_get_state_returns_result() {
        let router = test_rpc_router("secret123");
        let (status, body) = rpc_call(
            &router,
            Some("secret123"),
            "system.get_state",
            None,
            Some(json!(1)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["result"].is_object());
        assert!(body["result"]["app"].is_object());
        assert_eq!(body["id"], 1);
    }

    #[tokio::test]
    async fn rpc_ui_wait_returns_result() {
        // Предикат element/absent матчится мгновенно: элемента нет в живом сторе.
        let router = test_rpc_router("secret123");
        let params = json!({
            "predicate": {"kind": "element", "id": "no.such.element", "state": "absent"},
            "timeout_ms": 500,
        });
        let (status, body) = rpc_call(
            &router,
            Some("secret123"),
            "ui.wait",
            Some(params),
            Some(json!(1)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["ok"], true);
        assert_eq!(body["result"]["matched"]["state"], "absent");
    }

    #[tokio::test]
    async fn rpc_ui_wait_requires_predicate() {
        let router = test_rpc_router("secret123");
        let (status, body) = rpc_call(
            &router,
            Some("secret123"),
            "ui.wait",
            Some(json!({})),
            Some(json!(1)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["error"]["code"], -32602);
    }
}
