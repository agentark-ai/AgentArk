//! Generic connector scaffold definitions used by runtime orchestration.
//! This is intentionally provider-agnostic and reusable by generated apps/actions.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::pipeline::RetryPolicy;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum HttpMethod {
    #[default]
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl HttpMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum PaginationMode {
    #[default]
    None,
    Page,
    Cursor,
}

fn default_page_param() -> String {
    "page".to_string()
}

fn default_cursor_param() -> String {
    "cursor".to_string()
}

fn default_items_path() -> String {
    "items".to_string()
}

fn default_next_cursor_path() -> String {
    "next_cursor".to_string()
}

fn default_start_page() -> u64 {
    1
}

fn default_max_pages() -> u64 {
    20
}

fn default_page_size_param() -> String {
    "limit".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationConfig {
    #[serde(default)]
    pub mode: PaginationMode,
    #[serde(default = "default_page_param")]
    pub page_param: String,
    #[serde(default = "default_cursor_param")]
    pub cursor_param: String,
    #[serde(default = "default_items_path")]
    pub items_path: String,
    #[serde(default = "default_next_cursor_path")]
    pub next_cursor_path: String,
    #[serde(default = "default_start_page")]
    pub start_page: u64,
    #[serde(default = "default_max_pages")]
    pub max_pages: u64,
    #[serde(default = "default_page_size_param")]
    pub page_size_param: String,
    #[serde(default)]
    pub page_size: Option<u64>,
}

impl Default for PaginationConfig {
    fn default() -> Self {
        Self {
            mode: PaginationMode::None,
            page_param: default_page_param(),
            cursor_param: default_cursor_param(),
            items_path: default_items_path(),
            next_cursor_path: default_next_cursor_path(),
            start_page: default_start_page(),
            max_pages: default_max_pages(),
            page_size_param: default_page_size_param(),
            page_size: None,
        }
    }
}

fn default_refresh_statuses() -> Vec<u16> {
    vec![401, 403]
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthRefreshConfig {
    /// Action to execute when auth expires (e.g. refresh_oauth_token).
    pub action: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
    #[serde(default = "default_refresh_statuses")]
    pub retry_statuses: Vec<u16>,
}

fn default_request_timeout_secs() -> u64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConnectorRequestSpec {
    pub url: String,
    #[serde(default)]
    pub method: HttpMethod,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub query: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Option<serde_json::Value>,
    #[serde(default)]
    pub pagination: PaginationConfig,
    #[serde(default)]
    pub retry: RetryPolicy,
    #[serde(default)]
    pub auth_refresh: Option<AuthRefreshConfig>,
    /// Min spacing between requests.
    #[serde(default)]
    pub rate_limit_ms: u64,
    #[serde(default = "default_request_timeout_secs")]
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorPageResult {
    pub request_url: String,
    pub status: u16,
    pub item_count: usize,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorRunResult {
    pub method: String,
    pub total_requests: usize,
    pub total_items: usize,
    pub pages: Vec<ConnectorPageResult>,
    pub items: Vec<serde_json::Value>,
}

pub fn json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut cursor = value;
    for segment in path.split('.') {
        let seg = segment.trim();
        if seg.is_empty() {
            continue;
        }
        if let Ok(idx) = seg.parse::<usize>() {
            cursor = cursor.as_array()?.get(idx)?;
            continue;
        }
        cursor = cursor.as_object()?.get(seg)?;
    }
    Some(cursor)
}

pub fn extract_items(value: &serde_json::Value, items_path: &str) -> Vec<serde_json::Value> {
    let Some(raw) = json_path(value, items_path) else {
        return vec![];
    };
    match raw {
        serde_json::Value::Array(items) => items.clone(),
        v => vec![v.clone()],
    }
}

pub fn extract_next_cursor(value: &serde_json::Value, path: &str) -> Option<String> {
    let raw = json_path(value, path)?;
    if let Some(s) = raw.as_str() {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        return Some(s.to_string());
    }
    if raw.is_null() {
        None
    } else {
        Some(raw.to_string())
    }
}
