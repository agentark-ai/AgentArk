use super::*;

#[derive(Debug, Clone, Serialize)]
pub(super) struct IntegrationMonitoringRecommendation {
    pub id: String,
    pub integration_id: String,
    pub integration_name: String,
    pub automation_kind: String,
    pub title: String,
    pub summary: String,
    pub rationale: String,
    pub cadence_label: String,
}

enum RecommendationBlueprint {
    Watcher(serde_json::Value),
}

struct RecommendationSpec {
    view: IntegrationMonitoringRecommendation,
    blueprint: RecommendationBlueprint,
}

fn integration_enabled(
    manager: Option<&crate::core::config::SecureConfigManager>,
    id: &str,
) -> bool {
    manager
        .and_then(|mgr| {
            mgr.get_custom_secret(&super::integrations::integration_enabled_key(id))
                .ok()
                .flatten()
        })
        .and_then(|value| super::integrations::parse_boolish(&value))
        .unwrap_or(true)
}

async fn build_recommendation_specs(state: &AppState) -> Vec<RecommendationSpec> {
    let (config_dir, data_dir, integration_infos, enabled_ids) = {
        let agent = state.agent.read().await;
        (
            agent.config_dir.clone(),
            agent.data_dir.clone(),
            agent.integrations.list().await,
            agent
                .integrations
                .enabled_ids()
                .into_iter()
                .collect::<HashSet<_>>(),
        )
    };

    let manager =
        crate::core::config::SecureConfigManager::new_with_data_dir(&config_dir, Some(&data_dir))
            .ok();
    let info_by_id = integration_infos
        .into_iter()
        .map(|info| (info.id.clone(), info))
        .collect::<HashMap<_, _>>();

    let mut specs = Vec::new();

    let gmail_ready = manager
        .as_ref()
        .and_then(|mgr| mgr.get_custom_secret("gmail_tokens").ok().flatten())
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
        && integration_enabled(manager.as_ref(), "gmail");
    if gmail_ready {
        specs.push(RecommendationSpec {
            view: IntegrationMonitoringRecommendation {
                id: "gmail_unread_important".to_string(),
                integration_id: "gmail".to_string(),
                integration_name: "Gmail".to_string(),
                automation_kind: "watcher".to_string(),
                title: "Watch for new important unread email".to_string(),
                summary: "Poll Gmail every 5 minutes and alert when a new important or primary unread message appears.".to_string(),
                rationale: "This turns Gmail into a real background monitor instead of waiting for you to ask manually.".to_string(),
                cadence_label: "Every 5 minutes".to_string(),
            },
            blueprint: RecommendationBlueprint::Watcher(serde_json::json!({
                "description": "Monitor Gmail for new important unread messages",
                "poll_action": "gmail_scan",
                "poll_arguments": {
                    "query": "is:unread (category:primary OR is:important) newer_than:1d",
                    "max_results": 10
                },
                "condition_contains": "From:",
                "on_trigger": "Summarize the new email(s), identify urgency, and tell me what needs attention first.",
                "interval_secs": 300,
                "until_stopped": true,
                "notify_channel": ""
            }),
        });
    }

    if let Some(info) = info_by_id.get("github") {
        if enabled_ids.contains("github")
            && matches!(info.status, crate::integrations::IntegrationStatus::Connected)
        {
            specs.push(RecommendationSpec {
                view: IntegrationMonitoringRecommendation {
                    id: "github_unread_notifications".to_string(),
                    integration_id: "github".to_string(),
                    integration_name: info.name.clone(),
                    automation_kind: "watcher".to_string(),
                    title: "Watch for unread GitHub notifications".to_string(),
                    summary: "Poll GitHub notifications every 5 minutes and alert when a new unread item lands in your account.".to_string(),
                    rationale: format!(
                        "This gives {} a concrete ongoing monitoring job for GitHub without needing repo-specific setup first.",
                        crate::branding::PRODUCT_NAME
                    ),
                    cadence_label: "Every 5 minutes".to_string(),
                },
                blueprint: RecommendationBlueprint::Watcher(serde_json::json!({
                    "description": "Monitor GitHub for unread notifications",
                    "poll_action": "http_get",
                    "poll_arguments": {
                        "url": "https://api.github.com/notifications?all=false&participating=false&per_page=20",
                        "headers": {
                            "Authorization": "Bearer {{secret:github_token}}",
                            "User-Agent": crate::branding::versioned_user_agent(),
                            "Accept": "application/vnd.github+json"
                        }
                    },
                    "condition_contains": "\"unread\"",
                    "on_trigger": "Summarize the new GitHub notifications, point to the most important threads, and explain what likely needs action.",
                    "interval_secs": 300,
                    "until_stopped": true,
                    "notify_channel": ""
                }),
            });
        }
    }

    specs
}

pub(super) async fn list_integration_monitoring_recommendations(
    State(state): State<AppState>,
) -> Response {
    let recommendations = build_recommendation_specs(&state)
        .await
        .into_iter()
        .map(|spec| spec.view)
        .collect::<Vec<_>>();

    (StatusCode::OK, Json(serde_json::json!({ "recommendations": recommendations })))
        .into_response()
}

pub(super) async fn apply_integration_monitoring_recommendation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let recommendation_id = id.trim();
    if recommendation_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Recommendation id is required".to_string(),
            }),
        )
            .into_response();
    }

    let specs = build_recommendation_specs(&state).await;
    let Some(spec) = specs
        .into_iter()
        .find(|candidate| candidate.view.id == recommendation_id)
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Recommendation '{}' not found", recommendation_id),
            }),
        )
            .into_response();
    };

    let agent = state.agent.read().await;
    let (status, message) = match spec.blueprint {
        RecommendationBlueprint::Watcher(arguments) => match agent
            .handle_watch(&arguments, "integrations", None, None, None)
            .await
        {
            Some(message) => {
                let status = if message.starts_with("Updated existing watcher") {
                    "updated"
                } else {
                    "created"
                };
                (status, message)
            }
            None => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Failed to create watcher from recommendation".to_string(),
                    }),
                )
                    .into_response()
            }
        },
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": status,
            "recommendation_id": spec.view.id,
            "integration_id": spec.view.integration_id,
            "automation_kind": spec.view.automation_kind,
            "message": message,
        })),
    )
        .into_response()
}
