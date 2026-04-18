use super::*;

/// Actions response
#[derive(Debug, Serialize)]
struct ActionsResponse {
    pub skills: Vec<ActionInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<ActionInfo>>,
}

#[derive(Debug, Clone, Serialize)]
struct ActionInfo {
    pub name: String,
    pub description: String,
    pub version: String,
    pub source: String,
    pub editable: bool,
    pub enabled: bool,
    pub file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imported_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<ActionReviewInfo>,
}

#[derive(Debug, Clone, Serialize)]
struct ActionReviewInfo {
    pub status: String,
    pub ready: bool,
    pub allow_execute: bool,
    pub visible_in_catalog: bool,
    pub blocked_reason: Option<String>,
    pub risk_score_10: f32,
    pub risk_band: String,
    pub threat_level: String,
    pub warnings: Vec<String>,
    pub required_env: Vec<String>,
    pub missing_env: Vec<String>,
    pub permissions_needed: Vec<String>,
    pub requires_auth: bool,
    pub auth_configured: bool,
    pub notes: Vec<String>,
    pub reviewed_at: String,
}

/// Action content response
#[derive(Debug, Serialize)]
struct ActionContentResponse {
    pub name: String,
    pub content: String,
    pub editable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<ActionReviewInfo>,
}

/// Action content update request
#[derive(Debug, Deserialize)]
pub(super) struct ActionContentUpdate {
    pub content: String,
    #[serde(default)]
    pub force: bool,
}

/// Create action request
#[derive(Debug, Deserialize)]
pub(super) struct CreateActionRequest {
    pub name: String,
    pub content: String,
    /// Request install after non-blocking warnings. Blocking security findings still stop it.
    #[serde(default)]
    pub force: bool,
}

/// Import action from URL request
#[derive(Debug, Deserialize)]
pub(super) struct ImportActionRequest {
    pub url: String,
    /// Override the action name (otherwise derived from URL)
    #[serde(default)]
    pub name: Option<String>,
    /// Request install after non-blocking warnings. Blocking security findings still stop it.
    #[serde(default)]
    pub force: bool,
    /// Model to inject into frontmatter (e.g. "anthropic/claude-sonnet-4-20250514")
    #[serde(default)]
    pub model: Option<String>,
    /// If true, only analyze/preview security + required secrets without saving the skill.
    #[serde(default)]
    pub preview_only: bool,
    /// Optional explicit list of raw skill URLs to import as one bulk request.
    /// Used by Bulk Import confirmation flow after previewing a collection URL.
    #[serde(default)]
    pub selected_urls: Option<Vec<String>>,
}

fn action_review_status_label(status: &crate::runtime::ActionReviewStatus) -> String {
    match status {
        crate::runtime::ActionReviewStatus::Ready => "ready",
        crate::runtime::ActionReviewStatus::NeedsSecrets => "needs_secrets",
        crate::runtime::ActionReviewStatus::Blocked => "blocked",
        crate::runtime::ActionReviewStatus::Warning => "warning",
        crate::runtime::ActionReviewStatus::Unreviewed => "unreviewed",
    }
    .to_string()
}

fn action_review_info(review: crate::runtime::ActionReviewSnapshot) -> ActionReviewInfo {
    ActionReviewInfo {
        status: action_review_status_label(&review.status),
        ready: review.ready,
        allow_execute: review.allow_execute,
        visible_in_catalog: review.visible_in_catalog,
        blocked_reason: review.blocked_reason,
        risk_score_10: review.risk_score_10,
        risk_band: review.risk_band,
        threat_level: review.threat_level,
        warnings: review.warnings,
        required_env: review.required_env,
        missing_env: review.missing_env,
        permissions_needed: review.permissions_needed,
        requires_auth: review.requires_auth,
        auth_configured: review.auth_configured,
        notes: review.notes,
        reviewed_at: review.reviewed_at,
    }
}

/// List available actions
pub(super) async fn list_actions(State(state): State<AppState>) -> Response {
    let agent_guard = state.agent.read().await;
    let result = agent_guard.runtime.list_actions().await;
    match result {
        Ok(actions) => {
            let mut action_infos: Vec<ActionInfo> = Vec::with_capacity(actions.len());
            for s in actions {
                use crate::actions::ActionSource;
                let source_str = match &s.source {
                    ActionSource::System => "system",
                    ActionSource::Bundled => "bundled",
                    ActionSource::Custom => "custom",
                };
                // Custom and Bundled actions are editable (Bundled gets copied to custom on edit)
                // Only System actions are read-only
                let editable = s.source != ActionSource::System;
                let enabled = agent_guard.runtime.is_action_enabled(&s.name).await;
                let review = match agent_guard
                    .runtime
                    .refresh_action_review_state(&s.name)
                    .await
                {
                    Ok(Some(review)) => Some(action_review_info(review)),
                    Ok(None) | Err(_) => agent_guard
                        .runtime
                        .get_action_review(&s.name)
                        .await
                        .map(action_review_info),
                };
                let imported_at = s.file_path.as_deref().and_then(|p| {
                    std::fs::metadata(p)
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .map(|t| {
                            chrono::DateTime::<chrono::Utc>::from(t)
                                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
                        })
                });
                action_infos.push(ActionInfo {
                    name: s.name,
                    description: s.description,
                    version: s.version,
                    source: source_str.to_string(),
                    editable,
                    enabled,
                    file_path: s.file_path,
                    imported_at,
                    review,
                });
            }

            (
                StatusCode::OK,
                Json(ActionsResponse {
                    skills: action_infos.clone(),
                    actions: Some(action_infos),
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// Get action content (for editing)
pub(super) async fn get_action_content(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Response {
    let agent_guard = state.agent.read().await;

    // Get action info and content from runtime
    match agent_guard.runtime.get_action_content(&name).await {
        Ok(Some((info, content))) => {
            use crate::actions::ActionSource;
            // Custom and Bundled actions are editable (Bundled gets copied to custom on edit)
            let editable = info.source != ActionSource::System;
            let review = match agent_guard.runtime.refresh_action_review_state(&name).await {
                Ok(Some(review)) => Some(action_review_info(review)),
                Ok(None) | Err(_) => agent_guard
                    .runtime
                    .get_action_review(&name)
                    .await
                    .map(action_review_info),
            };
            (
                StatusCode::OK,
                Json(ActionContentResponse {
                    name: info.name,
                    content,
                    editable,
                    review,
                }),
            )
                .into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Skill '{}' not found", name),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// Update action content
pub(super) async fn update_action_content(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(update): Json<ActionContentUpdate>,
) -> Response {
    let agent_guard = state.agent.read().await;

    let existing = match agent_guard.runtime.get_action_content(&name).await {
        Ok(Some((info, _))) => info,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Skill '{}' not found", name),
                }),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response();
        }
    };

    if existing.source == crate::actions::ActionSource::System {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Skill is not editable (system skill)".to_string(),
            }),
        )
            .into_response();
    }
    if let Some(declared_name) = extract_skill_name_from_frontmatter(&update.content) {
        if declared_name != name {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Skill frontmatter name '{}' must match update target '{}'",
                        declared_name, name
                    ),
                }),
            )
                .into_response();
        }
    }

    let semantic_review = crate::security::skill_review::review_skill_import_with_configured_model(
        &agent_guard.llm,
        &agent_guard.config_dir,
        &format!("local://skills/{}", name),
        &name,
        &update.content,
    )
    .await;
    let blocked = semantic_review.policy.blocked;
    if blocked {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "blocked",
                "message": format!("Skill '{}' update blocked by semantic security policy", name),
                "review": agent_guard
                    .runtime
                    .get_action_review(&name)
                    .await
                    .map(action_review_info),
                "security": semantic_review_security_json(&semantic_review)
            })),
        )
            .into_response();
    }

    match agent_guard
        .runtime
        .update_semantically_reviewed_action(&name, &update.content, &semantic_review, update.force)
        .await
    {
        Ok(Some(review)) => {
            let needs_secrets = !review.missing_env.is_empty();
            let review = action_review_info(review);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": if needs_secrets { "needs_secrets" } else { "ok" },
                    "message": if needs_secrets {
                        format!("Skill '{}' updated, but needs secrets configured", name)
                    } else {
                        format!("Skill '{}' updated", name)
                    },
                    "review": review,
                    "security": semantic_review_security_json(&semantic_review)
                })),
            )
                .into_response()
        }
        Ok(None) => (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Skill is not editable (system skill)".to_string(),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// Enable/disable an action (non-destructive).
pub(super) async fn set_action_enabled_endpoint(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(req): Json<ActionEnabledRequest>,
) -> Response {
    let agent_guard = state.agent.read().await;
    match agent_guard
        .runtime
        .set_action_enabled(&name, req.enabled)
        .await
    {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ok","name":name,"enabled":req.enabled})),
        )
            .into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Skill not found or not configurable".to_string(),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

#[derive(Debug, Serialize)]
struct ActionSecretsResponse {
    required_env: Vec<String>,
    missing_env: Vec<String>,
    bindings: std::collections::HashMap<String, String>,
    configured: std::collections::HashMap<String, bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ActionSecretsUpdateRequest {
    secrets: Vec<ActionSecretUpdate>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ActionTestRequest {
    #[serde(default)]
    arguments: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone)]
struct ActionSecretUpdate {
    /// The required env name from the action (e.g. "OPENAI_API_KEY")
    env: String,
    /// Optional storage key name to bind to (e.g. "OPENAI_API_KEY_2") or "builtin"
    #[serde(default)]
    store_as: Option<String>,
    /// Optional value (if storing encrypted); omit when store_as="builtin"
    #[serde(default)]
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ActionEnabledRequest {
    enabled: bool,
}

fn split_frontmatter_block(content: &str) -> Option<(&str, &str)> {
    let body = content
        .strip_prefix("---\r\n")
        .or_else(|| content.strip_prefix("---\n"))?;
    let mut consumed = 0usize;
    for segment in body.split_inclusive('\n') {
        let line = segment.trim_end_matches(&['\r', '\n'][..]);
        if line == "---" {
            let rest_start = consumed + segment.len();
            return Some((&body[..consumed], &body[rest_start..]));
        }
        consumed += segment.len();
    }
    None
}

fn extract_frontmatter_text(content: &str) -> Option<&str> {
    split_frontmatter_block(content).map(|(frontmatter, _)| frontmatter)
}

fn extract_skill_name_from_frontmatter(content: &str) -> Option<String> {
    let frontmatter = extract_frontmatter_text(content)?;
    let parsed = serde_yaml::from_str::<serde_yaml::Value>(frontmatter).ok()?;
    let mapping = parsed.as_mapping()?;
    for (key, value) in mapping {
        if key.as_str() == Some("name") {
            return value
                .as_str()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string);
        }
    }
    None
}

fn unique_push(out: &mut Vec<String>, s: String) {
    if !out.iter().any(|x| x == &s) {
        out.push(s);
    }
}

fn is_known_skill_catalog_host(host: &str) -> bool {
    host == "clawhub.ai"
        || host.ends_with(".clawhub.ai")
        || host == "openclaw.ai"
        || host.ends_with(".openclaw.ai")
}

fn extract_required_envs_from_frontmatter(frontmatter: &str) -> Vec<String> {
    let mut envs: Vec<String> = Vec::new();
    if let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(frontmatter) {
        collect_required_envs_from_yaml(&value, &mut envs);
    }

    envs
}

fn collect_required_envs_from_yaml(value: &serde_yaml::Value, envs: &mut Vec<String>) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            for value in map.values() {
                collect_required_envs_from_yaml(value, envs);
            }
        }
        serde_yaml::Value::Sequence(items) => {
            for item in items {
                collect_required_envs_from_yaml(item, envs);
            }
        }
        serde_yaml::Value::String(text) => {
            for item in split_env_candidate_text(text) {
                if item.contains('_') && is_env_var_style_key(&item) {
                    unique_push(envs, item);
                }
            }
        }
        _ => {}
    }
}

fn split_env_candidate_text(text: &str) -> Vec<String> {
    text.split(|ch: char| ch == ',' || ch.is_whitespace() || ch == '[' || ch == ']')
        .map(|item| item.trim().trim_matches('"').trim_matches('\''))
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn import_finding_json(
    finding: &crate::security::action_guard::AnalysisFinding,
) -> serde_json::Value {
    serde_json::json!({
        "category": format!("{:?}", finding.category),
        "label": finding.import_label(),
        "description": finding.description,
        "explanation": finding.import_explanation(),
        "matched_text": finding.matched_text,
        "line": finding.line_number,
        "file": finding.file_path,
        "severity": finding.severity,
        "contextual": finding.is_contextual_import_signal(),
    })
}

fn semantic_review_security_json(
    review: &crate::security::skill_review::SemanticSkillReview,
) -> serde_json::Value {
    serde_json::json!({
        "threat_level": format!("{:?}", review.policy.threat_level),
        "warnings": review.policy.warnings.clone(),
        "findings": review.policy.findings.iter().map(import_finding_json).collect::<Vec<_>>(),
        "blocked": review.policy.blocked,
        "total_severity": review.policy.total_severity,
        "risk_score_10": review.policy.risk_score_10,
        "risk_band": review.policy.risk_band.clone(),
        "total_findings": review.policy.findings.len(),
        "contextual_findings": 0_usize,
        "capabilities": review.capabilities.clone(),
        "matched_rules": review.policy.matched_rules.clone(),
        "review_model": review.model.clone(),
        "review_summary": review.summary.clone(),
    })
}

fn builtin_env_from_agent_config(cfg: &crate::core::config::AgentConfig, env: &str) -> bool {
    let mut providers: Vec<&crate::core::LlmProvider> = vec![&cfg.llm];
    if let Some(fb) = cfg.llm_fallback.as_ref() {
        providers.push(fb);
    }
    for slot in &cfg.model_pool.slots {
        if slot.enabled {
            providers.push(&slot.provider);
        }
    }
    match env {
        "OPENAI_API_KEY" => providers.into_iter().any(|p| matches!(p, crate::core::LlmProvider::OpenAI { api_key, .. } if !api_key.is_empty())),
        "OPENROUTER_API_KEY" => providers.into_iter().any(|p| matches!(p, crate::core::LlmProvider::OpenAI { api_key, base_url, .. } if !api_key.is_empty() && base_url.as_deref().unwrap_or("").contains("openrouter"))),
        "ANTHROPIC_API_KEY" => providers.into_iter().any(|p| matches!(p, crate::core::LlmProvider::Anthropic { api_key, .. } if !api_key.is_empty())),
        _ => false,
    }
}

fn legacy_env_alias_configured(
    custom: &std::collections::HashMap<String, String>,
    env: &str,
) -> bool {
    let legacy_key = match env {
        "GITHUB_TOKEN" => Some("github_token"),
        "NOTION_TOKEN" => Some("notion_token"),
        "TWITTER_BEARER_TOKEN" => Some("twitter_bearer_token"),
        "ONEPASSWORD_TOKEN" => Some("onepassword_token"),
        "GOOGLE_PLACES_API_KEY" => Some("google_places_api_key"),
        "TWILIO_AUTH_TOKEN" => Some("twilio_auth_token"),
        "TWILIO_ACCOUNT_SID" => Some("twilio_account_sid"),
        "GARMIN_TOKEN" => Some("garmin_token"),
        "GARMIN_API_BASE" => Some("garmin_api_base"),
        "WHOOP_TOKEN" => Some("whoop_token"),
        "GA4_ACCESS_TOKEN" => Some("ga4_access_token"),
        "GA4_PROPERTY_ID" => Some("ga4_property_id"),
        "GSC_ACCESS_TOKEN" => Some("gsc_access_token"),
        "GSC_SITE_URL" => Some("gsc_site_url"),
        "SOCIAL_TWITTER_BEARER_TOKEN" => Some("social_twitter_bearer_token"),
        "SOCIAL_GA4_ACCESS_TOKEN" => Some("social_ga4_access_token"),
        "SOCIAL_GA4_PROPERTY_ID" => Some("social_ga4_property_id"),
        "HOMEY_TOKEN" => Some("homey_token"),
        "HOMEY_API_BASE" => Some("homey_api_base"),
        "FASTMAIL_TOKEN" => Some("fastmail_token"),
        "BEEPER_TOKEN" => Some("beeper_token"),
        "BEEPER_API_BASE" => Some("beeper_api_base"),
        _ => None,
    };
    legacy_key
        .and_then(|k| custom.get(k))
        .is_some_and(|v| !v.trim().is_empty())
}

fn env_is_configured_for_action(
    agent_cfg: &crate::core::config::AgentConfig,
    custom: &std::collections::HashMap<String, String>,
    action_name: &str,
    env: &str,
) -> bool {
    let binding_key = format!("action_envmap:{}:{}", action_name, env);
    let target = custom.get(&binding_key).map(|s| s.as_str()).unwrap_or(env);

    if target == "builtin" {
        return builtin_env_from_agent_config(agent_cfg, env);
    }

    // Allow both modern and legacy secret key storage formats.
    if crate::core::secrets::has_user_secret(custom, target) {
        return true;
    }

    // Compatibility with existing integration secret names.
    if legacy_env_alias_configured(custom, env) {
        return true;
    }

    // If it's a well-known provider env, we can satisfy it via the configured models.
    builtin_env_from_agent_config(agent_cfg, env)
}

pub(super) async fn get_action_secrets(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    let (config_dir, data_dir) = (agent.config_dir.clone(), agent.data_dir.clone());
    let mgr = match crate::core::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    ) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Config error: {}", e),
                }),
            )
                .into_response();
        }
    };
    let secrets = match mgr.load_secrets() {
        Ok(secrets) => secrets,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to load encrypted secrets: {}", e),
                }),
            )
                .into_response();
        }
    };
    let custom = &secrets.custom;

    let content = match agent.runtime.get_action_content(&name).await {
        Ok(Some((_info, c))) => c,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Skill '{}' not found", name),
                }),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response();
        }
    };

    let required_env = extract_frontmatter_text(&content)
        .map(extract_required_envs_from_frontmatter)
        .unwrap_or_default();

    let mut configured: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    let mut bindings: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut missing_env: Vec<String> = Vec::new();

    for env in &required_env {
        let binding_key = format!("action_envmap:{}:{}", name, env);
        if let Some(b) = custom.get(&binding_key) {
            bindings.insert(env.clone(), b.clone());
        }
        let ok = env_is_configured_for_action(&agent.config, custom, &name, env);
        configured.insert(env.clone(), ok);
        if !ok {
            missing_env.push(env.clone());
        }
    }

    (
        StatusCode::OK,
        Json(ActionSecretsResponse {
            required_env,
            missing_env,
            bindings,
            configured,
        }),
    )
        .into_response()
}

pub(super) async fn set_action_secrets(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(request): Json<ActionSecretsUpdateRequest>,
) -> Response {
    let agent = state.agent.read().await;
    // Validate target action exists before mutating secrets.
    match agent.runtime.get_action_content(&name).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: format!("Skill '{}' not found", name),
                }),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response();
        }
    }

    for item in &request.secrets {
        let env = item.env.trim();
        if !is_env_var_style_key(env) {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid env name '{}'", env),
                }),
            )
                .into_response();
        }

        let store_as = item.store_as.as_deref().unwrap_or(env).trim();
        if store_as != "builtin"
            && (store_as.is_empty()
                || store_as.len() > 128
                || store_as.chars().any(|c| c.is_whitespace())
                || !is_env_var_style_key(store_as))
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid store_as for '{}'", env),
                }),
            )
                .into_response();
        }
    }

    let (config_dir, data_dir) = (agent.config_dir.clone(), agent.data_dir.clone());
    drop(agent);
    let action_name = name.clone();
    let updates = request.secrets.clone();
    let write_result = tokio::task::spawn_blocking(move || -> Result<()> {
        let mgr = crate::core::config::SecureConfigManager::new_with_data_dir(
            &config_dir,
            Some(&data_dir),
        )?;
        mgr.update_custom_secrets(|custom| {
            for item in &updates {
                let env = item.env.trim();
                let store_as = item.store_as.as_deref().unwrap_or(env).trim();
                if store_as == "builtin" {
                    let binding_key = format!("action_envmap:{}:{}", action_name, env);
                    custom.insert(binding_key, "builtin".to_string());
                    continue;
                }

                // Store encrypted value (optional) and bind env -> store_as (per action).
                let env_key = format!("env:{}", store_as);
                let secret_key = format!("secret:{}", store_as);
                if let Some(val) = item
                    .value
                    .as_ref()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                {
                    custom.insert(env_key, val);
                } else {
                    let has_env_value = custom.get(&env_key).is_some_and(|v| !v.trim().is_empty());
                    let has_secret_value = custom
                        .get(&secret_key)
                        .is_some_and(|v| !v.trim().is_empty());
                    if !has_env_value && !has_secret_value {
                        return Err(anyhow::anyhow!(
                            "[BAD_REQUEST] Missing value for '{}' (and '{}' is not already stored)",
                            env,
                            store_as
                        ));
                    }
                }

                let binding_key = format!("action_envmap:{}:{}", action_name, env);
                custom.insert(binding_key, store_as.to_string());
            }
            Ok(())
        })?;
        Ok(())
    })
    .await;
    match write_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let msg = e.to_string();
            if let Some(rest) = msg.strip_prefix("[BAD_REQUEST] ") {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: rest.to_string(),
                    }),
                )
                    .into_response();
            }
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to store secrets: {}", msg),
                }),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Secret update task failed: {}", e),
                }),
            )
                .into_response();
        }
    }

    let agent = state.agent.read().await;
    match agent.runtime.refresh_action_review_state(&name).await {
        Ok(Some(review)) if review.allow_execute => {
            let _ = agent.runtime.set_action_enabled(&name, true).await;
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(
                "Failed to refresh security review state for '{}' after saving secrets: {}",
                name,
                error
            );
        }
    }
    drop(agent);

    // Return fresh status
    get_action_secrets(State(state), Path(name)).await
}

pub(super) async fn test_action(
    State(state): State<AppState>,
    maybe_caller: Option<Extension<crate::actions::ActionCallerPrincipal>>,
    Path(name): Path<String>,
    Json(request): Json<ActionTestRequest>,
) -> Response {
    let arguments = if request.arguments.is_null() {
        serde_json::json!({})
    } else {
        request.arguments
    };

    let result = {
        let agent = state.agent.read().await;
        let caller = maybe_caller.as_ref().map(|Extension(value)| value);
        match agent
            .runtime
            .execute_action_with_context(
                &name,
                &arguments,
                &super::build_direct_action_auth_context(
                    caller,
                    crate::actions::ActionExecutionSurface::Api,
                    true,
                ),
            )
            .await
        {
            Ok(output) => {
                if let Some(payload) = crate::runtime::parse_workflow_missing_inputs_marker(&output)
                {
                    let message = if payload.missing.is_empty() {
                        format!("Skill '{}' requires additional input.", payload.action)
                    } else {
                        format!(
                            "Skill '{}' needs required input(s): {}",
                            payload.action,
                            payload.missing.join(", ")
                        )
                    };
                    Ok(serde_json::json!({
                        "status": "needs_input",
                        "mode": "workflow",
                        "action": name.clone(),
                        "arguments": arguments.clone(),
                        "missing_inputs": payload.missing,
                        "required_inputs": payload.required,
                        "message": message
                    }))
                } else if let Some((workflow_action_name, user_query)) =
                    crate::runtime::parse_workflow_action_marker(&output)
                {
                    match agent
                        .runtime
                        .get_workflow_content(&workflow_action_name)
                        .await
                    {
                        Some(workflow_content) => {
                            match agent
                                .runtime
                                .execute_workflow_action(
                                    &workflow_action_name,
                                    &workflow_content,
                                    &user_query,
                                    &agent.llm,
                                )
                                .await
                            {
                                Ok(workflow_output) => Ok(serde_json::json!({
                                    "status": "ok",
                                    "mode": "workflow",
                                    "action": name.clone(),
                                    "arguments": arguments.clone(),
                                    "output": workflow_output
                                })),
                                Err(e) => Err(e),
                            }
                        }
                        None => Err(anyhow::anyhow!(
                            "Workflow content not found for action: {}",
                            workflow_action_name
                        )),
                    }
                } else {
                    Ok(serde_json::json!({
                        "status": "ok",
                        "mode": "native",
                        "action": name.clone(),
                        "arguments": arguments.clone(),
                        "output": output
                    }))
                }
            }
            Err(e) => Err(e),
        }
    };

    match result {
        Ok(output) => (StatusCode::OK, Json(output)).into_response(),
        Err(e) => {
            let error = e.to_string();
            let status = if error.to_ascii_lowercase().contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            (
                status,
                Json(serde_json::json!({
                    "status": "error",
                    "action": name,
                    "arguments": arguments,
                    "error": error
                })),
            )
                .into_response()
        }
    }
}

/// Create a new action with security verification
pub(super) async fn create_action(
    State(state): State<AppState>,
    Json(request): Json<CreateActionRequest>,
) -> Response {
    // Validate action name
    if !request
        .name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Skill name must contain only lowercase letters, numbers, and hyphens"
                    .to_string(),
            }),
        )
            .into_response();
    }

    let agent_guard = state.agent.read().await;
    if let Some(declared_name) = extract_skill_name_from_frontmatter(&request.content) {
        if declared_name != request.name {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Skill frontmatter name '{}' must match requested name '{}'",
                        declared_name, request.name
                    ),
                }),
            )
                .into_response();
        }
    }

    let semantic_review = crate::security::skill_review::review_skill_import_with_configured_model(
        &agent_guard.llm,
        &agent_guard.config_dir,
        "local://skills",
        &request.name,
        &request.content,
    )
    .await;
    let blocked = semantic_review.policy.blocked;
    let mut status = if blocked { "blocked" } else { "ok" };

    let required_env = extract_frontmatter_text(&request.content)
        .map(extract_required_envs_from_frontmatter)
        .unwrap_or_default();
    let (missing_env, bindings) = if !required_env.is_empty() {
        let secrets = match crate::core::config::SecureConfigManager::new_with_data_dir(
            &agent_guard.config_dir,
            Some(&agent_guard.data_dir),
        )
        .and_then(|mgr| mgr.load_secrets())
        {
            Ok(secrets) => secrets,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to load encrypted secrets: {}", e),
                    }),
                )
                    .into_response();
            }
        };
        let custom = &secrets.custom;
        let mut missing = Vec::new();
        let mut bindings: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for env in &required_env {
            let binding_key = format!("action_envmap:{}:{}", request.name, env);
            if let Some(b) = custom.get(&binding_key) {
                bindings.insert(env.clone(), b.clone());
            }
            if !env_is_configured_for_action(&agent_guard.config, custom, &request.name, env) {
                missing.push(env.clone());
            }
        }
        (missing, bindings)
    } else {
        (Vec::new(), std::collections::HashMap::new())
    };

    if status == "ok" && !missing_env.is_empty() {
        status = "needs_secrets";
    }

    let mut persisted_review = None;
    if !blocked {
        match agent_guard
            .runtime
            .install_semantically_reviewed_action(
                &request.name,
                &request.content,
                &semantic_review,
                request.force,
            )
            .await
        {
            Ok(review) => {
                if !review.missing_env.is_empty() {
                    status = "needs_secrets";
                }
                persisted_review = Some(action_review_info(review));
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: e.to_string(),
                    }),
                )
                    .into_response();
            }
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": status,
            "message": if blocked {
                format!("Skill '{}' blocked by semantic security policy", request.name)
            } else if status == "needs_secrets" {
                format!("Skill '{}' created, but needs secrets configured", request.name)
            } else {
                format!("Skill '{}' created", request.name)
            },
            "review": persisted_review,
            "secrets": {
                "required_env": required_env,
                "missing_env": missing_env,
                "bindings": bindings,
            },
            "security": semantic_review_security_json(&semantic_review)
        })),
    )
        .into_response()
}

fn normalize_model_identifier(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let candidate = if trimmed.contains('|') {
        let mut parts = trimmed.split('|');
        let provider = parts.next().unwrap_or("").trim();
        let model = parts.next().unwrap_or("").trim();
        if provider.is_empty() || model.is_empty() {
            return None;
        }
        format!("{}/{}", provider, model)
    } else {
        trimmed.to_string()
    };

    let (provider, model) = candidate.split_once('/')?;
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    if candidate.chars().any(|c| c.is_whitespace()) {
        return None;
    }
    Some(candidate)
}

/// Inject a model field into SKILL.md YAML frontmatter
fn inject_model_into_frontmatter(content: &str, model: &str) -> String {
    fn render_model_frontmatter(frontmatter: &str, model: &str) -> String {
        let mut mapping = serde_yaml::from_str::<serde_yaml::Value>(frontmatter)
            .ok()
            .and_then(|value| value.as_mapping().cloned())
            .unwrap_or_default();
        mapping.insert(
            serde_yaml::Value::String("model".to_string()),
            serde_yaml::Value::String(model.trim().to_string()),
        );
        let rendered = serde_yaml::to_string(&serde_yaml::Value::Mapping(mapping))
            .unwrap_or_else(|_| format!("model: {:?}", model.trim()));
        rendered
            .trim_start_matches("---\n")
            .trim_end_matches("...\n")
            .trim_end()
            .to_string()
    }

    if let Some((frontmatter, rest)) = split_frontmatter_block(content) {
        let rendered = render_model_frontmatter(frontmatter, model);
        return format!("---\n{}\n---\n{}", rendered, rest);
    }
    let rendered = render_model_frontmatter("", model);
    format!("---\n{}\n---\n\n{}", rendered, content)
}

const SKILL_IMPORT_MAX_BYTES: usize = 2 * 1024 * 1024;
const GITHUB_SKILL_ARCHIVE_MAX_BYTES: usize = 16 * 1024 * 1024;
const GITHUB_SKILL_ARCHIVE_MAX_ENTRIES: usize = 20_000;

async fn validate_import_fetch_url(raw: &str) -> Result<reqwest::Url, String> {
    crate::core::net::validate_public_https_url(raw)
        .await
        .map_err(|error| error.to_string())
}

async fn fetch_text_with_redirects(
    client: &reqwest::Client,
    initial: reqwest::Url,
    max_redirects: usize,
) -> Result<String, String> {
    let mut current = initial;
    for _ in 0..=max_redirects {
        let resp = client
            .get(current.clone())
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
            if bytes.len() > SKILL_IMPORT_MAX_BYTES {
                return Err("Import content too large".to_string());
            }
            return Ok(String::from_utf8_lossy(&bytes).to_string());
        }
        if resp.status().is_redirection() {
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| format!("HTTP {} (missing Location)", resp.status()))?;
            let next = current
                .join(location)
                .map_err(|e| format!("Invalid redirect URL: {}", e))?;
            current = validate_import_fetch_url(next.as_str()).await?;
            continue;
        }
        return Err(format!("HTTP {}", resp.status()));
    }
    Err("Too many redirects".to_string())
}

async fn fetch_bytes_with_redirects(
    client: &reqwest::Client,
    initial: reqwest::Url,
    max_redirects: usize,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let mut current = initial;
    for _ in 0..=max_redirects {
        let resp = client
            .get(current.clone())
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            if let Some(len) = resp.content_length() {
                if len > max_bytes as u64 {
                    return Err(format!(
                        "Response too large ({} bytes > {} limit)",
                        len, max_bytes
                    ));
                }
            }
            let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
            if bytes.len() > max_bytes {
                return Err(format!(
                    "Response too large ({} bytes > {} limit)",
                    bytes.len(),
                    max_bytes
                ));
            }
            return Ok(bytes.to_vec());
        }
        if resp.status().is_redirection() {
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| format!("HTTP {} (missing Location)", resp.status()))?;
            let next = current
                .join(location)
                .map_err(|e| format!("Invalid redirect URL: {}", e))?;
            current = validate_import_fetch_url(next.as_str()).await?;
            continue;
        }
        return Err(format!("HTTP {}", resp.status()));
    }
    Err("Too many redirects".to_string())
}

#[derive(Debug, Clone)]
struct GitHubLocation {
    owner: String,
    repo: String,
    git_ref: Option<String>,
    path: String,
    directory_hint: bool,
}

#[derive(Debug, Deserialize)]
struct GitHubContentItem {
    #[serde(rename = "type")]
    item_type: String,
    name: String,
    path: String,
    download_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum GitHubContentsResponse {
    One(GitHubContentItem),
    Many(Vec<GitHubContentItem>),
}

fn parse_github_location(raw: &str) -> Option<GitHubLocation> {
    let parsed = reqwest::Url::parse(raw).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    if host != "github.com" && host != "www.github.com" {
        return None;
    }

    let parts: Vec<String> = parsed
        .path_segments()
        .map(|s| {
            s.filter(|p| !p.trim().is_empty())
                .map(|p| p.to_string())
                .collect()
        })
        .unwrap_or_default();
    if parts.len() < 2 {
        return None;
    }

    let owner = parts[0].trim().to_string();
    let repo = parts[1].trim().trim_end_matches(".git").to_string();
    let mut git_ref: Option<String> = None;
    let mut path = String::new();
    let mut directory_hint = true;

    if parts.len() >= 4 && (parts[2] == "tree" || parts[2] == "blob") {
        git_ref = Some(parts[3].trim().to_string());
        if parts.len() > 4 {
            path = parts[4..].join("/");
        }
        let ends_with_md = path.to_ascii_lowercase().ends_with(".md");
        directory_hint = parts[2] == "tree" || !ends_with_md;
    } else if parts.len() > 2 {
        path = parts[2..].join("/");
        let lower = path.to_ascii_lowercase();
        directory_hint = !(lower.ends_with("skill.md") || lower.ends_with("action.md"));
    }

    if owner.is_empty() || repo.is_empty() {
        return None;
    }

    Some(GitHubLocation {
        owner,
        repo,
        git_ref,
        path: path.trim_matches('/').to_string(),
        directory_hint,
    })
}

async fn fetch_github_contents(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    git_ref: &str,
    path: &str,
    token: Option<&str>,
) -> Result<Vec<GitHubContentItem>, String> {
    let trimmed = path.trim_matches('/');
    let api_url = if trimmed.is_empty() {
        format!(
            "https://api.github.com/repos/{}/{}/contents?ref={}",
            owner, repo, git_ref
        )
    } else {
        format!(
            "https://api.github.com/repos/{}/{}/contents/{}?ref={}",
            owner, repo, trimmed, git_ref
        )
    };
    let validated = validate_import_fetch_url(&api_url).await?;
    let mut req = client
        .get(validated)
        .header(
            reqwest::header::USER_AGENT,
            crate::branding::versioned_user_agent(),
        )
        .header(reqwest::header::ACCEPT, "application/vnd.github+json");
    if let Some(tok) = token {
        req = req.header(reqwest::header::AUTHORIZATION, format!("Bearer {}", tok));
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API HTTP {}", resp.status()));
    }

    let body = resp.text().await.map_err(|e| e.to_string())?;
    let parsed: GitHubContentsResponse =
        serde_json::from_str(&body).map_err(|e| format!("GitHub API parse error: {}", e))?;
    Ok(match parsed {
        GitHubContentsResponse::One(item) => vec![item],
        GitHubContentsResponse::Many(items) => items,
    })
}

async fn collect_github_skill_urls(
    client: &reqwest::Client,
    request: &GitHubSkillUrlRequest<'_>,
) -> Result<Vec<String>, String> {
    let owner = request.owner;
    let repo = request.repo;
    let git_ref = request.git_ref;
    let root_path = request.root_path;
    let max_depth = request.max_depth;
    let max_files = request.max_files;
    let token = request.token;

    let mut found: Vec<String> = Vec::new();
    let mut dedupe: HashSet<String> = HashSet::new();
    let mut stack: Vec<(String, usize)> = vec![(root_path.trim_matches('/').to_string(), 0)];

    while let Some((path, depth)) = stack.pop() {
        let entries = fetch_github_contents(client, owner, repo, git_ref, &path, token).await?;
        for entry in entries {
            if entry.item_type == "file" {
                let lower = entry.name.to_ascii_lowercase();
                if lower == "skill.md" {
                    let raw = entry.download_url.unwrap_or_else(|| {
                        format!(
                            "https://raw.githubusercontent.com/{}/{}/{}/{}",
                            owner, repo, git_ref, entry.path
                        )
                    });
                    if dedupe.insert(raw.clone()) {
                        found.push(raw);
                        if found.len() >= max_files {
                            return Ok(found);
                        }
                    }
                }
            } else if entry.item_type == "dir" && depth < max_depth {
                stack.push((entry.path, depth + 1));
            }
        }
    }

    Ok(found)
}

struct GitHubSkillUrlRequest<'a> {
    owner: &'a str,
    repo: &'a str,
    git_ref: &'a str,
    root_path: &'a str,
    max_depth: usize,
    max_files: usize,
    token: Option<&'a str>,
}

async fn collect_github_skill_urls_from_archive(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    git_ref: &str,
    root_path: &str,
    max_depth: usize,
    max_files: usize,
) -> Result<Vec<String>, String> {
    let archive_candidates = [
        format!(
            "https://github.com/{}/{}/archive/refs/heads/{}.zip",
            owner, repo, git_ref
        ),
        format!(
            "https://github.com/{}/{}/archive/{}.zip",
            owner, repo, git_ref
        ),
        format!(
            "https://github.com/{}/{}/archive/refs/tags/{}.zip",
            owner, repo, git_ref
        ),
    ];

    let mut last_fetch_error = String::new();
    let mut archive_bytes: Option<Vec<u8>> = None;
    for candidate in &archive_candidates {
        let validated = match validate_import_fetch_url(candidate).await {
            Ok(v) => v,
            Err(e) => {
                last_fetch_error = e;
                continue;
            }
        };
        match fetch_bytes_with_redirects(client, validated, 3, GITHUB_SKILL_ARCHIVE_MAX_BYTES).await
        {
            Ok(bytes) => {
                archive_bytes = Some(bytes);
                break;
            }
            Err(e) => last_fetch_error = format!("{} ({})", e, candidate),
        }
    }

    let archive_bytes = archive_bytes.ok_or_else(|| {
        if last_fetch_error.is_empty() {
            "Failed to download GitHub repository archive".to_string()
        } else {
            format!(
                "Failed to download GitHub repository archive: {}",
                last_fetch_error
            )
        }
    })?;

    let cursor = std::io::Cursor::new(archive_bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| format!("Invalid repository archive format: {}", e))?;
    if archive.len() > GITHUB_SKILL_ARCHIVE_MAX_ENTRIES {
        return Err(format!(
            "Repository archive has too many entries ({} > {} limit)",
            archive.len(),
            GITHUB_SKILL_ARCHIVE_MAX_ENTRIES
        ));
    }

    let root_path = root_path.trim_matches('/');
    let root_prefix = if root_path.is_empty() {
        String::new()
    } else {
        format!("{}/", root_path)
    };

    let mut found: Vec<String> = Vec::new();
    let mut dedupe: HashSet<String> = HashSet::new();

    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| format!("Failed reading archive entry: {}", e))?;
        if !file.is_file() {
            continue;
        }

        let entry_name = file.name().replace('\\', "/");
        let Some((_, rel_path)) = entry_name.split_once('/') else {
            continue;
        };
        let rel_path = rel_path.trim_matches('/');
        if rel_path.is_empty() {
            continue;
        }

        let in_scope = if root_prefix.is_empty() {
            rel_path
        } else if rel_path.starts_with(&root_prefix) {
            &rel_path[root_prefix.len()..]
        } else {
            continue;
        };

        let in_scope = in_scope.trim_matches('/');
        if in_scope.is_empty() {
            continue;
        }

        let lower = in_scope.to_ascii_lowercase();
        if !(lower.ends_with("/skill.md") || lower == "skill.md") {
            continue;
        }

        let file_depth = in_scope.matches('/').count();
        if file_depth > max_depth {
            continue;
        }

        let raw_url = format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}",
            owner, repo, git_ref, rel_path
        );
        if dedupe.insert(raw_url.clone()) {
            found.push(raw_url);
            if found.len() >= max_files {
                break;
            }
        }
    }

    Ok(found)
}

async fn discover_github_collection_urls(
    client: &reqwest::Client,
    raw_url: &str,
    token: Option<&str>,
) -> Result<Option<Vec<String>>, String> {
    let Some(loc) = parse_github_location(raw_url) else {
        return Ok(None);
    };
    if !loc.directory_hint {
        return Ok(None);
    }

    let refs: Vec<String> = if let Some(r) = loc.git_ref.clone() {
        vec![r]
    } else {
        vec!["main".to_string(), "master".to_string()]
    };

    let mut last_err = String::new();
    for git_ref in refs {
        match collect_github_skill_urls(
            client,
            &GitHubSkillUrlRequest {
                owner: &loc.owner,
                repo: &loc.repo,
                git_ref: &git_ref,
                root_path: &loc.path,
                max_depth: 4,
                max_files: 400,
                token,
            },
        )
        .await
        {
            Ok(urls) if !urls.is_empty() => return Ok(Some(urls)),
            Ok(_) => {
                last_err = format!(
                    "No SKILL.md files were found under '{}/{}@{}:{}'",
                    loc.owner, loc.repo, git_ref, loc.path
                );
            }
            Err(api_err) => {
                match collect_github_skill_urls_from_archive(
                    client, &loc.owner, &loc.repo, &git_ref, &loc.path, 4, 400,
                )
                .await
                {
                    Ok(urls) if !urls.is_empty() => return Ok(Some(urls)),
                    Ok(_) => {
                        last_err = format!(
                            "GitHub API error: {}. Archive fallback found no SKILL.md files under '{}/{}@{}:{}'",
                            api_err, loc.owner, loc.repo, git_ref, loc.path
                        );
                    }
                    Err(archive_err) => {
                        last_err = format!(
                            "GitHub API error: {}. Archive fallback error: {}",
                            api_err, archive_err
                        );
                    }
                }
            }
        }
    }

    if last_err.is_empty() {
        Ok(None)
    } else {
        Err(last_err)
    }
}

fn build_import_candidate_urls(source_url: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let parsed = match reqwest::Url::parse(source_url) {
        Ok(url) => url,
        Err(_) => return vec![source_url.to_string()],
    };
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    let canonical_host = parsed.host_str().unwrap_or("").trim();
    let path = parsed.path();
    let lower_url = source_url.to_ascii_lowercase();

    if is_known_skill_catalog_host(&host) {
        let path_trim = path.trim_matches('/');
        if path_trim.to_ascii_lowercase().ends_with(".md") {
            out.push(source_url.to_string());
        } else {
            let mut segments: Vec<&str> = path_trim
                .split('/')
                .filter(|segment| !segment.trim().is_empty())
                .collect();
            if matches!(segments.first(), Some(first) if first.eq_ignore_ascii_case("skills") || first.eq_ignore_ascii_case("skill"))
            {
                segments.remove(0);
            }
            if !segments.is_empty() {
                let mut slugs: Vec<String> = Vec::new();
                let full_slug = segments.join("/");
                if !full_slug.trim().is_empty() {
                    slugs.push(full_slug);
                }
                if segments.len() >= 2 {
                    slugs.push(segments[segments.len() - 2..].join("/"));
                }
                if let Some(last) = segments.last() {
                    slugs.push((*last).to_string());
                }

                for slug in slugs {
                    let slug = slug
                        .chars()
                        .filter(|c| {
                            c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '/'
                        })
                        .collect::<String>()
                        .trim_matches('/')
                        .to_string();
                    if slug.is_empty() {
                        continue;
                    }
                    unique_push(
                        &mut out,
                        format!(
                            "https://{}/api/v1/skills/{}/file?path=SKILL.md",
                            canonical_host, slug
                        ),
                    );
                    unique_push(
                        &mut out,
                        format!(
                            "https://{}/api/v1/skills/{}/file?path=SKILL.md&tag=latest",
                            canonical_host, slug
                        ),
                    );
                    unique_push(
                        &mut out,
                        format!(
                            "https://{}/api/v1/skills/{}/file?path=SKILL.md&version=latest",
                            canonical_host, slug
                        ),
                    );
                }
            }
        }
    } else if lower_url.contains("github.com") && lower_url.contains("/blob/") {
        if lower_url.ends_with(".md") || lower_url.contains("skill.md") {
            out.push(
                source_url
                    .replace("github.com", "raw.githubusercontent.com")
                    .replace("/blob/", "/"),
            );
        } else {
            let base = source_url
                .replace("github.com", "raw.githubusercontent.com")
                .replace("/blob/", "/");
            let base = base.trim_end_matches('/').to_string();
            out.push(format!("{}/SKILL.md", base));
        }
        out.push(source_url.to_string());
    } else if lower_url.contains("github.com") && lower_url.contains("/tree/") {
        let base = source_url
            .replace("github.com", "raw.githubusercontent.com")
            .replace("/tree/", "/");
        let base = base.trim_end_matches('/').to_string();
        out.push(format!("{}/SKILL.md", base));
        out.push(source_url.to_string());
    } else if host == "github.com" {
        let parts: Vec<String> = parsed
            .path_segments()
            .map(|segments| {
                segments
                    .filter(|part| !part.trim().is_empty())
                    .map(|part| part.to_string())
                    .collect()
            })
            .unwrap_or_default();
        if parts.len() >= 2 {
            let owner = parts[0].trim();
            let repo = parts[1].trim_end_matches(".git").trim();
            let tail = if parts.len() > 2 {
                Some(parts[2..].join("/"))
            } else {
                None
            };
            for branch in ["main", "master"] {
                let mut base = format!(
                    "https://raw.githubusercontent.com/{}/{}/{}",
                    owner, repo, branch
                );
                if let Some(tail) = &tail {
                    let tail = tail.trim_matches('/');
                    if !tail.is_empty() {
                        base.push('/');
                        base.push_str(tail);
                    }
                }
                let base = base.trim_end_matches('/').to_string();
                out.push(format!("{}/SKILL.md", base));
            }
        }
        out.push(source_url.to_string());
    } else {
        out.push(source_url.to_string());
    }

    out
}

pub(crate) async fn import_action_from_content_with_agent(
    agent: &Agent,
    source_url: &str,
    mut content: String,
    requested_name: Option<&str>,
    force: bool,
    model_override: Option<&str>,
    preview_only: bool,
) -> Result<serde_json::Value, String> {
    let name_from_content = extract_skill_name_from_frontmatter(&content);

    // Derive action name: user override > frontmatter > URL path
    let action_name = if let Some(name) = requested_name {
        if !name.trim().is_empty() {
            name.to_string()
        } else {
            name_from_content.clone().unwrap_or_default()
        }
    } else if let Some(ref name) = name_from_content {
        name.clone()
    } else {
        let segments: Vec<&str> = source_url.trim_end_matches('/').split('/').collect();
        segments
            .iter()
            .rev()
            .find(|s| !s.is_empty() && **s != "SKILL.md" && !s.contains('.'))
            .map(|s| s.to_string())
            .unwrap_or_else(|| "imported-action".to_string())
    };

    // Sanitize to kebab-case
    let action_name: String = action_name
        .to_lowercase()
        .replace([' ', '_'], "-")
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
        .collect();

    if action_name.is_empty() {
        return Err("Could not determine skill name from URL. Please provide a name.".to_string());
    }
    if !action_name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(
            "Skill name must contain only lowercase letters, numbers, and hyphens".to_string(),
        );
    }

    // Guardrail: a common mistake is importing a rendered web page URL instead of raw skill markdown.
    // Creating a skill from HTML leads to "No description" and non-functional workflow tests.
    let trimmed = content.trim_start();
    let looks_like_html = trimmed.starts_with("<!DOCTYPE html")
        || trimmed.starts_with("<html")
        || trimmed.starts_with("<!doctype html");
    if looks_like_html {
        return Err(
            "Imported content is HTML, not raw skill markdown. Please provide a raw SKILL.md URL."
                .to_string(),
        );
    }

    // Inject model into frontmatter if specified
    if let Some(model_str) = model_override {
        if !model_str.trim().is_empty() {
            let normalized = normalize_model_identifier(model_str)
                .ok_or_else(|| "Invalid model format. Expected 'provider/model'.".to_string())?;
            content = inject_model_into_frontmatter(&content, &normalized);
        }
    }

    let semantic_review = crate::security::skill_review::review_skill_import_with_configured_model(
        &agent.llm,
        &agent.config_dir,
        source_url,
        &action_name,
        &content,
    )
    .await;
    let blocked = semantic_review.policy.blocked;
    let findings: Vec<serde_json::Value> = semantic_review
        .policy
        .findings
        .iter()
        .map(import_finding_json)
        .collect();
    let warnings = semantic_review.policy.warnings.clone();
    let threat_level = format!("{:?}", semantic_review.policy.threat_level);
    let total_severity = semantic_review.policy.total_severity;
    let risk_score_10 = semantic_review.policy.risk_score_10;
    let risk_band = semantic_review.policy.risk_band.clone();
    let total_findings = semantic_review.policy.findings.len();
    let contextual_findings = 0_usize;
    let semantic_capabilities = semantic_review.capabilities.clone();
    let semantic_matched_rules = semantic_review.policy.matched_rules.clone();
    let semantic_review_model = semantic_review.model.clone();
    let semantic_review_summary = semantic_review.summary.clone();

    let mut status = if blocked {
        "blocked"
    } else if preview_only {
        "preview"
    } else {
        "ok"
    };

    let required_env = extract_frontmatter_text(&content)
        .map(extract_required_envs_from_frontmatter)
        .unwrap_or_default();
    let (missing_env, bindings) = if !required_env.is_empty() {
        let secrets = crate::core::config::SecureConfigManager::new_with_data_dir(
            &agent.config_dir,
            Some(&agent.data_dir),
        )
        .and_then(|mgr| mgr.load_secrets())
        .map_err(|e| format!("Failed to load encrypted secrets: {}", e))?;
        let custom = &secrets.custom;

        let mut missing = Vec::new();
        let mut bindings: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for env in &required_env {
            let binding_key = format!("action_envmap:{}:{}", action_name, env);
            if let Some(b) = custom.get(&binding_key) {
                bindings.insert(env.clone(), b.clone());
            }
            if !env_is_configured_for_action(&agent.config, custom, &action_name, env) {
                missing.push(env.clone());
            }
        }
        (missing, bindings)
    } else {
        (Vec::new(), std::collections::HashMap::new())
    };

    if !preview_only && status == "ok" && !missing_env.is_empty() {
        status = "needs_secrets";
    }

    if !preview_only && !blocked {
        let review = agent
            .runtime
            .install_semantically_reviewed_action(&action_name, &content, &semantic_review, force)
            .await
            .map_err(|e| e.to_string())?;
        if !review.missing_env.is_empty() {
            status = "needs_secrets";
        }
    }

    Ok(serde_json::json!({
        "status": status,
        "name": action_name,
        "message": if blocked {
            format!("Skill '{}' blocked by semantic security policy", action_name)
        } else if preview_only {
            if !missing_env.is_empty() {
                format!("Preview ready for '{}'. Required secrets detected.", action_name)
            } else {
                format!("Preview ready for '{}'. Click Import Template to save.", action_name)
            }
        } else if status == "needs_secrets" {
            format!("Skill '{}' imported, but needs secrets configured", action_name)
        } else {
            format!("Skill '{}' imported from URL", action_name)
        },
        "secrets": {
            "required_env": required_env,
            "missing_env": missing_env,
            "bindings": bindings,
        },
        "security": {
            "threat_level": threat_level,
            "warnings": warnings,
            "findings": findings,
            "blocked": blocked,
            "total_severity": total_severity,
            "risk_score_10": risk_score_10,
            "risk_band": risk_band,
            "total_findings": total_findings,
            "contextual_findings": contextual_findings,
            "capabilities": semantic_capabilities,
            "matched_rules": semantic_matched_rules,
            "review_model": semantic_review_model,
            "review_summary": semantic_review_summary,
        }
    }))
}

async fn import_action_from_content(
    state: &AppState,
    source_url: &str,
    content: String,
    requested_name: Option<&str>,
    force: bool,
    model_override: Option<&str>,
    preview_only: bool,
) -> Result<serde_json::Value, String> {
    let agent_guard = state.agent.read().await;
    import_action_from_content_with_agent(
        &agent_guard,
        source_url,
        content,
        requested_name,
        force,
        model_override,
        preview_only,
    )
    .await
}

#[derive(Debug, Clone)]
pub(crate) struct FetchedSkillMarkdown {
    pub source_url: String,
    pub content: String,
}

#[cfg(test)]
static TEST_SKILL_FETCH_OVERRIDES: std::sync::OnceLock<
    std::sync::Mutex<HashMap<String, FetchedSkillMarkdown>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
fn test_skill_fetch_overrides() -> &'static std::sync::Mutex<HashMap<String, FetchedSkillMarkdown>>
{
    TEST_SKILL_FETCH_OVERRIDES.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

#[cfg(test)]
pub(crate) fn register_test_skill_fetch_override(
    request_url: &str,
    source_url: &str,
    content: &str,
) {
    let mut overrides = test_skill_fetch_overrides()
        .lock()
        .expect("test skill fetch overrides lock");
    overrides.insert(
        request_url.trim().to_string(),
        FetchedSkillMarkdown {
            source_url: source_url.trim().to_string(),
            content: content.to_string(),
        },
    );
}

#[cfg(test)]
pub(crate) fn clear_test_skill_fetch_override(request_url: &str) {
    let mut overrides = test_skill_fetch_overrides()
        .lock()
        .expect("test skill fetch overrides lock");
    overrides.remove(request_url.trim());
}

pub(crate) async fn fetch_skill_markdown_from_url_shared(
    agent: &Agent,
    url: &str,
) -> Result<FetchedSkillMarkdown, String> {
    let url = url.trim();
    #[cfg(test)]
    {
        let overrides = test_skill_fetch_overrides()
            .lock()
            .expect("test skill fetch overrides lock");
        if let Some(fetched) = overrides.get(url).cloned() {
            return Ok(fetched);
        }
    }
    let _validated = validate_import_fetch_url(url).await?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("Failed to initialize HTTP client: {}", e))?;

    let github_token =
        crate::integrations::github::GitHubConnector::load_token_from(&agent.config_dir);
    let gh_tok = github_token.as_deref();

    let mut single_url_override: Option<String> = None;
    if let Some(loc) = parse_github_location(url) {
        if loc.directory_hint {
            match discover_github_collection_urls(&client, url, gh_tok).await {
                Ok(Some(mut discovered)) => {
                    discovered.sort();
                    discovered.dedup();
                    if discovered.len() > 1 {
                        return Err(
                            "That URL resolves to multiple skills. Use the Skills page bulk import flow for collection URLs."
                                .to_string(),
                        );
                    }
                    single_url_override = discovered.into_iter().next();
                }
                Ok(None) => {}
                Err(e) => {
                    return Err(format!("Failed to scan GitHub collection URL: {}", e));
                }
            }
        }
    }

    let candidate_source_url = single_url_override.as_deref().unwrap_or(url);
    let urls_to_try = build_import_candidate_urls(candidate_source_url);
    if urls_to_try.is_empty() {
        return Err(
            "Could not derive any raw skill URL candidates from the provided link.".to_string(),
        );
    }

    let mut content = None;
    let mut fetched_url: Option<String> = None;
    let mut last_error = String::new();
    for try_url in &urls_to_try {
        let validated = match validate_import_fetch_url(try_url).await {
            Ok(v) => v,
            Err(reason) => {
                last_error = reason;
                continue;
            }
        };
        match fetch_text_with_redirects(&client, validated, 3).await {
            Ok(text) => {
                content = Some(text);
                fetched_url = Some(try_url.clone());
                break;
            }
            Err(e) => last_error = e,
        }
    }

    let content = content.ok_or_else(|| {
        if url.contains("github.com")
            && (url.contains("/blob/") || url.contains("/tree/"))
            && !url.contains(".md")
        {
            format!(
                "Failed to fetch skill from URL. If this is a GitHub folder/repo, {} now scans for SKILL.md automatically. Tried {:?}: {}",
                crate::branding::PRODUCT_NAME,
                urls_to_try,
                last_error
            )
        } else {
            format!(
                "Failed to fetch skill from URL (tried {:?}): {}",
                urls_to_try, last_error
            )
        }
    })?;

    Ok(FetchedSkillMarkdown {
        source_url: fetched_url.unwrap_or_else(|| candidate_source_url.to_string()),
        content,
    })
}

pub(crate) async fn import_action_from_url_shared(
    agent: &Agent,
    url: &str,
    name: Option<&str>,
    force: bool,
    model: Option<&str>,
    preview_only: bool,
) -> Result<serde_json::Value, String> {
    let fetched = fetch_skill_markdown_from_url_shared(agent, url).await?;
    import_action_from_content_with_agent(
        agent,
        &fetched.source_url,
        fetched.content,
        name,
        force,
        model,
        preview_only,
    )
    .await
}

/// Import an action from a URL (e.g. GitHub raw content)
pub(super) async fn import_action(
    State(state): State<AppState>,
    Json(request): Json<ImportActionRequest>,
) -> Response {
    // Validate URL early (SSRF guard + scheme/host policy)
    let url = request.url.trim();
    if let Err(reason) = validate_import_fetch_url(url).await {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: reason }),
        )
            .into_response();
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        // Prevent reqwest from following redirects implicitly; we validate each redirect target.
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to initialize HTTP client: {}", e),
                }),
            )
                .into_response();
        }
    };

    // Load GitHub PAT (if configured) for authenticated API requests (higher rate limits).
    let github_token: Option<String> = {
        let config_dir = state.agent.read().await.config_dir.clone();
        crate::integrations::github::GitHubConnector::load_token_from(&config_dir)
    };
    let gh_tok = github_token.as_deref();

    // Explicit selected URL mode (bulk confirmation flow):
    // Import exactly the provided child skill URLs in a single request.
    if let Some(raw_selected) = request.selected_urls.as_ref() {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut selected_urls: Vec<String> = Vec::new();
        for entry in raw_selected {
            let trimmed = entry.trim();
            if trimmed.is_empty() {
                continue;
            }
            if seen.insert(trimmed.to_string()) {
                selected_urls.push(trimmed.to_string());
            }
        }

        if selected_urls.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "selected_urls was provided but no valid URLs were included."
                        .to_string(),
                }),
            )
                .into_response();
        }

        let mut imported: Vec<serde_json::Value> = Vec::new();
        let mut failed: Vec<serde_json::Value> = Vec::new();

        for file_url in selected_urls {
            let validated = match validate_import_fetch_url(&file_url).await {
                Ok(v) => v,
                Err(reason) => {
                    failed.push(serde_json::json!({
                        "url": file_url,
                        "error": reason,
                    }));
                    continue;
                }
            };
            let text = match fetch_text_with_redirects(&client, validated, 3).await {
                Ok(t) => t,
                Err(e) => {
                    failed.push(serde_json::json!({
                        "url": file_url,
                        "error": e,
                    }));
                    continue;
                }
            };
            match import_action_from_content(
                &state,
                &file_url,
                text,
                None,
                request.force,
                request.model.as_deref(),
                request.preview_only,
            )
            .await
            {
                Ok(result) => {
                    imported.push(serde_json::json!({
                        "url": file_url,
                        "result": result,
                    }));
                }
                Err(e) => {
                    failed.push(serde_json::json!({
                        "url": file_url,
                        "error": e,
                    }));
                }
            }
        }

        if imported.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "No skills could be imported from selected_urls. Failures: {}",
                        serde_json::to_string(&failed).unwrap_or_else(|_| "[]".to_string())
                    ),
                }),
            )
                .into_response();
        }

        let status = if request.preview_only {
            if failed.is_empty() {
                "preview"
            } else {
                "preview_partial"
            }
        } else if failed.is_empty() {
            "ok"
        } else {
            "partial"
        };
        let base_name = request
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| "bulk-import".to_string());

        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": status,
                "name": base_name,
                "message": if request.preview_only {
                    if failed.is_empty() {
                        format!("Previewed {} selected action(s)", imported.len())
                    } else {
                        format!(
                            "Previewed {} selected action(s) ({} failed)",
                            imported.len(),
                            failed.len()
                        )
                    }
                } else if failed.is_empty() {
                    format!("Imported {} selected action(s)", imported.len())
                } else {
                    format!(
                        "Imported {} selected action(s) ({} failed)",
                        imported.len(),
                        failed.len()
                    )
                },
                "source_url": url,
                "imported_count": imported.len(),
                "failed_count": failed.len(),
                "imported": imported,
                "failed": failed,
            })),
        )
            .into_response();
    }

    // Collection URL support: one GitHub folder/repo URL can contain many SKILL.md files.
    let mut single_url_override: Option<String> = None;
    if let Some(loc) = parse_github_location(url) {
        if loc.directory_hint {
            match discover_github_collection_urls(&client, url, gh_tok).await {
                Ok(Some(mut discovered)) => {
                    discovered.sort();
                    discovered.dedup();
                    if discovered.len() > 1 {
                        let mut imported: Vec<serde_json::Value> = Vec::new();
                        let mut failed: Vec<serde_json::Value> = Vec::new();

                        for file_url in discovered {
                            let validated = match validate_import_fetch_url(&file_url).await {
                                Ok(v) => v,
                                Err(reason) => {
                                    failed.push(serde_json::json!({
                                        "url": file_url,
                                        "error": reason,
                                    }));
                                    continue;
                                }
                            };
                            let text = match fetch_text_with_redirects(&client, validated, 3).await
                            {
                                Ok(t) => t,
                                Err(e) => {
                                    failed.push(serde_json::json!({
                                        "url": file_url,
                                        "error": e,
                                    }));
                                    continue;
                                }
                            };
                            match import_action_from_content(
                                &state,
                                &file_url,
                                text,
                                None,
                                request.force,
                                request.model.as_deref(),
                                request.preview_only,
                            )
                            .await
                            {
                                Ok(result) => {
                                    imported.push(serde_json::json!({
                                        "url": file_url,
                                        "result": result,
                                    }));
                                }
                                Err(e) => {
                                    failed.push(serde_json::json!({
                                        "url": file_url,
                                        "error": e,
                                    }));
                                }
                            }
                        }

                        if imported.is_empty() {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(ErrorResponse {
                                    error: format!(
                                        "No skills could be imported from collection URL. Failures: {}",
                                        serde_json::to_string(&failed)
                                            .unwrap_or_else(|_| "[]".to_string())
                                    ),
                                }),
                            )
                                .into_response();
                        }

                        let status = if failed.is_empty() { "ok" } else { "partial" };
                        let base_name = request
                            .name
                            .clone()
                            .filter(|n| !n.trim().is_empty())
                            .unwrap_or_else(|| "bulk-import".to_string());

                        return (
                            StatusCode::OK,
                            Json(serde_json::json!({
                                "status": status,
                                "name": base_name,
                                "message": if request.preview_only {
                                    if failed.is_empty() {
                                        format!("Previewed {} actions from collection URL", imported.len())
                                    } else {
                                        format!(
                                            "Previewed {} actions from collection URL ({} failed)",
                                            imported.len(),
                                            failed.len()
                                        )
                                    }
                                } else if failed.is_empty() {
                                    format!("Imported {} actions from collection URL", imported.len())
                                } else {
                                    format!(
                                        "Imported {} actions from collection URL ({} failed)",
                                        imported.len(),
                                        failed.len()
                                    )
                                },
                                "source_url": url,
                                "imported_count": imported.len(),
                                "failed_count": failed.len(),
                                "imported": imported,
                                "failed": failed,
                            })),
                        )
                            .into_response();
                    } else if discovered.len() == 1 {
                        single_url_override = discovered.into_iter().next();
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!("Failed to scan GitHub collection URL: {}", e),
                        }),
                    )
                        .into_response();
                }
            }
        }
    }

    let effective_url = single_url_override.as_deref().unwrap_or(url);
    let agent_guard = state.agent.read().await;
    match import_action_from_url_shared(
        &agent_guard,
        effective_url,
        request.name.as_deref(),
        request.force,
        request.model.as_deref(),
        request.preview_only,
    )
    .await
    {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response(),
    }
}

/// Delete an action
pub(super) async fn delete_action(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Response {
    let agent_guard = state.agent.read().await;
    let source = match agent_guard.runtime.get_action_content(&name).await {
        Ok(Some((info, _))) => Some(info.source),
        Ok(None) => None,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response();
        }
    };

    if source.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Skill '{}' not found", name),
            }),
        )
            .into_response();
    }

    match agent_guard.runtime.delete_action(&name).await {
        Ok(true) => {
            let message = match source {
                Some(crate::actions::ActionSource::Bundled) => {
                    "Bundled skill deleted from this install"
                }
                Some(crate::actions::ActionSource::Custom) => "Custom skill deleted",
                _ => "Skill updated",
            };
            spawn_autonomy_analysis_tick(state.agent.clone(), "action_deleted");
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "message": message})),
            )
                .into_response()
        }
        Ok(false) => (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Skill cannot be deleted (system skill)".to_string(),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}
