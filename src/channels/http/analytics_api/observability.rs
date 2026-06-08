use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct ObservabilitySettingsResponse {
    pub enabled: bool,
    pub provider: String,
    pub endpoint: String,
    pub service_name: String,
    pub header_name: String,
    pub privacy_mode: String,
    pub auth_token_configured: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ObservabilitySettingsUpdate {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub service_name: Option<String>,
    #[serde(default)]
    pub header_name: Option<String>,
    #[serde(default)]
    pub privacy_mode: Option<String>,
    #[serde(default)]
    pub auth_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ObservabilityLogQuery {
    #[serde(default)]
    limit: Option<usize>,
}

pub(super) fn build_observability_settings_response(
    config: &crate::core::runtime::config::ObservabilityConfig,
    config_dir: &FsPath,
    data_dir: &FsPath,
) -> ObservabilitySettingsResponse {
    let auth_token_configured = crate::core::platform::observability::has_observability_auth_token(
        config_dir,
        Some(data_dir),
    )
    .unwrap_or(false);
    ObservabilitySettingsResponse {
        enabled: config.enabled,
        provider: crate::core::platform::observability::normalize_observability_provider(
            &config.provider,
        ),
        endpoint: config.endpoint.trim().to_string(),
        service_name: if config.service_name.trim().is_empty() {
            "agentark".to_string()
        } else {
            config.service_name.trim().to_string()
        },
        header_name: crate::core::platform::observability::normalize_observability_header_name(
            &config.header_name,
        ),
        privacy_mode: crate::core::platform::observability::normalize_observability_privacy_mode(
            &config.privacy_mode,
        ),
        auth_token_configured,
    }
}

pub(super) async fn get_observability_logs(
    State(state): State<AppState>,
    Query(query): Query<ObservabilityLogQuery>,
) -> Response {
    let (storage, config_dir) = {
        let agent = state.agent.read().await;
        (agent.storage.clone(), agent.config_dir.clone())
    };
    let limit = query.limit.unwrap_or(40).clamp(1, 120);
    let mut logs =
        crate::core::platform::observability::load_delivery_logs(&storage, &config_dir).await;
    if logs.len() > limit {
        logs.truncate(limit);
    }
    let issues = crate::core::platform::observability::summarize_log_issues(&logs);
    Json(serde_json::json!({
        "logs": logs,
        "issues": issues,
    }))
    .into_response()
}

pub(super) async fn test_observability_export(State(state): State<AppState>) -> Response {
    let (config, config_dir, data_dir, storage) = {
        let agent = state.agent.read().await;
        (
            agent.config.clone(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
            agent.storage.clone(),
        )
    };
    match crate::core::platform::observability::export_test_trace(
        &config,
        &config_dir,
        &data_dir,
        &storage,
    )
    .await
    {
        Ok(_) => Json(serde_json::json!({
            "status": "ok",
            "message": "Sent a test observability trace.",
        }))
        .into_response(),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: err.to_string(),
            }),
        )
            .into_response(),
    }
}
