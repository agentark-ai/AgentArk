use super::super::*;

impl ActionRuntime {
    pub(in crate::runtime) fn browser_session_id(arguments: &serde_json::Value) -> Result<&str> {
        arguments
            .get("session_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("session_id required"))
    }

    pub(in crate::runtime) fn is_browser_wrapper_action(action_name: &str) -> bool {
        matches!(
            action_name,
            "browser_auto"
                | "browser_navigate"
                | "browser_click"
                | "browser_type"
                | "browser_scroll"
                | "browser_snapshot"
                | "browser_screenshot"
                | "browser_back"
                | "browser_press"
                | "browser_console"
        )
    }

    pub(in crate::runtime) fn normalize_browser_target_url(raw: &str) -> String {
        let trimmed = raw.trim();
        if reqwest::Url::parse(trimmed).is_ok() {
            return trimmed.to_string();
        }
        if trimmed.starts_with('/') {
            return format!(
                "{}{}",
                crate::core::runtime::net::internal_api_base_url().trim_end_matches('/'),
                trimmed
            );
        }
        trimmed.to_string()
    }

    pub(in crate::runtime) fn browser_profile_selector(
        arguments: &serde_json::Value,
    ) -> Option<String> {
        ["profile_id", "profile"]
            .into_iter()
            .filter_map(|key| {
                arguments
                    .get(key)
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            })
            .next()
    }

    pub(in crate::runtime) fn browser_profile_candidates_json(
        candidates: &[crate::core::BrowserProfileResolveCandidate],
    ) -> serde_json::Value {
        serde_json::Value::Array(
            candidates
                .iter()
                .map(|candidate| {
                    serde_json::json!({
                        "id": candidate.profile.id.clone(),
                        "name": candidate.profile.name.clone(),
                        "score": candidate.score,
                        "login_state": candidate.profile.login_state,
                        "target_kind": candidate.profile.target_kind,
                        "tags": candidate.profile.tags.clone(),
                    })
                })
                .collect(),
        )
    }

    pub(in crate::runtime) async fn browser_session_create_options(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<
        std::result::Result<
            (
                crate::integrations::browser::BrowserSessionCreateOptions,
                Option<crate::core::BrowserProfileRecord>,
            ),
            String,
        >,
    > {
        let Some(selector) = Self::browser_profile_selector(arguments) else {
            return Ok(Ok((
                crate::integrations::browser::BrowserSessionCreateOptions::default(),
                None,
            )));
        };
        let Some(storage) = self.storage.as_ref() else {
            return Ok(Err(structured_tool_completion_output(
                action_name,
                "failed",
                "A browser profile was requested, but profile storage is not available in this runtime.",
                serde_json::json!({
                    "success": false,
                    "reason": "browser_profile_storage_unavailable",
                    "profile_selector": selector,
                }),
            )));
        };
        match crate::core::BrowserProfileControlPlane::resolve(storage, &selector).await? {
            crate::core::BrowserProfileResolveOutcome::Resolved {
                profile,
                candidates,
            } => {
                let _ = candidates;
                Ok(Ok((
                    crate::integrations::browser::BrowserSessionCreateOptions::from_browser_profile(
                        &profile,
                    ),
                    Some(profile),
                )))
            }
            crate::core::BrowserProfileResolveOutcome::Ambiguous { candidates } => {
                Ok(Err(structured_tool_completion_output(
                    action_name,
                    "needs_input",
                    "More than one saved browser profile matches the requested profile. Choose one profile id or name.",
                    serde_json::json!({
                        "success": false,
                        "reason": "browser_profile_ambiguous",
                        "profile_selector": selector,
                        "candidates": Self::browser_profile_candidates_json(&candidates),
                    }),
                )))
            }
            crate::core::BrowserProfileResolveOutcome::NotFound { candidates } => {
                Ok(Err(structured_tool_completion_output(
                    action_name,
                    "needs_input",
                    "No saved browser profile matched the requested profile.",
                    serde_json::json!({
                        "success": false,
                        "reason": "browser_profile_not_found",
                        "profile_selector": selector,
                        "candidates": Self::browser_profile_candidates_json(&candidates),
                    }),
                )))
            }
        }
    }

    pub(in crate::runtime) async fn execute_browser_wrapper_action(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let browser = crate::integrations::browser::BrowserIntegration::new();
        if !browser.is_available().await {
            anyhow::bail!("Browser automation bridge is not available");
        }

        match action_name {
            "browser_auto" => {
                let action = arguments
                    .get("action")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("start_session");
                if action != "start_session" {
                    anyhow::bail!("browser_auto supports action=start_session");
                }
                let (create_options, profile) = match self
                    .browser_session_create_options(action_name, arguments)
                    .await?
                {
                    Ok(value) => value,
                    Err(output) => return Ok(output),
                };
                let session_id = browser.create_session_with_options(&create_options).await?;
                let url = arguments
                    .get("url")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let (final_url, title, body_text, diagnostics) = if let Some(url) = url {
                    let normalized_url = Self::normalize_browser_target_url(url);
                    let (final_url, title) = browser.navigate(&session_id, &normalized_url).await?;
                    let content = browser.get_content(&session_id).await?;
                    (
                        final_url,
                        title,
                        Some(runtime_truncate_chars(&content.body_text, 5_000)),
                        Some(content.diagnostics),
                    )
                } else {
                    (String::new(), String::new(), None, None)
                };
                if let (Some(storage), Some(profile)) = (self.storage.as_ref(), profile.as_ref()) {
                    let _ = crate::core::BrowserProfileControlPlane::record_session(
                        storage,
                        crate::core::BrowserProfileSessionRecord {
                            profile_id: profile.id.clone(),
                            session_id: Some(session_id.clone()),
                            started_at: chrono::Utc::now().to_rfc3339(),
                            outcome: "started".to_string(),
                            channel: Some("runtime".to_string()),
                            note: arguments
                                .get("task")
                                .and_then(|value| value.as_str())
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .map(str::to_string),
                            title: (!title.trim().is_empty()).then(|| title.clone()),
                            url: (!final_url.trim().is_empty()).then(|| final_url.clone()),
                            ..Default::default()
                        },
                    )
                    .await;
                }
                Ok(structured_tool_completion_output(
                    action_name,
                    "completed",
                    if final_url.trim().is_empty() {
                        format!("Started browser session {}.", session_id)
                    } else {
                        format!(
                            "Started browser session {} and navigated to {}.",
                            session_id, final_url
                        )
                    },
                    serde_json::json!({
                        "session_id": session_id,
                        "url": final_url,
                        "title": title,
                        "body_text": body_text,
                        "diagnostics": diagnostics,
                        "profile": profile.as_ref().map(|profile| serde_json::json!({
                            "id": profile.id.clone(),
                            "name": profile.name.clone(),
                        })),
                    }),
                ))
            }
            "browser_navigate" => {
                let url = arguments
                    .get("url")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("url required"))?;
                let session_id = match arguments
                    .get("session_id")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    Some(session_id) => session_id.to_string(),
                    None => {
                        let (create_options, profile) = match self
                            .browser_session_create_options(action_name, arguments)
                            .await?
                        {
                            Ok(value) => value,
                            Err(output) => return Ok(output),
                        };
                        let session_id =
                            browser.create_session_with_options(&create_options).await?;
                        if let (Some(storage), Some(profile)) =
                            (self.storage.as_ref(), profile.as_ref())
                        {
                            let _ = crate::core::BrowserProfileControlPlane::record_session(
                                storage,
                                crate::core::BrowserProfileSessionRecord {
                                    profile_id: profile.id.clone(),
                                    session_id: Some(session_id.clone()),
                                    started_at: chrono::Utc::now().to_rfc3339(),
                                    outcome: "started".to_string(),
                                    channel: Some("runtime".to_string()),
                                    note: Some(format!("Navigate to {}", url)),
                                    ..Default::default()
                                },
                            )
                            .await;
                        }
                        session_id
                    }
                };
                let normalized_url = Self::normalize_browser_target_url(url);
                let (final_url, title) = browser.navigate(&session_id, &normalized_url).await?;
                Ok(structured_tool_completion_output(
                    action_name,
                    "completed",
                    format!("Navigated browser session {} to {}.", session_id, final_url),
                    serde_json::json!({
                        "session_id": session_id,
                        "url": final_url,
                        "title": title,
                    }),
                ))
            }
            "browser_click" => {
                let session_id = Self::browser_session_id(arguments)?;
                let element_index = arguments
                    .get("element_index")
                    .or_else(|| arguments.get("index"))
                    .and_then(|value| value.as_u64())
                    .map(|value| usize::try_from(value).context("element_index is out of range"))
                    .transpose()?;
                let selector = arguments.get("selector").and_then(|value| value.as_str());
                let text = arguments.get("text").and_then(|value| value.as_str());
                let x = arguments
                    .get("x")
                    .and_then(|value| value.as_i64())
                    .map(|value| i32::try_from(value).context("x is out of range"))
                    .transpose()?;
                let y = arguments
                    .get("y")
                    .and_then(|value| value.as_i64())
                    .map(|value| i32::try_from(value).context("y is out of range"))
                    .transpose()?;
                if element_index.is_none()
                    && selector.is_none()
                    && text.is_none()
                    && (x.is_none() || y.is_none())
                {
                    anyhow::bail!(
                        "browser_click requires element_index, selector, text, or both x and y"
                    );
                }
                browser
                    .click(session_id, selector, text, x, y, element_index)
                    .await?;
                Ok(structured_tool_completion_output(
                    action_name,
                    "completed",
                    "Clicked in browser session.",
                    serde_json::json!({
                        "session_id": session_id,
                        "status": "clicked",
                    }),
                ))
            }
            "browser_type" => {
                let session_id = Self::browser_session_id(arguments)?;
                let text = arguments
                    .get("text")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| anyhow::anyhow!("text required"))?;
                let selector = arguments.get("selector").and_then(|value| value.as_str());
                let element_index = arguments
                    .get("element_index")
                    .or_else(|| arguments.get("index"))
                    .and_then(|value| value.as_u64())
                    .map(|value| value as usize);
                let clear = arguments
                    .get("clear")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                browser
                    .type_text(session_id, text, selector, element_index, clear)
                    .await?;
                Ok(structured_tool_completion_output(
                    action_name,
                    "completed",
                    "Typed text in browser session.",
                    serde_json::json!({
                        "session_id": session_id,
                        "status": "typed",
                    }),
                ))
            }
            "browser_scroll" => {
                let session_id = Self::browser_session_id(arguments)?;
                let direction = arguments
                    .get("direction")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("down");
                if !matches!(direction, "up" | "down") {
                    anyhow::bail!("direction must be up or down");
                }
                let amount = arguments
                    .get("amount")
                    .and_then(|value| value.as_i64())
                    .map(|value| i32::try_from(value).context("amount is out of range"))
                    .transpose()?;
                browser.scroll(session_id, direction, amount).await?;
                Ok(structured_tool_completion_output(
                    action_name,
                    "completed",
                    "Scrolled browser session.",
                    serde_json::json!({
                        "session_id": session_id,
                        "direction": direction,
                        "amount": amount,
                    }),
                ))
            }
            "browser_snapshot" => {
                let session_id = Self::browser_session_id(arguments)?;
                let include_text = arguments
                    .get("include_text")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(true);
                let include_elements = arguments
                    .get("include_elements")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(true);
                let element_limit = arguments
                    .get("element_limit")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(50)
                    .min(50) as usize;
                let content = browser.get_content(session_id).await?;
                let elements = if include_elements {
                    content
                        .elements
                        .iter()
                        .take(element_limit)
                        .cloned()
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let body_text = if include_text {
                    runtime_truncate_chars(&content.body_text, 5_000)
                } else {
                    String::new()
                };
                Ok(structured_tool_completion_output(
                    action_name,
                    "completed",
                    format!(
                        "Snapshot captured for {} with {} visible element(s).",
                        content.url,
                        elements.len()
                    ),
                    serde_json::json!({
                        "session_id": session_id,
                        "title": content.title,
                        "url": content.url,
                        "body_text": body_text,
                        "elements": elements,
                        "diagnostics": content.diagnostics,
                    }),
                ))
            }
            "browser_screenshot" => {
                let session_id = Self::browser_session_id(arguments)?;
                let bytes = browser.screenshot(session_id).await?;
                let image_base64 = base64::engine::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &bytes,
                );
                Ok(structured_tool_completion_output(
                    action_name,
                    "completed",
                    format!("Captured browser screenshot ({} bytes).", bytes.len()),
                    serde_json::json!({
                        "session_id": session_id,
                        "image_base64": image_base64,
                        "size_bytes": bytes.len(),
                    }),
                ))
            }
            "browser_back" => {
                let session_id = Self::browser_session_id(arguments)?;
                let (url, title) = browser.back(session_id).await?;
                Ok(structured_tool_completion_output(
                    action_name,
                    "completed",
                    "Navigated browser session back.",
                    serde_json::json!({
                        "session_id": session_id,
                        "url": url,
                        "title": title,
                    }),
                ))
            }
            "browser_press" => {
                let session_id = Self::browser_session_id(arguments)?;
                let key = arguments
                    .get("key")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("key required"))?;
                browser.press_key(session_id, key).await?;
                Ok(structured_tool_completion_output(
                    action_name,
                    "completed",
                    "Pressed key in browser session.",
                    serde_json::json!({
                        "session_id": session_id,
                        "key": key,
                    }),
                ))
            }
            "browser_console" => {
                let session_id = Self::browser_session_id(arguments)?;
                let severity = arguments
                    .get("severity")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_ascii_lowercase);
                let limit = arguments
                    .get("limit")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(40)
                    .clamp(1, 80) as usize;
                let content = browser.get_content(session_id).await?;
                let mut diagnostics = content
                    .diagnostics
                    .into_iter()
                    .filter(|entry| {
                        severity
                            .as_ref()
                            .map(|severity| entry.severity.eq_ignore_ascii_case(severity))
                            .unwrap_or(true)
                    })
                    .collect::<Vec<_>>();
                if diagnostics.len() > limit {
                    diagnostics.drain(0..diagnostics.len() - limit);
                }
                Ok(structured_tool_completion_output(
                    action_name,
                    "completed",
                    format!("Returned {} browser diagnostic(s).", diagnostics.len()),
                    serde_json::json!({
                        "session_id": session_id,
                        "severity": severity,
                        "diagnostics": diagnostics,
                    }),
                ))
            }
            _ => anyhow::bail!("Unsupported browser action {}", action_name),
        }
    }
}
