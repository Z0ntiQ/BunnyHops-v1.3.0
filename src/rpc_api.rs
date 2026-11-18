//! JSON-RPC 2.0 endpoint `/rpc` с Bearer-токеном.
//!
//! Методы: `system.get_state`, `ui.inspect`, `ui.click`, `ui.type`, `ui.wait`, `app.execute`.
//! Коды ошибок: -32601 (нет метода), -32602 (параметры), -32603 (внутренняя, data.stacktrace).
//! Авторизация: `Authorization: Bearer <token>` обязателен на `/rpc`.

use axum::body::to_bytes;
use axum::extract::{Request, State};
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

/// JSON-RPC 2.0 ответ (успех).
#[derive(Serialize)]
struct JsonRpcSuccess {
    jsonrpc: String,
    id: Value,
    result: Value,
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

/// Состояние RPC-роутера: токен + ссылка на AppState (для вызовов функций).
#[derive(Clone)]
pub struct RpcState {
    pub token: String,
    pub app_state: AppState,
}

/// Создать RPC-роутер.
pub fn rpc_router(token: String, app_state: AppState) -> Router {
    let state = RpcState { token, app_state };
    Router::new()
        .route("/rpc", post(rpc_handler))
        .with_state(state)
}

/// Обработчик POST /rpc.
async fn rpc_handler(State(state): State<RpcState>, req: Request) -> Response {
    // 1. Проверка Bearer-токена
    let auth = req.headers().get("authorization");
    let token_ok = auth
        .and_then(|h| h.to_str().ok())
        .map(|h| h.strip_prefix("Bearer ").unwrap_or(h))
        .map(|t| t == state.token)
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
        "system.get_state" => handle_system_get_state(&state, request.params).await,
        "ui.inspect" => handle_ui_inspect(&state, request.params).await,
        "ui.click" => handle_ui_click(&state, request.params).await,
        "ui.type" => handle_ui_type(&state, request.params).await,
        "ui.wait" => handle_ui_wait(&state, request.params).await,
        "app.execute" => handle_app_execute(&state, request.params).await,
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
            let resp = JsonRpcSuccess {
                jsonrpc: "2.0".to_string(),
                id,
                result,
            };
            control::json_response(StatusCode::OK, &serde_json::to_value(resp).unwrap())
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
async fn handle_system_get_state(_state: &RpcState, _params: Option<Value>) -> Result<Value, (i32, String, Option<Value>)> {
    // Используем существующую функцию debug_api::full_state()
    Ok(debug_api::full_state())
}

/// ui.inspect — возвращает BROOP-карту (как GET /debug/broop).
async fn handle_ui_inspect(_state: &RpcState, _params: Option<Value>) -> Result<Value, (i32, String, Option<Value>)> {
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
                    o.insert("declared_as".into(), obj.get("id").cloned().unwrap_or(Value::Null));
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
            !catalog
                .iter()
                .any(|o| debug_api::matches_catalog(o.get("id").and_then(Value::as_str).unwrap_or(""), debug_api::element_id(el)))
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

/// ui.click — клик по элементу BROOP по id.
async fn handle_ui_click(_state: &RpcState, params: Option<Value>) -> Result<Value, (i32, String, Option<Value>)> {
    let id = params
        .as_ref()
        .and_then(|p| p.get("id"))
        .and_then(Value::as_str)
        .ok_or((-32602, "Missing required parameter: id".to_string(), None))?;

    match debug_api::gate_check(id) {
        Ok(_) => {
            let seq_before = debug_api::ui_seq();
            debug_api::publish(json!({"type": "ui_command", "path": "element.click", "value": {"id": id}}));
            let result = wait_ui_echo_rpc(seq_before).await;
            Ok(result)
        }
        Err(debug_api::GateReject::NotFound) => {
            debug_api::publish(json!({"action": "elements.collect"}));
            let rx = debug_api::register_elements_wait(id);
            let wait_result = tokio::time::timeout(std::time::Duration::from_millis(2000), rx).await;
            if wait_result.is_err() {
                return Err((
                    -32603,
                    format!("Object '{}' not on screen; map — GET /debug/broop (map refresh timeout)", id),
                    Some(json!({"stacktrace": "gate_check NotFound after round-trip timeout"})),
                ));
            }
            match debug_api::gate_check(id) {
                Ok(_) => {
                    let seq_before = debug_api::ui_seq();
                    debug_api::publish(json!({"type": "ui_command", "path": "element.click", "value": {"id": id}}));
                    let result = wait_ui_echo_rpc(seq_before).await;
                    Ok(result)
                }
                Err(debug_api::GateReject::NotFound) => Err((
                    -32603,
                    format!("Object '{}' not on screen after map refresh; map — GET /debug/broop", id),
                    Some(json!({"stacktrace": "gate_check NotFound after round-trip"})),
                )),
                Err(debug_api::GateReject::Disabled) => Err((
                    -32603,
                    format!("Object '{}' is disabled", id),
                    Some(json!({"stacktrace": "gate_check Disabled"})),
                )),
                Err(debug_api::GateReject::Invisible) => Err((
                    -32603,
                    format!("Object '{}' is invisible", id),
                    Some(json!({"stacktrace": "gate_check Invisible"})),
                )),
            }
        }
        Err(debug_api::GateReject::Disabled) => Err((
            -32603,
            format!("Object '{}' is disabled", id),
            Some(json!({"stacktrace": "gate_check Disabled"})),
        )),
        Err(debug_api::GateReject::Invisible) => Err((
            -32603,
            format!("Object '{}' is invisible", id),
            Some(json!({"stacktrace": "gate_check Invisible"})),
        )),
    }
}

/// ui.type — установка текста в элемент.
async fn handle_ui_type(_state: &RpcState, params: Option<Value>) -> Result<Value, (i32, String, Option<Value>)> {
    let id = params
        .as_ref()
        .and_then(|p| p.get("id"))
        .and_then(Value::as_str)
        .ok_or((-32602, "Missing required parameter: id".to_string(), None))?;
    let text = params
        .as_ref()
        .and_then(|p| p.get("text"))
        .ok_or((-32602, "Missing required parameter: text".to_string(), None))?;

    if debug_api::live_elements().into_iter().all(|el| debug_api::element_id(&el) != id) {
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

/// ui.wait — ожидание условия (заглушка, возвращает -32603 not implemented).
async fn handle_ui_wait(_state: &RpcState, _params: Option<Value>) -> Result<Value, (i32, String, Option<Value>)> {
    Err((
        -32603,
        "Method not implemented".to_string(),
        Some(json!({"stacktrace": "not implemented"})),
    ))
}

/// app.execute — выполнение действия из реестра debug_api (ui.set, ui.reset, ui.click, element.click, element.set, eval, traffic.clear, logs.clear).
async fn handle_app_execute(_state: &RpcState, params: Option<Value>) -> Result<Value, (i32, String, Option<Value>)> {
    let action = params
        .as_ref()
        .and_then(|p| p.get("action"))
        .and_then(Value::as_str)
        .ok_or((-32602, "Missing required parameter: action".to_string(), None))?;
    let payload = params
        .as_ref()
        .and_then(|p| p.get("payload"))
        .cloned()
        .unwrap_or(json!({}));

    if !debug_api::is_known_action(action) {
        return Err((
            -32601,
            format!("Unknown action '{}'", action),
            None,
        ));
    }

    let (before, after, result) = debug_api::run_action(action, payload).await;
    let diff = debug_api::json_diff(&before, &after);
    match result {
        Ok(data) => Ok(json!({"ok": true, "status": "ok", "action": action, "result": data, "stateDiff": diff})),
        Err((status, message)) => Err((
            -32603,
            message,
            Some(json!({"status": status.as_u16()})),
        )),
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
        rpc_router(token.to_string(), test_app_state())
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
        let (status, body) = rpc_call(&router, None, "system.get_state", None, Some(json!(1))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], -32600);
        assert!(body["error"]["message"].as_str().unwrap().contains("Bearer token required"));
    }

    #[tokio::test]
    async fn rpc_unknown_method_is_32601() {
        let router = test_rpc_router("secret123");
        let (status, body) = rpc_call(&router, Some("secret123"), "unknown.method", None, Some(json!(1))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["error"]["code"], -32601);
        assert!(body["error"]["message"].as_str().unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn rpc_missing_params_is_32602() {
        let router = test_rpc_router("secret123");
        let (status, body) = rpc_call(&router, Some("secret123"), "ui.click", None, Some(json!(1))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["error"]["code"], -32602);
        assert!(body["error"]["message"].as_str().unwrap().contains("Missing required parameter"));
    }

    #[tokio::test]
    async fn rpc_get_state_returns_result() {
        let router = test_rpc_router("secret123");
        let (status, body) = rpc_call(&router, Some("secret123"), "system.get_state", None, Some(json!(1))).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["result"].is_object());
        assert!(body["result"]["app"].is_object());
        assert_eq!(body["id"], 1);
    }
}