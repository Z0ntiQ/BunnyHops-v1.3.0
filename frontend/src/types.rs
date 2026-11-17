//! types: вынесено из app.rs (атомарные модули, правило пользователя 13.11.2026).

use crate::logic::*;
use crate::ui::StepResult;

/* ---------- model ----------


#[derive(Clone, Debug, PartialEq)]
pub struct RespState {
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) content_type: String,
    pub(crate) final_url: String,
    pub(crate) body_text: String,
    pub(crate) bytes: u64,
    pub(crate) time_ms: f64,
    pub(crate) started_at: f64,
    pub(crate) is_textual: bool,
    pub(crate) streaming: bool,
    pub(crate) cancelled: bool,
    pub(crate) image_blob_url: Option<String>,
}
*/

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Step {
    pub id: String,
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub headers: Vec<KvRow>,
    #[serde(default)]
    pub capture_name: String,
    #[serde(default = "default_capture_path")]
    pub capture_path: String,
    // статус/результат live-прогона в localStorage не пишутся (как в React-версии)
    #[serde(skip, default = "default_status")]
    pub status: String,
    #[serde(skip, default)]
    pub result: Option<StepResult>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SavedReq {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) method: String,
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) headers: Vec<KvRow>,
    #[serde(default)]
    pub(crate) body: String,
    // слепок последнего ответа для переиспользования
    #[serde(default)]
    pub(crate) resp_status: u16,
    #[serde(default)]
    pub(crate) resp_body: String,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct ApiCall {
    pub(crate) id: String,
    pub(crate) method: String,
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) headers: Vec<KvRow>,
    #[serde(default)]
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) ts: u64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SavedGroup {
    pub(crate) id: String,
    pub(crate) num: u32,
    pub(crate) name: String,
    pub(crate) tabs: Vec<SavedReq>,
}

pub fn default_status() -> String {
    "idle".to_string()
}

pub fn default_capture_path() -> String {
    "$.token".to_string()
}

#[derive(Clone, Debug)]
pub struct RespState {
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) content_type: String,
    pub(crate) final_url: String,
    pub(crate) body_text: String,
    pub(crate) bytes: u64,
    pub(crate) time_ms: f64,
    pub(crate) started_at: f64,
    pub(crate) is_textual: bool,
    pub(crate) streaming: bool,
    pub(crate) cancelled: bool,
    pub(crate) image_blob_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LineData {
    pub(crate) hit: bool,
    pub(crate) has_error: bool,
    pub(crate) segs: Vec<Seg>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SearchData {
    pub(crate) total: usize,
    pub(crate) lines: Vec<LineData>,
}
