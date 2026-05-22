//! Security settings control plane.
//!
//! Exposes REST endpoints for reading and updating the runtime-mutable
//! pieces of `SecurityConfig`: the tool-argument guard whitelist and the
//! abuse-tracker thresholds. Operator decisions for pending security
//! escalations use dedicated abuse-review routes and also resolve the
//! existing `approval_log` entry.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use super::{AppState, error_response, spawn_security_log};
use crate::core::config::{AbuseTrackerConfig, SecurityConfig};
use crate::security::tool_args_guard::ToolArgsGuardConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SecuritySettingsPayload {
    pub tool_args: ToolArgsGuardConfig,
    pub abuse_tracker: AbuseTrackerConfig,
}

impl From<&SecurityConfig> for SecuritySettingsPayload {
    fn from(cfg: &SecurityConfig) -> Self {
        Self {
            tool_args: cfg.tool_args.clone(),
            abuse_tracker: cfg.abuse_tracker.clone(),
        }
    }
}

pub(super) async fn get_security_settings(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let payload = SecuritySettingsPayload::from(&agent.config.security);
    Json(payload).into_response()
}

pub(super) async fn update_security_settings(
    State(state): State<AppState>,
    Json(payload): Json<SecuritySettingsPayload>,
) -> Response {
    if payload.abuse_tracker.window_minutes == 0 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "abuse_tracker.window_minutes must be at least 1",
        );
    }
    if payload.abuse_tracker.trips_threshold == 0 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "abuse_tracker.trips_threshold must be at least 1",
        );
    }

    let mut agent = state.agent.write().await;
    agent.config.security = SecurityConfig {
        tool_args: payload.tool_args.clone(),
        abuse_tracker: payload.abuse_tracker.clone(),
    };
    agent
        .runtime
        .set_tool_args_guard_config(payload.tool_args.clone());
    if let Err(error) = agent.config.save(&agent.config_dir, Some(&agent.data_dir)) {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to save security settings: {}", error),
        );
    }

    let response = SecuritySettingsPayload::from(&agent.config.security);
    Json(response).into_response()
}

pub(super) async fn list_abuse_reviews(State(state): State<AppState>) -> Response {
    let (storage, config) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config.security.abuse_tracker.clone(),
        )
    };
    let tracker = crate::security::abuse_tracker::AbuseTracker::new(storage.db(), config);
    match tracker.list_reviews().await {
        Ok(reviews) => {
            let count = reviews.len();
            Json(serde_json::json!({
                "reviews": reviews,
                "count": count,
            }))
            .into_response()
        }
        Err(error) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Failed to list abuse reviews: {}", error),
        ),
    }
}

pub(super) async fn approve_abuse_review(
    State(state): State<AppState>,
    Path(source_key_hash): Path<String>,
) -> Response {
    decide_abuse_review(state, source_key_hash, true).await
}

pub(super) async fn reject_abuse_review(
    State(state): State<AppState>,
    Path(source_key_hash): Path<String>,
) -> Response {
    decide_abuse_review(state, source_key_hash, false).await
}

async fn decide_abuse_review(state: AppState, source_key_hash: String, approve: bool) -> Response {
    if !is_abuse_source_hash(&source_key_hash) {
        return error_response(StatusCode::BAD_REQUEST, "Invalid abuse-review source hash");
    }
    let (storage, config) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config.security.abuse_tracker.clone(),
        )
    };
    let tracker = crate::security::abuse_tracker::AbuseTracker::new(storage.db(), config);
    let outcome = if approve {
        tracker.approve_hash(&source_key_hash).await
    } else {
        tracker.reject_hash(&source_key_hash).await
    };
    match outcome {
        Ok(()) => {
            spawn_security_log(
                state.agent.clone(),
                "abuse_review_decision",
                if approve { "low" } else { "medium" },
                format!(
                    "Security abuse-review source {} by local_ui. source_key_hash={}",
                    if approve { "resumed" } else { "paused" },
                    source_key_hash
                ),
                Some(format!("source_key_hash={}", source_key_hash)),
            );
            Json(serde_json::json!({
                "status": "ok",
                "source_key_hash": source_key_hash,
                "decision": if approve { "approved" } else { "rejected" },
            }))
            .into_response()
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

fn is_abuse_source_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
