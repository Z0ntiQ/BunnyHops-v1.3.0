use axum::body::{to_bytes, Body};
use axum::extract::{Query, Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use axum::Router;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tokio::time::sleep;

use crate::mock_random::randomize_text;

const MOCK_DIR_NAME: &str = "mock";
const TRAFFIC_CAP: usize = 50;
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

static RUNNING_SERVERS: OnceLock<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>> = OnceLock::new();
static TRAFFIC_STORE: OnceLock<Mutex<HashMap<String, VecDeque<TrafficEntry>>>> = OnceLock::new();
static CYCLE_COUNTERS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();

fn running_servers() -> &'static Mutex<HashMap<String, tokio::task::JoinHandle<()>>> {
    RUNNING_SERVERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Снимок состояния mock-движка для control-плоскости (/api/status).
pub fn control_snapshot() -> serde_json::Value {
    let servers: Vec<serde_json::Value> = running_servers()
        .lock()
        .unwrap()
        .iter()
        .map(|(name, handle)| {
            serde_json::json!({
                "name": name,
                "running": !handle.is_finished(),
                "port": load_profile(name).map(|p| p.listen_port).unwrap_or(0),
            })
        })
        .collect();
    let running_count = servers
        .iter()
        .filter(|s| s.get("running").and_then(serde_json::Value::as_bool).unwrap_or(false))
        .count();
    let traffic_total: usize = traffic_store().lock().unwrap().values().map(|q| q.len()).sum();
    serde_json::json!({
        "servers": servers,
        "running_count": running_count,
        "traffic_entries_total": traffic_total,
    })
}

fn traffic_store() -> &'static Mutex<HashMap<String, VecDeque<TrafficEntry>>> {
    TRAFFIC_STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cycle_counters() -> &'static Mutex<HashMap<String, usize>> {
    CYCLE_COUNTERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn mock_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".bunny_hops")
        .join(MOCK_DIR_NAME)
}

fn sanitize_name(name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err("empty name".into());
    }
    for ch in name.chars() {
        if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() && ch != '_' && ch != '-' {
            return Err("invalid name".into());
        }
    }
    Ok(name.to_string())
}

fn profile_path(name: &str) -> PathBuf {
    mock_dir().join(format!("{name}.json"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Variant {
    id: String,
    label: String,
    status: u16,
    json: String,
    randomize: bool,
    delay_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Rule {
    id: String,
    label: String,
    method: String,
    #[serde(rename = "match")]
    match_type: String,
    path: String,
    enabled: bool,
    active_variant: String,
    cycle: bool,
    variants: Vec<Variant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Profile {
    name: String,
    listen_port: u16,
    upstream: Option<String>,
    rules: Vec<Rule>,
}

#[derive(Debug, Clone, Serialize)]
struct TrafficEntry {
    ts: f64,
    method: String,
    path: String,
    status: u16,
    ms: u64,
    rule: Option<String>,
    variant: Option<String>,
}

#[derive(Deserialize)]
struct NameQuery {
    name: String,
}

#[derive(Deserialize)]
struct DeleteBody {
    name: String,
}

#[derive(Deserialize)]
struct StartBody {
    name: String,
}

#[derive(Deserialize)]
struct PreviewBody {
    json: String,
    randomize: bool,
}

fn load_profile(name: &str) -> Result<Profile, String> {
    let path = profile_path(name);
    let data = fs::read_to_string(&path).map_err(|_| "profile not found")?;
    serde_json::from_str(&data).map_err(|e| format!("invalid json: {e}"))
}

fn save_profile(profile: &Profile) -> Result<(), String> {
    let dir = mock_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir: {e}"))?;
    let path = profile_path(&profile.name);
    let data = serde_json::to_string_pretty(profile).map_err(|e| format!("serialize: {e}"))?;
    fs::write(&path, data).map_err(|e| format!("write: {e}"))
}

fn list_profiles() -> Result<Vec<ProfileSummary>, String> {
    let dir = mock_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("read_dir: {e}"))? {
        let entry = entry.map_err(|e| format!("entry: {e}"))?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                if let Ok(profile) = load_profile(name) {
                    let running = running_servers().lock().unwrap().contains_key(name);
                    out.push(ProfileSummary {
                        name: name.to_string(),
                        listen_port: profile.listen_port,
                        running,
                        rules: profile.rules.len(),
                    });
                }
            }
        }
    }
    Ok(out)
}

#[derive(Serialize)]
struct ProfileSummary {
    name: String,
    listen_port: u16,
    running: bool,
    rules: usize,
}

fn record_traffic(name: &str, entry: TrafficEntry) {
    let mut store = traffic_store().lock().unwrap();
    let deque = store.entry(name.to_string()).or_default();
    if deque.len() >= TRAFFIC_CAP {
        deque.pop_front();
    }
    deque.push_back(entry);
}

fn get_traffic(name: &str) -> Vec<TrafficEntry> {
    traffic_store()
        .lock()
        .unwrap()
        .get(name)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect()
}

async fn gateway_handler(
    State(profile_name): State<String>,
    req: Request,
) -> Response {
    let start = SystemTime::now();
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let full_path = format!("{path}{query}");

    let profile = match load_profile(&profile_name) {
        Ok(p) => p,
        Err(_) => {
            let err = TrafficEntry {
                ts: now_ts(),
                method: method.to_string(),
                path: full_path.clone(),
                status: 502,
                ms: elapsed_ms(start),
                rule: None,
                variant: None,
            };
            record_traffic(&profile_name, err);
            return json_error("profile not found", StatusCode::BAD_GATEWAY);
        }
    };

    let mut matched_rule: Option<&Rule> = None;
    let mut matched_variant: Option<&Variant> = None;

    for rule in &profile.rules {
        if !rule.enabled {
            continue;
        }
        let method_ok = rule.method == "*" || rule.method.eq_ignore_ascii_case(method.as_str());
        if !method_ok {
            continue;
        }
        let path_ok = match rule.match_type.as_str() {
            "exact" => rule.path == path,
            "prefix" => path.starts_with(&rule.path),
            _ => false,
        };
        if !path_ok {
            continue;
        }
        matched_rule = Some(rule);
        let variant_id = &rule.active_variant;
        if let Some(v) = rule.variants.iter().find(|v| v.id == *variant_id) {
            matched_variant = Some(v);
        }
        break;
    }

    if let (Some(rule), Some(variant)) = (matched_rule, matched_variant) {
        let chosen_idx: usize = if rule.cycle && !rule.variants.is_empty() {
            let mut counters = cycle_counters().lock().unwrap();
            let key = format!("{profile_name}:{}", rule.id);
            match counters.get_mut(&key) {
                Some(idx) => {
                    *idx = (*idx + 1) % rule.variants.len();
                    *idx
                }
                None => {
                    let i = rule.variants.iter().position(|v| v.id == variant.id).unwrap_or(0);
                    counters.insert(key, i);
                    i
                }
            }
        } else {
            rule.variants.iter().position(|v| v.id == variant.id).unwrap_or(0)
        };
        let chosen = rule.variants.get(chosen_idx).unwrap_or(variant);
        if chosen.delay_ms > 0 {
            sleep(Duration::from_millis(chosen.delay_ms)).await;
        }
        let body_text = if chosen.randomize {
            randomize_text(&chosen.json)
        } else {
            chosen.json.clone()
        };
        let entry = TrafficEntry {
            ts: now_ts(),
            method: method.to_string(),
            path: full_path,
            status: chosen.status,
            ms: elapsed_ms(start),
            rule: Some(rule.id.clone()),
            variant: Some(chosen.id.clone()),
        };
        record_traffic(&profile_name, entry);
        return Response::builder()
            .status(StatusCode::from_u16(chosen.status).unwrap_or(StatusCode::OK))
            .header("Content-Type", "application/json")
            .body(Body::from(body_text))
            .unwrap();
    }

    if let Some(upstream) = &profile.upstream {
        return proxy_upstream(upstream, method, full_path, req).await;
    }

    let entry = TrafficEntry {
        ts: now_ts(),
        method: method.to_string(),
        path: full_path,
        status: 404,
        ms: elapsed_ms(start),
        rule: None,
        variant: None,
    };
    record_traffic(&profile_name, entry);
    json_error("no matching rule", StatusCode::NOT_FOUND)
}

async fn proxy_upstream(
    upstream: &str,
    method: Method,
    path: String,
    req: Request,
) -> Response {
    let client = Client::new();
    let url = format!("{upstream}{path}");
    let mut builder = client.request(method.clone(), &url);

    let headers = req.headers().clone();
    for (name, value) in headers.iter() {
        let key = name.as_str().to_ascii_lowercase();
        if !HOP_BY_HOP.contains(&key.as_str()) {
            builder = builder.header(name, value);
        }
    }

    let body_bytes = match to_bytes(req.into_body(), usize::MAX).await {
        Ok(b) => b,
        Err(_) => return json_error("read body", StatusCode::BAD_GATEWAY),
    };
    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes.to_vec());
    }

    let resp = match builder.send().await {
        Ok(r) => r,
        Err(e) => return json_error(&format!("upstream: {e}"), StatusCode::BAD_GATEWAY),
    };

    let status = resp.status();
    let resp_headers = resp.headers().clone();
    let body = match resp.bytes().await {
        Ok(b) => b,
        Err(_) => return json_error("read upstream body", StatusCode::BAD_GATEWAY),
    };

    let mut response = Response::builder().status(status);
    for (name, value) in resp_headers.iter() {
        let key = name.as_str().to_ascii_lowercase();
        if !HOP_BY_HOP.contains(&key.as_str()) && key != "content-length" {
            response = response.header(name, value);
        }
    }
    response.body(Body::from(body.to_vec())).unwrap()
}

fn now_ts() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn elapsed_ms(start: SystemTime) -> u64 {
    start.elapsed().unwrap_or_default().as_millis() as u64
}

fn json_error(msg: &str, status: StatusCode) -> Response {
    let body = serde_json::json!({ "error": msg }).to_string();
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap()
}

async fn profiles_handler() -> Json<Value> {
    let profiles = list_profiles().unwrap_or_default();
    Json(serde_json::json!({ "profiles": profiles }))
}

async fn profile_get_handler(Query(q): Query<NameQuery>) -> Response {
    match load_profile(&q.name) {
        Ok(p) => Json(serde_json::to_value(p).unwrap()).into_response(),
        Err(_) => json_error("not found", StatusCode::NOT_FOUND),
    }
}

async fn profile_post_handler(Json(profile): Json<Profile>) -> Response {
    if sanitize_name(&profile.name).is_err() {
        return json_error("invalid name", StatusCode::BAD_REQUEST);
    }
    if profile.listen_port == 0 {
        return json_error("invalid port", StatusCode::BAD_REQUEST);
    }
    if let Err(e) = save_profile(&profile) {
        return json_error(&e, StatusCode::BAD_REQUEST);
    }
    crate::control::log_action("mock", "profile.save", serde_json::json!({"name": profile.name, "port": profile.listen_port}));
    Json(serde_json::json!({ "ok": true })).into_response()
}

async fn profile_delete_handler(Json(body): Json<DeleteBody>) -> Response {
    let name = match sanitize_name(&body.name) {
        Ok(n) => n,
        Err(_) => return json_error("invalid name", StatusCode::BAD_REQUEST),
    };
    stop_server(&name).await;
    let path = profile_path(&name);
    let _ = fs::remove_file(path);
    crate::control::log_action("mock", "profile.delete", serde_json::json!({"name": name}));
    Json(serde_json::json!({ "ok": true })).into_response()
}

async fn start_handler(Json(body): Json<StartBody>) -> Response {
    let name = match sanitize_name(&body.name) {
        Ok(n) => n,
        Err(_) => return json_error("invalid name", StatusCode::BAD_REQUEST),
    };
    let profile = match load_profile(&name) {
        Ok(p) => p,
        Err(_) => return json_error("profile not found", StatusCode::BAD_REQUEST),
    };
    if running_servers().lock().unwrap().contains_key(&name) {
        return Json(serde_json::json!({ "ok": true, "port": profile.listen_port })).into_response();
    }
    let port = profile.listen_port;
    let name_clone = name.clone();
    // bind до ответа: 200 только если порт реально занят
    let listener = match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) => return json_error(&format!("bind {port}: {e}"), StatusCode::BAD_GATEWAY),
    };
    let handle = tokio::spawn(async move {
        run_mock_server(listener, name_clone).await;
    });
    crate::control::log_action("mock", "start", serde_json::json!({"name": name, "port": port}));
    running_servers().lock().unwrap().insert(name, handle);
    Json(serde_json::json!({ "ok": true, "port": port })).into_response()
}

async fn stop_handler(Json(body): Json<StartBody>) -> Response {
    let name = match sanitize_name(&body.name) {
        Ok(n) => n,
        Err(_) => return json_error("invalid name", StatusCode::BAD_REQUEST),
    };
    stop_server(&name).await;
    crate::control::log_action("mock", "stop", serde_json::json!({"name": name}));
    Json(serde_json::json!({ "ok": true })).into_response()
}

async fn stop_server(name: &str) {
    let mut servers = running_servers().lock().unwrap();
    if let Some(handle) = servers.remove(name) {
        handle.abort();
    }
}

async fn status_handler() -> Json<Value> {
    let servers = running_servers().lock().unwrap();
    let running: Vec<_> = servers
        .keys()
        .filter_map(|name| {
            load_profile(name).ok().map(|p| {
                serde_json::json!({ "name": name, "port": p.listen_port })
            })
        })
        .collect();
    Json(serde_json::json!({ "running": running }))
}

async fn traffic_handler(Query(q): Query<NameQuery>) -> Json<Value> {
    let traffic = get_traffic(&q.name);
    Json(serde_json::json!({ "traffic": traffic }))
}

async fn preview_handler(Json(body): Json<PreviewBody>) -> Json<Value> {
    let text = if body.randomize {
        randomize_text(&body.json)
    } else {
        body.json
    };
    Json(serde_json::json!({ "text": text }))
}

async fn run_mock_server(listener: TcpListener, name: String) {
    let removing = name.clone();
    let app = Router::new().fallback(move |req: Request| {
        let name = name.clone();
        async move { gateway_handler(State(name), req).await }
    });
    let _ = axum::serve(listener, app).await;
    running_servers().lock().unwrap().remove(&removing);
}

pub fn control_router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/api/mock/profiles", get(profiles_handler))
        .route("/api/mock/profile", get(profile_get_handler))
        .route("/api/mock/profile", post(profile_post_handler))
        .route("/api/mock/profile/delete", post(profile_delete_handler))
        .route("/api/mock/start", post(start_handler))
        .route("/api/mock/stop", post(stop_handler))
        .route("/api/mock/status", get(status_handler))
        .route("/api/mock/traffic", get(traffic_handler))
        .route("/api/mock/preview", post(preview_handler))
}
