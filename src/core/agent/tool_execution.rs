use super::*;

#[derive(Default)]
struct AppDeployProgressRelayState {
    announced_file_writes: bool,
    sent_messages: HashSet<String>,
}

fn app_deploy_chat_progress_message(
    ev: &crate::core::StreamEvent,
    state: &mut AppDeployProgressRelayState,
) -> Option<String> {
    let content = match ev {
        crate::core::StreamEvent::ToolProgress { name, content, .. }
        | crate::core::StreamEvent::ToolResult { name, content }
            if name == "app_deploy" =>
        {
            content.trim()
        }
        _ => return None,
    };
    if content.is_empty() {
        return None;
    }

    let lower = content.to_ascii_lowercase();
    let mut message = if lower.starts_with("deploying '") {
        Some(format!("I'm deploying the app now. {}", content))
    } else if lower.starts_with("writing ") || lower.contains(" line ") {
        if state.announced_file_writes {
            None
        } else {
            state.announced_file_writes = true;
            Some("I'm writing the generated app files now.".to_string())
        }
    } else if lower.contains("files ready") {
        Some("The app files are ready. I'm preparing the runtime now.".to_string())
    } else if lower == "saved app metadata" {
        Some("I saved the app metadata and deployment settings.".to_string())
    } else if lower.starts_with("assigned port ") {
        Some(
            "I reserved the app runtime port. Next I'm checking whether any required setup is still missing."
                .to_string(),
        )
    } else if lower == "installing dependencies..." {
        Some("I'm installing the app dependencies now.".to_string())
    } else if lower == "no dependencies to install" {
        Some("No dependency install is needed. I'm starting the app now.".to_string())
    } else if lower.starts_with("starting server on port ") {
        Some("I'm starting the app server now.".to_string())
    } else if lower == "server container started" {
        Some("The app container started. I'm checking that it comes up cleanly.".to_string())
    } else if lower == "docker unavailable; started local app process" {
        Some(
            "Docker was unavailable, so I started the app locally instead. I'm checking it now."
                .to_string(),
        )
    } else if lower.starts_with("validating deployed app") {
        Some("I'm validating the deployed app now.".to_string())
    } else if lower.starts_with("starting public tunnel for app access")
        || lower.starts_with("starting cloudflare tunnel for public app access")
    {
        Some("I'm trying to start a public tunnel so you can open the app externally.".to_string())
    } else if lower.starts_with("app created but waiting for required inputs:") {
        Some(content.to_string())
    } else if lower.starts_with("static app ready at ") {
        Some("The static app is ready. I'm preparing the access link now.".to_string())
    } else if lower.starts_with("dynamic app ready at ") {
        Some(
            "The app process is up. I'm validating that it stays healthy before I share the final status."
                .to_string(),
        )
    } else {
        None
    }?;

    message = safe_truncate(&message, 220);
    if !state.sent_messages.insert(message.clone()) {
        return None;
    }
    Some(message)
}

fn merge_chat_visible_progress_payload(
    payload: Option<serde_json::Value>,
    chat_message: &str,
) -> Option<serde_json::Value> {
    let mut merged = match payload {
        Some(serde_json::Value::Object(obj)) => obj,
        Some(other) => {
            let mut obj = serde_json::Map::new();
            obj.insert("payload".to_string(), other);
            obj
        }
        None => serde_json::Map::new(),
    };
    merged.insert("chat_visible".to_string(), serde_json::json!(true));
    merged.insert("chat_message".to_string(), serde_json::json!(chat_message));
    Some(serde_json::Value::Object(merged))
}

fn trace_json_data(value: serde_json::Value) -> Option<String> {
    if value.is_null() {
        None
    } else {
        serde_json::to_string_pretty(&value).ok()
    }
}

async fn push_trace_step(
    trace_ref: &Arc<RwLock<ExecutionTrace>>,
    icon: &str,
    title: impl Into<String>,
    detail: impl Into<String>,
    step_type: &str,
    data: Option<serde_json::Value>,
    duration_ms: Option<u64>,
) {
    trace_ref.write().await.steps.push(ExecutionStep {
        icon: icon.to_string(),
        title: title.into(),
        detail: detail.into(),
        step_type: step_type.to_string(),
        data: data.and_then(trace_json_data),
        timestamp: chrono::Utc::now(),
        duration_ms,
    });
}

fn json_changed_keys(previous_raw: Option<&[u8]>, next: &serde_json::Value) -> Vec<String> {
    let previous = previous_raw
        .and_then(|raw| serde_json::from_slice::<serde_json::Value>(raw).ok())
        .unwrap_or(serde_json::Value::Null);

    let (Some(previous_obj), Some(next_obj)) = (previous.as_object(), next.as_object()) else {
        if previous == *next {
            return Vec::new();
        }
        return vec!["policy".to_string()];
    };

    let mut changed = previous_obj
        .keys()
        .chain(next_obj.keys())
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .filter(|key| previous_obj.get(key) != next_obj.get(key))
        .collect::<Vec<_>>();
    changed.sort();
    changed
}

async fn send_app_deploy_progress_to_conversation(
    request_channel: &str,
    conversation_id: Option<&str>,
    telegram_config: Option<&crate::core::config::TelegramConfig>,
    whatsapp_config: Option<&crate::channels::whatsapp::WhatsAppChannelConfig>,
    agent_name: &str,
    message: &str,
) {
    if message.trim().is_empty() {
        return;
    }
    match request_channel {
        #[cfg(feature = "telegram")]
        "telegram" => {
            let Some(config) = telegram_config else {
                return;
            };
            let Some(chat_id) = conversation_id
                .and_then(|cid| cid.strip_prefix("telegram:"))
                .and_then(|value| value.parse::<i64>().ok())
            else {
                return;
            };
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                crate::channels::telegram::send_message_to_chat(config, chat_id, message),
            )
            .await;
        }
        "whatsapp" => {
            let Some(config) = whatsapp_config else {
                return;
            };
            let Some(phone_number) = conversation_id.and_then(|cid| cid.strip_prefix("whatsapp:"))
            else {
                return;
            };
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                crate::channels::whatsapp::send_message_to_recipient(
                    config,
                    phone_number,
                    agent_name,
                    message,
                ),
            )
            .await;
        }
        _ => {}
    }
}

pub(crate) struct ToolExecutionContext<'a> {
    pub request_channel: &'a str,
    pub current_turn_is_explicit_approval: bool,
    pub trace_id: Option<&'a str>,
    pub conversation_id: Option<&'a str>,
    pub project_id: Option<&'a str>,
    pub strategy_version: Option<&'a str>,
    pub policy_version: Option<&'a str>,
    pub prompt_version: Option<&'a str>,
    pub model_slot: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub(crate) struct ToolCallOutput {
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ToolExecutionBatch {
    pub outputs: Vec<ToolCallOutput>,
}

fn action_has_dangerous_capabilities(action_def: Option<&crate::actions::ActionDef>) -> bool {
    action_def.is_some_and(|action| {
        action.capabilities.iter().any(|cap| {
            matches!(
                crate::security::action_guard::ActionGuard::permission_risk(
                    &crate::security::action_guard::ActionGuard::parse_permission(cap)
                ),
                crate::security::action_guard::PermissionRisk::Dangerous
            )
        })
    })
}

fn tool_call_has_structural_side_effect_markers(call: &crate::core::llm::ToolCall) -> bool {
    call.arguments.get("notify_channel").is_some()
        || call.arguments.get("on_trigger").is_some()
        || call.arguments.get("files").is_some()
}

fn blocked_by_saved_rule_message(constraints: &super::UserExecutionConstraints) -> String {
    if constraints.require_explicit_approval_before_side_effects
        && constraints.show_plan_before_side_effects
    {
        "Blocked by saved user rule: show the plan first, then wait for explicit approval before any side-effecting action.".to_string()
    } else if constraints.require_explicit_approval_before_side_effects {
        "Blocked by saved user rule: explicit approval is required before any side-effecting action.".to_string()
    } else {
        "Blocked by saved user rule: show the plan before any side-effecting action.".to_string()
    }
}

impl ToolExecutionBatch {
    pub(crate) fn combined_output(&self) -> String {
        self.outputs
            .iter()
            .map(|output| output.content.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone)]
struct AppSemanticFingerprint {
    title_tokens: std::collections::HashSet<String>,
    keyword_tokens: std::collections::HashSet<String>,
    file_tokens: std::collections::HashSet<String>,
    is_static: bool,
}

#[derive(Debug, Clone)]
struct AppDuplicateMatch {
    app: serde_json::Value,
    match_kind: &'static str,
    score: f32,
    reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DuplicateAppResolution {
    ReuseExisting,
    ReplaceExisting,
}

impl Agent {
    fn canonicalize_json_value(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort_unstable();
                let mut ordered = serde_json::Map::new();
                for key in keys {
                    if let Some(inner) = map.get(key) {
                        ordered.insert(key.clone(), Self::canonicalize_json_value(inner));
                    }
                }
                serde_json::Value::Object(ordered)
            }
            serde_json::Value::Array(items) => serde_json::Value::Array(
                items
                    .iter()
                    .map(Self::canonicalize_json_value)
                    .collect::<Vec<_>>(),
            ),
            _ => value.clone(),
        }
    }

    async fn classify_tool_call_side_effecting(
        &self,
        call: &crate::core::llm::ToolCall,
        action_def: Option<&crate::actions::ActionDef>,
    ) -> bool {
        if action_has_dangerous_capabilities(action_def)
            || tool_call_has_structural_side_effect_markers(call)
        {
            return true;
        }

        let Some(action) = action_def else {
            return true;
        };
        let Some(candidate) = self
            .llm_candidates_for_role(&crate::core::config::ModelRole::Fast)
            .into_iter()
            .next()
        else {
            return true;
        };

        let schema = action.input_schema.as_object().cloned().unwrap_or_default();
        let required = schema
            .get("required")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .take(12)
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let properties = schema
            .get("properties")
            .and_then(|value| value.as_object())
            .map(|props| {
                let mut keys = props.keys().cloned().collect::<Vec<_>>();
                keys.sort();
                keys.into_iter().take(16).collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let prompt = format!(
            "Classify the following tool call.\n\nAction name: {name}\nDescription: {description}\nCapabilities: {capabilities}\nRequired fields: {required}\nSchema fields: {properties}\nArguments: {arguments}\n\nReturn JSON only with this shape:\n{{\"side_effecting\":true}}\n\nRule:\n- true if executing the tool call would create, update, delete, send, schedule, notify, persist, deploy, restart, or otherwise mutate state outside pure read/inspection.\n- false only for read-only inspection, search, retrieval, listing, validation, or analysis actions that do not change state.",
            name = action.name,
            description = action.description,
            capabilities = if action.capabilities.is_empty() {
                "(none)".to_string()
            } else {
                action.capabilities.join(", ")
            },
            required = if required.is_empty() {
                "(none)".to_string()
            } else {
                required.join(", ")
            },
            properties = if properties.is_empty() {
                "(none)".to_string()
            } else {
                properties.join(", ")
            },
            arguments = serde_json::to_string(&call.arguments).unwrap_or_default(),
        );

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(900),
            candidate.client.chat(
                "You classify tool calls as side-effecting or read-only. Output JSON only.",
                &prompt,
                &[],
                &[],
            ),
        )
        .await;
        let Ok(Ok(resp)) = result else {
            return true;
        };
        self.record_llm_usage("system", "tool_side_effect_classifier", &resp)
            .await;
        extract_json_object_from_text(&resp.content)
            .and_then(|payload| {
                payload
                    .get("side_effecting")
                    .and_then(|value| value.as_bool())
            })
            .unwrap_or(true)
    }

    pub(crate) fn tool_call_signature(call: &crate::core::llm::ToolCall) -> String {
        let normalized_name = call.name.trim().to_ascii_lowercase();
        if normalized_name == "watch" {
            return format!(
                "watch:{}",
                crate::core::watcher::watcher_tool_call_signature_from_arguments(&call.arguments)
            );
        }
        if normalized_name == "schedule_task" {
            let description = call
                .arguments
                .get("task")
                .and_then(|value| value.as_str())
                .unwrap_or("scheduled task");
            let action_name = call
                .arguments
                .get("action")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let action_arguments = call
                .arguments
                .get("action_arguments")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let cron_expr = call.arguments.get("cron").and_then(|value| value.as_str());
            let at_time = call.arguments.get("at").and_then(|value| value.as_str());
            return format!(
                "schedule_task:{}",
                crate::core::task::task_request_signature_from_fields(
                    action_name,
                    description,
                    &action_arguments,
                    cron_expr,
                    at_time
                )
            );
        }
        let canonical_args = Self::canonicalize_json_value(&call.arguments);
        let args = serde_json::to_string(&canonical_args).unwrap_or_else(|_| "{}".to_string());
        format!("{}:{}", normalized_name, args)
    }

    fn classify_self_tune_tool_output(output: &str) -> Option<bool> {
        let lowered = output.trim().to_ascii_lowercase();
        if lowered.is_empty()
            || lowered.contains("blocked by safety policy")
            || lowered.contains("blocked by saved user rule")
        {
            return None;
        }
        Some(!(lowered.starts_with("error ") || lowered.starts_with("error:")))
    }

    pub(crate) async fn record_self_tune_tool_outcome(
        &self,
        tool_name: &str,
        success: bool,
        latency_ms: u64,
    ) {
        if tool_name.trim().is_empty() {
            return;
        }
        crate::core::self_tune::record_tool_outcome(&self.storage, tool_name, success, latency_ms)
            .await;
    }

    async fn record_self_tune_tool_output(&self, tool_name: &str, output: &str, latency_ms: u64) {
        if let Some(success) = Self::classify_self_tune_tool_output(output) {
            self.record_self_tune_tool_outcome(tool_name, success, latency_ms)
                .await;
        }
    }

    pub(crate) async fn record_self_tune_autonomous_success(&self) {
        crate::core::self_tune::record_autonomous_success(&self.storage).await;
    }

    pub(crate) async fn record_self_tune_user_rejection(&self) {
        crate::core::self_tune::record_user_rejection(&self.storage).await;
    }

    fn find_json_object_bounds(raw: &str) -> Option<(usize, usize)> {
        let mut depth = 0i32;
        let mut start: Option<usize> = None;
        let mut in_string = false;
        let mut escaped = false;

        for (idx, ch) in raw.char_indices() {
            if in_string {
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    continue;
                }
                if ch == '"' {
                    in_string = false;
                }
                continue;
            }

            match ch {
                '"' => in_string = true,
                '{' => {
                    if depth == 0 {
                        start = Some(idx);
                    }
                    depth += 1;
                }
                '}' => {
                    if depth > 0 {
                        depth -= 1;
                        if depth == 0 {
                            if let Some(s) = start {
                                return Some((s, idx + ch.len_utf8()));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn parse_json_object_str(raw: &str) -> Option<serde_json::Value> {
        let mut candidate = raw.trim().to_string();
        if candidate.is_empty() {
            return None;
        }

        for _ in 0..5 {
            let trimmed = candidate.trim();
            if trimmed.is_empty() {
                return None;
            }

            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
                match parsed {
                    serde_json::Value::Object(_) => return Some(parsed),
                    serde_json::Value::String(s) => {
                        candidate = s;
                        continue;
                    }
                    _ => {}
                }
            }

            if let Some((start, end)) = Self::find_json_object_bounds(trimmed) {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&trimmed[start..end])
                {
                    if parsed.is_object() {
                        return Some(parsed);
                    }
                }
            }

            if trimmed.starts_with('"') && trimmed.ends_with('"') {
                if let Ok(unwrapped) = serde_json::from_str::<String>(trimmed) {
                    candidate = unwrapped;
                    continue;
                }
            }

            if trimmed.contains("\\\"") {
                let rebuilt = trimmed.replace("\\\"", "\"");
                if rebuilt != trimmed {
                    candidate = rebuilt;
                    continue;
                }
            }

            break;
        }

        None
    }

    fn extract_files_object(
        value: &serde_json::Value,
    ) -> Option<serde_json::Map<String, serde_json::Value>> {
        if let Some(obj) = value.as_object() {
            return Some(obj.clone());
        }

        if let Some(raw) = value.as_str() {
            if let Some(parsed) = Self::parse_json_object_str(raw) {
                return parsed.as_object().cloned();
            }
        }

        let rows = value.as_array()?;
        let mut out = serde_json::Map::new();
        for row in rows {
            let Some(item) = row.as_object() else {
                continue;
            };
            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("filename").and_then(|v| v.as_str()))
                .or_else(|| item.get("path").and_then(|v| v.as_str()))
                .map(|v| v.trim())
                .unwrap_or("");
            if name.is_empty() {
                continue;
            }
            let content = item
                .get("content")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("text").and_then(|v| v.as_str()))
                .or_else(|| item.get("body").and_then(|v| v.as_str()))
                .unwrap_or("");
            out.insert(
                name.to_string(),
                serde_json::Value::String(content.to_string()),
            );
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    pub(crate) fn normalize_app_deploy_arguments(
        arguments: &serde_json::Value,
    ) -> serde_json::Value {
        let mut nested = if let Some(obj) = arguments.as_object() {
            if let Some(files_value) = obj.get("files") {
                if let Some(files_obj) = Self::extract_files_object(files_value) {
                    let mut normalized = obj.clone();
                    normalized.insert("files".to_string(), serde_json::Value::Object(files_obj));
                    return serde_json::Value::Object(normalized);
                }
            }

            let mut found: Option<serde_json::Value> = None;
            for key in [
                "payload",
                "arguments",
                "args",
                "input",
                "params",
                "tool_input",
                "tool_arguments",
            ] {
                if let Some(candidate) = obj.get(key) {
                    if candidate.is_object() {
                        found = Some(candidate.clone());
                        break;
                    }
                    if let Some(s) = candidate.as_str() {
                        if let Some(parsed) = Self::parse_json_object_str(s) {
                            found = Some(parsed);
                            break;
                        }
                    }
                }
            }
            found
        } else if let Some(s) = arguments.as_str() {
            Self::parse_json_object_str(s)
        } else {
            None
        };

        let Some(mut normalized) = nested.take() else {
            return arguments.clone();
        };

        if let Some(nested_obj) = normalized.as_object_mut() {
            if let Some(files_value) = nested_obj.get("files").cloned() {
                if let Some(files_obj) = Self::extract_files_object(&files_value) {
                    nested_obj.insert("files".to_string(), serde_json::Value::Object(files_obj));
                }
            } else {
                for alias in ["file_map", "source_files", "project_files", "artifacts"] {
                    if let Some(alias_value) = nested_obj.get(alias).cloned() {
                        if let Some(files_obj) = Self::extract_files_object(&alias_value) {
                            nested_obj
                                .insert("files".to_string(), serde_json::Value::Object(files_obj));
                            break;
                        }
                    }
                }
            }
        }

        if let (Some(root), Some(nested_obj)) = (arguments.as_object(), normalized.as_object_mut())
        {
            for key in [
                "title",
                "entry_command",
                "install_command",
                "runtime_image",
                "runtime_preference",
                "expose_public",
                "access_guard",
                "required_inputs",
                "required_secrets",
                "required_env",
                "required_config",
                "config",
            ] {
                if nested_obj.get(key).is_none() {
                    if let Some(v) = root.get(key) {
                        nested_obj.insert(key.to_string(), v.clone());
                    }
                }
            }
        }

        normalized
    }

    fn summarize_app_deploy_stream_payload(arguments: &serde_json::Value) -> serde_json::Value {
        let normalized = Self::normalize_app_deploy_arguments(arguments);
        let Some(obj) = normalized.as_object() else {
            return normalized;
        };

        let mut summary = serde_json::Map::new();
        for key in [
            "title",
            "entry_command",
            "install_command",
            "runtime_image",
            "runtime_preference",
            "expose_public",
            "access_guard",
        ] {
            if let Some(value) = obj.get(key) {
                summary.insert(key.to_string(), value.clone());
            }
        }

        if let Some(files) = obj.get("files").and_then(|v| v.as_object()) {
            let mut file_names: Vec<String> = files.keys().cloned().collect();
            file_names.sort_unstable();
            let total_file_count = file_names.len();
            let truncated = total_file_count > 120;
            if truncated {
                file_names.truncate(120);
            }
            let total_bytes: usize = files
                .values()
                .filter_map(|v| v.as_str())
                .map(|s| s.len())
                .sum();

            summary.insert(
                "file_count".to_string(),
                serde_json::json!(total_file_count),
            );
            summary.insert("file_names".to_string(), serde_json::json!(file_names));
            summary.insert("file_bytes".to_string(), serde_json::json!(total_bytes));
            if truncated {
                summary.insert("file_names_truncated".to_string(), serde_json::json!(true));
            }
        }

        serde_json::Value::Object(summary)
    }

    fn extract_output_route_components(url: &str) -> Option<(String, String)> {
        let path = if url.starts_with("http://") || url.starts_with("https://") {
            match reqwest::Url::parse(url) {
                Ok(parsed) => parsed.path().to_string(),
                Err(_) => return None,
            }
        } else {
            url.to_string()
        };
        let marker = "/api/outputs/";
        let idx = path.find(marker)?;
        let tail = &path[idx + marker.len()..];
        let mut parts = tail.splitn(2, '/');
        let exec_id = parts.next()?.trim().to_string();
        let filename = parts.next()?.trim().to_string();
        if exec_id.is_empty() || filename.is_empty() {
            return None;
        }
        let filename = match urlencoding::decode(&filename) {
            Ok(v) => v.to_string(),
            Err(_) => filename,
        };
        Some((exec_id, filename))
    }

    async fn load_video_bytes(&self, source_url: &str, max_bytes: usize) -> Result<Vec<u8>> {
        if source_url.starts_with("data:") {
            if let Some(comma_idx) = source_url.find(',') {
                let (meta, payload) = source_url.split_at(comma_idx);
                let payload = &payload[1..];
                if meta.contains(";base64") {
                    use base64::Engine;
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(payload.as_bytes())
                        .map_err(|e| anyhow::anyhow!("Failed to decode data URL video: {}", e))?;
                    if bytes.len() > max_bytes {
                        anyhow::bail!(
                            "Video too large for channel delivery: {} bytes (max {})",
                            bytes.len(),
                            max_bytes
                        );
                    }
                    return Ok(bytes);
                }
            }
            anyhow::bail!("Unsupported data URL video format");
        }

        if let Some((exec_id, filename)) = Self::extract_output_route_components(source_url) {
            if uuid::Uuid::parse_str(&exec_id).is_ok()
                && !filename.contains('/')
                && !filename.contains('\\')
                && !filename.contains("..")
            {
                let path = self.data_dir.join("outputs").join(exec_id).join(filename);
                let bytes = tokio::fs::read(&path).await?;
                if bytes.len() > max_bytes {
                    anyhow::bail!(
                        "Video too large for channel delivery: {} bytes (max {})",
                        bytes.len(),
                        max_bytes
                    );
                }
                return Ok(bytes);
            }
        }

        if source_url.starts_with("http://") || source_url.starts_with("https://") {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(90))
                .build()?;
            let resp = client.get(source_url).send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("Failed to fetch video URL (status {})", resp.status());
            }
            if let Some(len) = resp.content_length() {
                if len > max_bytes as u64 {
                    anyhow::bail!(
                        "Video too large for channel delivery: {} bytes (max {})",
                        len,
                        max_bytes
                    );
                }
            }
            let bytes = resp.bytes().await?.to_vec();
            if bytes.len() > max_bytes {
                anyhow::bail!(
                    "Video too large for channel delivery: {} bytes (max {})",
                    bytes.len(),
                    max_bytes
                );
            }
            return Ok(bytes);
        }

        anyhow::bail!("Unsupported video URL format for delivery")
    }

    async fn extract_video_preview_from_bytes(&self, video_bytes: &[u8]) -> Result<Vec<u8>> {
        let temp_dir = std::env::temp_dir().join(format!("video-preview-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&temp_dir).await?;
        let input_path = temp_dir.join("input.mp4");
        let output_path = temp_dir.join("preview.jpg");
        tokio::fs::write(&input_path, video_bytes).await?;

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(45),
            tokio::process::Command::new("ffmpeg")
                .args([
                    "-y",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-ss",
                    "00:00:00.500",
                    "-i",
                    &input_path.to_string_lossy(),
                    "-frames:v",
                    "1",
                    "-vf",
                    "scale=960:-1",
                    &output_path.to_string_lossy(),
                ])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("ffmpeg preview extraction timed out"))??;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("ffmpeg preview extraction failed: {}", stderr);
        }
        let preview = tokio::fs::read(&output_path).await?;
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        Ok(preview)
    }

    async fn build_app_runtime_failure_hint(&self, app_id: &str) -> Option<String> {
        if self.app_registry.is_static(app_id).await {
            return None;
        }
        let app_dir = self.app_registry.get_dir(app_id).await?;
        let current_port = self.app_registry.get_port(app_id).await;
        let log_tail = crate::actions::app::read_local_runtime_log_tail(&app_dir, 4096).await;

        if current_port.is_none() {
            if log_tail.is_empty() {
                return Some(
                    "Dynamic app runtime is not active (process/container likely exited)."
                        .to_string(),
                );
            }
            return Some(format!(
                "Dynamic app runtime is not active. Recent runtime logs:\n{}",
                log_tail
            ));
        }

        if log_tail.is_empty() {
            None
        } else {
            Some(format!("Recent runtime logs:\n{}", log_tail))
        }
    }

    fn detect_app_runtime_error_marker(content: &str) -> Option<&'static str> {
        let needles: [(&str, &str); 8] = [
            ("error loading", "error loading"),
            ("failed to load", "failed to load"),
            ("something went wrong", "something went wrong"),
            ("application error", "application error"),
            ("could not fetch", "could not fetch"),
            ("unable to fetch", "unable to fetch"),
            ("please try again", "please try again"),
            ("runtime error", "runtime error"),
        ];
        for (needle, label) in needles {
            if content.contains(needle) {
                return Some(label);
            }
        }
        None
    }

    // App deploy self-heal is currently parked, but keep its helpers available
    // for the planned re-enable path without widening dead-code allowances.
    #[allow(dead_code)]
    fn app_deploy_files_signature(arguments: &serde_json::Value) -> Option<String> {
        let files = arguments.get("files")?;
        let canonical = Self::canonicalize_json_value(files);
        serde_json::to_string(&canonical).ok()
    }

    fn resolve_duplicate_app(match_kind: &str, existing_running: bool) -> DuplicateAppResolution {
        if match_kind == "exact_files" && existing_running {
            DuplicateAppResolution::ReuseExisting
        } else {
            DuplicateAppResolution::ReplaceExisting
        }
    }

    async fn stop_and_remove_existing_app(
        &self,
        app_id: &str,
        app_title: Option<&str>,
    ) -> Result<()> {
        if app_id.is_empty()
            || app_id.len() > 64
            || !app_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            anyhow::bail!("refusing to remove invalid app id '{}'", app_id);
        }

        let app_dir = self
            .app_registry
            .get_dir(app_id)
            .await
            .unwrap_or_else(|| self.data_dir.join("apps").join(app_id));

        self.app_registry.stop(app_id).await?;

        match tokio::fs::remove_dir_all(&app_dir).await {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                anyhow::bail!(
                    "failed to remove app directory '{}': {}",
                    app_dir.display(),
                    error
                );
            }
        }

        if let Err(error) = self
            .storage
            .delete_app_notifications(app_id, app_title)
            .await
        {
            tracing::warn!(
                "failed to delete app notifications during replacement for {}: {}",
                app_id,
                error
            );
        }

        Ok(())
    }

    fn normalize_app_title(value: &str) -> String {
        value
            .to_ascii_lowercase()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c.is_ascii_whitespace() {
                    c
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn is_generic_title(title: &str) -> bool {
        let mut token_count = 0usize;
        let generic = [
            "app",
            "dashboard",
            "tool",
            "site",
            "website",
            "web",
            "page",
            "project",
            "demo",
        ];
        for token in title.split_whitespace() {
            token_count += 1;
            if !generic.iter().any(|g| g == &token) {
                return false;
            }
        }
        token_count > 0
    }

    fn is_probably_text_bytes(bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return false;
        }
        let sample_len = std::cmp::min(bytes.len(), 4096);
        let sample = &bytes[..sample_len];
        if sample.contains(&0) {
            return false;
        }
        let control_count = sample
            .iter()
            .filter(|b| {
                let c = **b;
                c < 0x20 && c != b'\n' && c != b'\r' && c != b'\t'
            })
            .count();
        (control_count as f32 / sample_len as f32) <= 0.12
    }

    fn extract_semantic_excerpt_from_bytes(bytes: &[u8], max_chars: usize) -> Option<String> {
        if max_chars == 0 || !Self::is_probably_text_bytes(bytes) {
            return None;
        }
        let excerpt = String::from_utf8_lossy(bytes)
            .chars()
            .take(max_chars)
            .collect::<String>();
        if excerpt.trim().is_empty() {
            None
        } else {
            Some(excerpt)
        }
    }

    fn app_token_stopword(token: &str) -> bool {
        matches!(
            token,
            "the"
                | "and"
                | "for"
                | "with"
                | "from"
                | "into"
                | "this"
                | "that"
                | "are"
                | "was"
                | "were"
                | "you"
                | "your"
                | "http"
                | "https"
                | "www"
                | "com"
                | "org"
                | "net"
                | "api"
                | "app"
                | "apps"
                | "dashboard"
                | "tool"
                | "page"
                | "static"
                | "dynamic"
        )
    }

    fn append_semantic_tokens(
        target: &mut std::collections::HashSet<String>,
        text: &str,
        max_tokens: usize,
    ) {
        if target.len() >= max_tokens {
            return;
        }
        for raw in text.split(|c: char| !c.is_ascii_alphanumeric()) {
            let token = raw.trim().to_ascii_lowercase();
            if token.len() < 3 || Self::app_token_stopword(&token) {
                continue;
            }
            target.insert(token);
            if target.len() >= max_tokens {
                break;
            }
        }
    }

    fn jaccard_similarity(
        left: &std::collections::HashSet<String>,
        right: &std::collections::HashSet<String>,
    ) -> f32 {
        if left.is_empty() || right.is_empty() {
            return 0.0;
        }
        let inter = left.intersection(right).count() as f32;
        let union = left.union(right).count() as f32;
        if union <= f32::EPSILON {
            0.0
        } else {
            inter / union
        }
    }

    fn compact_app_lookup_key(value: &str) -> String {
        value
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect()
    }

    fn should_skip_app_inventory_dir(name: &str) -> bool {
        matches!(
            name.trim().to_ascii_lowercase().as_str(),
            ".git"
                | ".hg"
                | ".svn"
                | "__pycache__"
                | ".mypy_cache"
                | ".pytest_cache"
                | ".ruff_cache"
                | ".turbo"
                | ".next"
                | ".venv"
                | "node_modules"
                | "_deps"
                | "target"
        )
    }

    fn preferred_app_file_rank(path: &str) -> usize {
        let lower = path.trim().replace('\\', "/").to_ascii_lowercase();
        match lower.as_str() {
            ".app_meta.json" => 0,
            "app.py" => 1,
            "main.py" => 2,
            "server.py" => 3,
            "server.js" | "server.ts" => 4,
            "index.html" => 5,
            "package.json" => 6,
            "requirements.txt" => 7,
            "pyproject.toml" => 8,
            "vite.config.ts" | "vite.config.js" => 9,
            "src/main.tsx" | "src/main.jsx" => 10,
            "src/app.tsx" | "src/app.jsx" => 11,
            "src/index.tsx" | "src/index.jsx" => 12,
            "readme.md" => 13,
            _ if lower.ends_with("/app.py") => 14,
            _ if lower.ends_with("/main.py") => 15,
            _ if lower.ends_with("/server.py") || lower.ends_with("/server.js") => 16,
            _ if lower.ends_with("/index.html") => 17,
            _ if lower.ends_with("/package.json") => 18,
            _ if lower.ends_with("/requirements.txt") => 19,
            _ if lower.ends_with(".html") => 30,
            _ if lower.ends_with(".py") => 31,
            _ if lower.ends_with(".ts") || lower.ends_with(".tsx") => 32,
            _ if lower.ends_with(".js") || lower.ends_with(".jsx") => 33,
            _ if lower.ends_with(".css") => 34,
            _ => 100,
        }
    }

    fn score_deployed_app_match(query: &str, app_id: &str, title: &str) -> Option<(f32, String)> {
        let query = query.trim();
        if query.is_empty() {
            return None;
        }
        let normalized_query = Self::normalize_app_title(query);
        let normalized_title = Self::normalize_app_title(title);
        let compact_query = Self::compact_app_lookup_key(query);
        let compact_title = Self::compact_app_lookup_key(title);
        let compact_id = Self::compact_app_lookup_key(app_id);

        if !compact_query.is_empty() && compact_query == compact_id {
            return Some((1.0, "exact_id".to_string()));
        }
        if !normalized_query.is_empty() && normalized_query == normalized_title {
            return Some((0.99, "exact_title".to_string()));
        }
        if !compact_query.is_empty() && compact_id.contains(&compact_query) {
            return Some((0.94, "id_substring".to_string()));
        }
        if !compact_query.is_empty() && compact_title.contains(&compact_query) {
            return Some((0.92, "title_substring".to_string()));
        }

        let mut query_tokens = std::collections::HashSet::new();
        Self::append_semantic_tokens(&mut query_tokens, &normalized_query, 18);
        let mut app_tokens = std::collections::HashSet::new();
        Self::append_semantic_tokens(&mut app_tokens, &normalized_title, 24);
        Self::append_semantic_tokens(&mut app_tokens, app_id, 28);
        if query_tokens.is_empty() || app_tokens.is_empty() {
            return None;
        }
        let overlap = Self::jaccard_similarity(&query_tokens, &app_tokens);
        if overlap < 0.20 {
            None
        } else {
            Some((0.35 + (overlap * 0.55), "token_overlap".to_string()))
        }
    }

    fn rank_deployed_apps(
        query: &str,
        apps: &[serde_json::Value],
    ) -> Vec<(f32, String, serde_json::Value, String)> {
        let mut ranked_apps: Vec<(f32, String, serde_json::Value, String)> = apps
            .iter()
            .map(|app| {
                let app_id = app
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let title = app
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("App")
                    .to_string();
                let (score, reason) = if query.is_empty() {
                    (0.0, "listed".to_string())
                } else {
                    Self::score_deployed_app_match(query, &app_id, &title)
                        .unwrap_or((0.0, "no_match".to_string()))
                };
                (score, app_id, app.clone(), reason)
            })
            .collect();
        ranked_apps.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.cmp(&b.1))
        });
        ranked_apps
    }

    fn select_best_ranked_app<'a>(
        query: &str,
        ranked_apps: &'a [(f32, String, serde_json::Value, String)],
    ) -> Option<&'a (f32, String, serde_json::Value, String)> {
        if query.is_empty() {
            return if ranked_apps.len() == 1 {
                ranked_apps.first()
            } else {
                None
            };
        }

        ranked_apps.first().filter(|(score, _, _, _)| {
            let next_score = ranked_apps.get(1).map(|row| row.0).unwrap_or(0.0);
            *score >= 0.55 || (*score >= 0.30 && (*score - next_score) >= 0.10)
        })
    }

    async fn collect_app_file_inventory(
        &self,
        app_dir: &std::path::Path,
        max_files: usize,
    ) -> (Vec<serde_json::Value>, usize, u64, bool) {
        let root = app_dir.to_path_buf();
        let capped_max = max_files.clamp(1, 200);
        (tokio::task::spawn_blocking(move || {
            let mut rows: Vec<(usize, String, u64)> = Vec::new();
            let mut total_files = 0usize;
            let mut total_bytes = 0u64;

            let walker = walkdir::WalkDir::new(&root)
                .into_iter()
                .filter_entry(|entry| {
                    if !entry.file_type().is_dir() {
                        return true;
                    }
                    entry
                        .file_name()
                        .to_str()
                        .map(|name| !Self::should_skip_app_inventory_dir(name))
                        .unwrap_or(true)
                });

            for entry in walker {
                let Ok(entry) = entry else {
                    continue;
                };
                if !entry.file_type().is_file() {
                    continue;
                }
                total_files += 1;
                let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
                total_bytes = total_bytes.saturating_add(len);
                let relative = entry
                    .path()
                    .strip_prefix(&root)
                    .unwrap_or(entry.path())
                    .to_string_lossy()
                    .replace('\\', "/");
                rows.push((Self::preferred_app_file_rank(&relative), relative, len));
            }

            rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            let truncated = rows.len() > capped_max;
            let files = rows
                .into_iter()
                .take(capped_max)
                .map(|(_, path, bytes)| serde_json::json!({ "path": path, "bytes": bytes }))
                .collect::<Vec<_>>();
            (files, total_files, total_bytes, truncated)
        })
        .await)
            .unwrap_or_default()
    }

    async fn build_deployed_app_inspection(
        &self,
        app: &serde_json::Value,
        include_files: bool,
        include_logs: bool,
    ) -> Option<serde_json::Value> {
        let app_id = app
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())?;
        let title = app
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("App")
            .to_string();
        let app_dir = self.app_registry.get_dir(app_id).await?;
        let meta_path = app_dir.join(".app_meta.json");
        let meta: Option<serde_json::Value> = tokio::fs::read(&meta_path)
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());

        let local_base = Self::user_facing_local_base_url();
        let relative_url = app.get("url").and_then(|v| v.as_str()).unwrap_or("/apps/");
        let local_url = Self::absolutize_public_url(Some(local_base.as_str()), relative_url);
        let access_guard_enabled = app
            .get("access_guard_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let required_inputs = meta
            .as_ref()
            .map(crate::actions::app::parse_required_inputs)
            .unwrap_or_default()
            .into_iter()
            .map(|item| {
                serde_json::json!({
                    "key": item.key,
                    "sensitive": item.sensitive,
                })
            })
            .collect::<Vec<_>>();
        let config_keys = meta
            .as_ref()
            .and_then(|m| m.get("config_values").and_then(|v| v.as_object()))
            .map(|obj| obj.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        let (files, file_count, file_bytes, file_list_truncated) = if include_files {
            self.collect_app_file_inventory(&app_dir, 48).await
        } else {
            (Vec::new(), 0, 0, false)
        };
        let suggested_read_files = files
            .iter()
            .filter_map(|row| row.get("path").and_then(|v| v.as_str()))
            .take(8)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let recent_runtime_logs = if include_logs
            && !app
                .get("is_static")
                .and_then(|v| v.as_bool())
                .unwrap_or(true)
        {
            let tail = crate::actions::app::read_local_runtime_log_tail(&app_dir, 4096).await;
            if tail.trim().is_empty() {
                None
            } else {
                Some(tail)
            }
        } else {
            None
        };

        let mut out = serde_json::json!({
            "id": app_id,
            "title": title,
            "app_dir": app_dir.to_string_lossy().to_string(),
            "metadata_path": meta_path.to_string_lossy().to_string(),
          "local_url": local_url,
          "running": app.get("running").and_then(|v| v.as_bool()).unwrap_or(false),
          "is_static": app.get("is_static").and_then(|v| v.as_bool()).unwrap_or(true),
          "runtime_mode": app.get("runtime_mode").and_then(|v| v.as_str()).unwrap_or("unknown"),
          "created_at": app.get("created_at").and_then(|v| v.as_str()).unwrap_or(""),
          "port": app.get("port").and_then(|v| v.as_u64()),
          "access_guard_enabled": access_guard_enabled,
          "entry_command": meta.as_ref().and_then(|m| m.get("entry_command").and_then(|v| v.as_str())),
          "install_command": meta.as_ref().and_then(|m| m.get("install_command").and_then(|v| v.as_str())),
          "runtime_preference": meta.as_ref().and_then(|m| m.get("runtime_preference").and_then(|v| v.as_str())),
            "runtime_image": meta.as_ref().and_then(|m| m.get("runtime_image").and_then(|v| v.as_str())),
            "required_inputs": required_inputs,
            "config_keys": config_keys,
            "file_count": file_count,
            "file_bytes": file_bytes,
            "suggested_read_files": suggested_read_files,
            "suggested_actions": ["file_read", "file_write", "app_restart", "http_get"],
        });
        if let Some(obj) = out.as_object_mut() {
            if include_files {
                obj.insert("files".to_string(), serde_json::json!(files));
                obj.insert(
                    "file_list_truncated".to_string(),
                    serde_json::json!(file_list_truncated),
                );
            }
            if let Some(log_tail) = recent_runtime_logs {
                obj.insert(
                    "recent_runtime_logs".to_string(),
                    serde_json::json!(log_tail),
                );
            }
        }
        Some(out)
    }

    fn sample_overlap_tokens(
        left: &std::collections::HashSet<String>,
        right: &std::collections::HashSet<String>,
        max_items: usize,
    ) -> Vec<String> {
        let mut overlap: Vec<String> = left.intersection(right).cloned().collect();
        overlap.sort_unstable();
        overlap.into_iter().take(max_items).collect()
    }

    fn build_requested_app_fingerprint(
        arguments: &serde_json::Value,
    ) -> Option<AppSemanticFingerprint> {
        let requested_files = arguments.get("files").and_then(|v| v.as_object())?;
        let requested_title = arguments
            .get("title")
            .and_then(|v| v.as_str())
            .map(Self::normalize_app_title)
            .unwrap_or_default();
        let is_static = arguments
            .get("entry_command")
            .and_then(|v| v.as_str())
            .is_none();

        let mut title_tokens = std::collections::HashSet::new();
        Self::append_semantic_tokens(&mut title_tokens, &requested_title, 24);

        let mut file_tokens = std::collections::HashSet::new();
        let mut keyword_tokens = std::collections::HashSet::new();
        for (path, value) in requested_files {
            Self::append_semantic_tokens(&mut file_tokens, path, 80);
            if let Some(content) = value.as_str() {
                // Bound extraction cost on very large generated files.
                let excerpt = content.chars().take(20_000).collect::<String>();
                Self::append_semantic_tokens(&mut keyword_tokens, &excerpt, 420);
            }
        }

        Some(AppSemanticFingerprint {
            title_tokens,
            keyword_tokens,
            file_tokens,
            is_static,
        })
    }

    async fn build_existing_app_fingerprint(
        &self,
        app_id: &str,
        app: &serde_json::Value,
    ) -> Option<AppSemanticFingerprint> {
        let app_dir = self.app_registry.get_dir(app_id).await?;
        let title = app
            .get("title")
            .and_then(|v| v.as_str())
            .map(Self::normalize_app_title)
            .unwrap_or_default();
        let is_static = app
            .get("is_static")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let mut title_tokens = std::collections::HashSet::new();
        Self::append_semantic_tokens(&mut title_tokens, &title, 24);

        let mut file_tokens = std::collections::HashSet::new();
        let mut keyword_tokens = std::collections::HashSet::new();
        let mut dirs = vec![app_dir.clone()];
        let mut files_seen = 0usize;
        let mut char_budget = 120_000usize;

        while let Some(dir) = dirs.pop() {
            let mut entries = match tokio::fs::read_dir(&dir).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let metadata = match entry.metadata().await {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if metadata.is_dir() {
                    dirs.push(path);
                    continue;
                }
                files_seen += 1;
                if files_seen > 64 {
                    break;
                }
                let relative = path
                    .strip_prefix(&app_dir)
                    .unwrap_or(path.as_path())
                    .to_string_lossy()
                    .replace('\\', "/");
                Self::append_semantic_tokens(&mut file_tokens, &relative, 120);
                if char_budget == 0 {
                    continue;
                }
                if metadata.len() > 1_000_000 {
                    continue;
                }
                let Ok(content) = tokio::fs::read(&path).await else {
                    continue;
                };
                let take_chars = std::cmp::min(char_budget, 24_000);
                let Some(excerpt) = Self::extract_semantic_excerpt_from_bytes(&content, take_chars)
                else {
                    continue;
                };
                char_budget = char_budget.saturating_sub(excerpt.chars().count());
                Self::append_semantic_tokens(&mut keyword_tokens, &excerpt, 520);
            }
        }

        Some(AppSemanticFingerprint {
            title_tokens,
            keyword_tokens,
            file_tokens,
            is_static,
        })
    }

    fn score_app_similarity(
        requested: &AppSemanticFingerprint,
        existing: &AppSemanticFingerprint,
    ) -> (f32, String) {
        let title_score = Self::jaccard_similarity(&requested.title_tokens, &existing.title_tokens);
        let keyword_score =
            Self::jaccard_similarity(&requested.keyword_tokens, &existing.keyword_tokens);
        let file_score = Self::jaccard_similarity(&requested.file_tokens, &existing.file_tokens);
        let runtime_bonus = if requested.is_static == existing.is_static {
            0.05
        } else {
            0.0
        };
        let score =
            (0.35 * title_score) + (0.40 * keyword_score) + (0.20 * file_score) + runtime_bonus;
        let overlaps =
            Self::sample_overlap_tokens(&requested.keyword_tokens, &existing.keyword_tokens, 5);
        let overlap_text = if overlaps.is_empty() {
            "no strong shared keywords".to_string()
        } else {
            format!("shared keywords: {}", overlaps.join(", "))
        };
        let reason = format!(
            "{} | title {:.0}%, content {:.0}%, files {:.0}%",
            overlap_text,
            title_score * 100.0,
            keyword_score * 100.0,
            file_score * 100.0
        );
        (score, reason)
    }

    async fn app_files_match_existing(
        &self,
        app_id: &str,
        requested_files: &serde_json::Map<String, serde_json::Value>,
    ) -> bool {
        if requested_files.is_empty() {
            return false;
        }
        let Some(app_dir) = self.app_registry.get_dir(app_id).await else {
            return false;
        };
        for (relative_path, content_value) in requested_files {
            let Some(expected) = content_value.as_str() else {
                return false;
            };
            if relative_path.contains("..")
                || relative_path.starts_with('/')
                || relative_path.starts_with('\\')
            {
                return false;
            }
            let file_path = app_dir.join(relative_path);
            let actual = match tokio::fs::read_to_string(&file_path).await {
                Ok(v) => v,
                Err(_) => return false,
            };
            if actual != expected {
                return false;
            }
        }
        true
    }

    async fn find_existing_duplicate_app(
        &self,
        arguments: &serde_json::Value,
    ) -> Option<AppDuplicateMatch> {
        let requested_title = arguments
            .get("title")
            .and_then(|v| v.as_str())
            .map(Self::normalize_app_title)
            .unwrap_or_default();
        let requested_files = arguments.get("files").and_then(|v| v.as_object())?;
        let requested_fingerprint = Self::build_requested_app_fingerprint(arguments)?;
        let apps = self.app_registry.list().await;
        let mut best_fuzzy: Option<AppDuplicateMatch> = None;
        const SIMILARITY_THRESHOLD: f32 = 0.58;

        for app in apps {
            let app_id = app
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("");
            if app_id.is_empty() {
                continue;
            }
            let existing_title = app
                .get("title")
                .and_then(|v| v.as_str())
                .map(Self::normalize_app_title)
                .unwrap_or_default();
            let title_match = !requested_title.is_empty()
                && !existing_title.is_empty()
                && !Self::is_generic_title(&requested_title)
                && requested_title == existing_title;
            let files_match = self.app_files_match_existing(app_id, requested_files).await;
            if files_match {
                return Some(AppDuplicateMatch {
                    app,
                    match_kind: "exact_files",
                    score: 1.0,
                    reason: "files are identical".to_string(),
                });
            }
            if title_match {
                return Some(AppDuplicateMatch {
                    app,
                    match_kind: "exact_title",
                    score: 0.92,
                    reason: "title matches exactly".to_string(),
                });
            }

            let Some(existing_fingerprint) =
                self.build_existing_app_fingerprint(app_id, &app).await
            else {
                continue;
            };
            let (score, reason) =
                Self::score_app_similarity(&requested_fingerprint, &existing_fingerprint);
            if score < SIMILARITY_THRESHOLD {
                continue;
            }
            let should_replace = best_fuzzy.as_ref().map(|m| score > m.score).unwrap_or(true);
            if should_replace {
                best_fuzzy = Some(AppDuplicateMatch {
                    app,
                    match_kind: "fuzzy",
                    score,
                    reason,
                });
            }
        }

        best_fuzzy
    }

    #[allow(dead_code)]
    async fn build_app_deploy_self_heal_arguments(
        &self,
        current_args: &serde_json::Value,
        validation_issue: &str,
        request_channel: &str,
        attempt: usize,
        max_attempts: usize,
    ) -> Result<serde_json::Value> {
        let context = serde_json::json!({
            "title": current_args.get("title"),
            "entry_command": current_args.get("entry_command"),
            "install_command": current_args.get("install_command"),
            "runtime_image": current_args.get("runtime_image"),
            "runtime_preference": current_args.get("runtime_preference"),
            "required_inputs": current_args.get("required_inputs"),
            "config": current_args.get("config"),
            "files": current_args.get("files"),
        });
        let context_json = serde_json::to_string(&context).unwrap_or_default();
        if context_json.len() > 180_000 {
            anyhow::bail!(
                "app payload too large for auto-fix prompt ({} chars)",
                context_json.len()
            );
        }

        let system_prompt = "You repair broken deployed apps. Return ONLY a JSON object. No markdown, no explanations.";
        let user_prompt = format!(
            "A deployed app failed runtime validation.\n\
Attempt {}/{}.\n\
Validation issue:\n{}\n\n\
Current app_deploy context (JSON):\n{}\n\n\
Return a JSON object with at least a complete `files` object.\n\
Optional keys you may include when needed: `entry_command`, `install_command`, `runtime_image`, `runtime_preference`, `required_inputs`, `config`.\n\
Do not include any extra prose.",
            attempt,
            max_attempts,
            validation_issue.trim(),
            context_json
        );

        let empty_actions: Vec<crate::actions::ActionDef> = Vec::new();
        let repair = self
            .llm
            .chat(system_prompt, &user_prompt, &[], &empty_actions)
            .await?;
        self.record_llm_usage(request_channel, "app_deploy_self_heal", &repair)
            .await;

        let parsed = Self::parse_json_object_str(&repair.content)
            .ok_or_else(|| anyhow::anyhow!("self-heal model returned non-JSON response"))?;
        let normalized = Self::normalize_app_deploy_arguments(&parsed);
        let Some(patch_obj) = normalized.as_object() else {
            anyhow::bail!("self-heal patch was not a JSON object");
        };
        let Some(patch_files) = patch_obj.get("files").and_then(|v| v.as_object()) else {
            anyhow::bail!("self-heal patch missing `files` object");
        };
        if patch_files.is_empty() {
            anyhow::bail!("self-heal patch returned empty `files`");
        }

        let current_sig = Self::app_deploy_files_signature(current_args).unwrap_or_default();
        let next_sig = serde_json::to_string(&Self::canonicalize_json_value(
            &serde_json::Value::Object(patch_files.clone()),
        ))
        .unwrap_or_default();
        if !current_sig.is_empty() && current_sig == next_sig {
            anyhow::bail!("self-heal patch did not change files");
        }

        let mut merged = current_args.clone();
        let Some(merged_obj) = merged.as_object_mut() else {
            anyhow::bail!("current app args are not an object");
        };
        merged_obj.insert(
            "files".to_string(),
            serde_json::Value::Object(patch_files.clone()),
        );
        for key in [
            "entry_command",
            "install_command",
            "runtime_image",
            "runtime_preference",
            "required_inputs",
            "config",
        ] {
            if let Some(value) = patch_obj.get(key) {
                merged_obj.insert(key.to_string(), value.clone());
            }
        }

        Ok(merged)
    }

    async fn validate_and_capture_app_preview(
        &self,
        app_url_with_key: &str,
        app_id: &str,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Result<(Option<String>, bool, usize, String)> {
        const MAX_APP_VERIFY_ATTEMPTS: usize = 3;
        let sidecar_available = self.browser_sessions.is_available().await;
        let integration = if sidecar_available {
            Some(self.browser_sessions.integration().clone())
        } else {
            None
        };
        let http_client = Self::build_internal_control_client().ok();
        let internal_probe_url = if app_url_with_key.starts_with("http://")
            || app_url_with_key.starts_with("https://")
        {
            app_url_with_key.to_string()
        } else if app_url_with_key.starts_with('/') {
            format!("{}{}", Self::internal_api_base_url(), app_url_with_key)
        } else {
            format!("{}/{}", Self::internal_api_base_url(), app_url_with_key)
        };

        if !sidecar_available && http_client.is_none() {
            return Ok((
                None,
                false,
                0,
                "No validation backends available (browser sidecar + HTTP probe unavailable)"
                    .to_string(),
            ));
        }
        let mut last_error = "Unknown validation error".to_string();

        for attempt in 1..=MAX_APP_VERIFY_ATTEMPTS {
            if let Some(tx) = stream_tx {
                let _ = tx.try_send(StreamEvent::ToolProgress {
                    name: "app_deploy".to_string(),
                    content: format!(
                        "Validating deployed app (attempt {}/{})",
                        attempt, MAX_APP_VERIFY_ATTEMPTS
                    ),
                    payload: None,
                });
            }

            // Primary readiness signal: direct HTTP probe to the deployed app URL.
            if let Some(client) = &http_client {
                match client.get(&internal_probe_url).send().await {
                    Ok(resp) if !resp.status().is_server_error() => {
                        let status = resp.status();
                        if let Some(integration) = &integration {
                            let sidecar_session = match integration.create_session().await {
                                Ok(s) => s,
                                Err(e) => {
                                    return Ok((
                                        None,
                                        true,
                                        attempt,
                                        format!(
                                            "HTTP probe passed on attempt {} (status {}, preview unavailable: create_session failed: {})",
                                            attempt, status, e
                                        ),
                                    ));
                                }
                            };

                            let preview_result: Result<String> = async {
                                let _ = integration
                                    .navigate(&sidecar_session, app_url_with_key)
                                    .await?;
                                tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
                                let content = integration.get_content(&sidecar_session).await?;
                                let combined = format!("{}\n{}", content.title, content.body_text)
                                    .to_lowercase();
                                let lock_page_detected = combined.contains("access key required")
                                    || (combined.contains("enter access key")
                                        && combined.contains("unlock"));
                                if lock_page_detected {
                                    anyhow::bail!("app opened in locked mode");
                                }
                                if let Some(marker) =
                                    Self::detect_app_runtime_error_marker(&combined)
                                {
                                    anyhow::bail!(
                                        "app page reports runtime error marker: {}",
                                        marker
                                    );
                                }
                                let screenshot = integration.screenshot(&sidecar_session).await?;
                                if screenshot.is_empty() {
                                    anyhow::bail!("empty screenshot returned");
                                }
                                self.persist_app_preview_screenshot(app_id, &screenshot)
                                    .await
                            }
                            .await;

                            let _ = integration.close_session(&sidecar_session).await;
                            match preview_result {
                                Ok(screenshot_url) => {
                                    return Ok((
                                        Some(screenshot_url),
                                        true,
                                        attempt,
                                        format!(
                                            "HTTP probe + screenshot validation passed on attempt {} (status {})",
                                            attempt, status
                                        ),
                                    ));
                                }
                                Err(e) => {
                                    let err_text = e.to_string();
                                    if err_text.contains("runtime error marker")
                                        || err_text.contains("locked mode")
                                    {
                                        last_error = err_text;
                                        tokio::time::sleep(std::time::Duration::from_millis(500))
                                            .await;
                                        continue;
                                    }
                                    return Ok((
                                        None,
                                        true,
                                        attempt,
                                        format!(
                                            "HTTP probe passed on attempt {} (status {}, preview unavailable: {})",
                                            attempt, status, err_text
                                        ),
                                    ));
                                }
                            }
                        }

                        if let Ok(body) = resp.text().await {
                            let lower = body.to_lowercase();
                            if let Some(marker) = Self::detect_app_runtime_error_marker(&lower) {
                                last_error = format!(
                                    "HTTP probe body reports runtime error marker: {}",
                                    marker
                                );
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                continue;
                            }
                        }

                        return Ok((
                            None,
                            true,
                            attempt,
                            format!(
                                "HTTP probe passed on attempt {} (status {}, browser sidecar unavailable)",
                                attempt, status
                            ),
                        ));
                    }
                    Ok(resp) => {
                        last_error = format!("HTTP probe failed with status {}", resp.status());
                    }
                    Err(e) => {
                        last_error = format!("HTTP probe request failed: {}", e);
                    }
                }
            }

            // Fallback when HTTP probe is inconclusive: sidecar navigation/content validation.
            let Some(integration) = &integration else {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                continue;
            };

            let sidecar_session = match integration.create_session().await {
                Ok(s) => s,
                Err(e) => {
                    last_error = format!("create_session failed: {}", e);
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    continue;
                }
            };

            let attempt_result: Result<String> = async {
                let _ = integration
                    .navigate(&sidecar_session, app_url_with_key)
                    .await?;
                tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

                let content = integration.get_content(&sidecar_session).await?;
                let combined = format!("{}\n{}", content.title, content.body_text).to_lowercase();
                let lock_page_detected = combined.contains("access key required")
                    || (combined.contains("enter access key") && combined.contains("unlock"));
                if lock_page_detected {
                    anyhow::bail!("app opened in locked mode");
                }
                if let Some(marker) = Self::detect_app_runtime_error_marker(&combined) {
                    anyhow::bail!("app page reports runtime error marker: {}", marker);
                }

                let screenshot = integration.screenshot(&sidecar_session).await?;
                if screenshot.is_empty() {
                    anyhow::bail!("empty screenshot returned");
                }
                self.persist_app_preview_screenshot(app_id, &screenshot)
                    .await
            }
            .await;

            let _ = integration.close_session(&sidecar_session).await;

            match attempt_result {
                Ok(screenshot_url) => {
                    return Ok((
                        Some(screenshot_url),
                        true,
                        attempt,
                        format!("Validated on attempt {}", attempt),
                    ));
                }
                Err(e) => {
                    last_error = e.to_string();
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        }

        if let Some(runtime_hint) = self.build_app_runtime_failure_hint(app_id).await {
            last_error = format!("{}\n{}", last_error, runtime_hint);
        }
        Ok((None, false, MAX_APP_VERIFY_ATTEMPTS, last_error))
    }

    async fn append_moltbook_tool_activity(
        &self,
        sub_action: &str,
        args: &serde_json::Value,
        result: Option<&serde_json::Value>,
        error: Option<&str>,
    ) {
        let mut events: Vec<serde_json::Value> = self
            .storage
            .get(MOLTBOOK_ACTIVITY_LOG_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_slice::<Vec<serde_json::Value>>(&raw).ok())
            .unwrap_or_default();

        let urls = collect_moltbook_urls(sub_action, args, result);
        let action_kind = moltbook_action_kind(sub_action);

        let mut details = serde_json::json!({
            "source": "tool_call",
            "sub_action": sub_action,
            "action_kind": action_kind,
            "urls": urls
        });
        if let Some(post_id) = args.get("post_id").and_then(|v| v.as_str()) {
            details["post_id"] = serde_json::Value::String(post_id.to_string());
        }
        if let Some(submolt) = args.get("submolt").and_then(|v| v.as_str()) {
            details["submolt"] = serde_json::Value::String(submolt.to_string());
        }
        if let Some(query) = args.get("query").and_then(|v| v.as_str()) {
            details["query_preview"] = serde_json::Value::String(safe_truncate(query, 120));
        }
        if let Some(content) = args.get("content").and_then(|v| v.as_str()) {
            details["content_chars"] = serde_json::Value::from(content.chars().count() as u64);
            details["content_preview"] = serde_json::Value::String(safe_truncate(content, 220));
        }
        if let Some(title) = args.get("title").and_then(|v| v.as_str()) {
            details["title_preview"] = serde_json::Value::String(safe_truncate(title, 120));
        }
        if let Some(err) = error {
            details["error"] = serde_json::Value::String(safe_truncate(err, 300));
        }
        if let Some(post_id) = result
            .and_then(|r| r.get("post"))
            .and_then(|p| p.get("id"))
            .and_then(|v| v.as_str())
        {
            details["result_post_id"] = serde_json::Value::String(post_id.to_string());
        }

        events.push(serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "run_id": uuid::Uuid::new_v4().to_string(),
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "level": if error.is_some() { "error" } else { "info" },
            "action": format!("tool_{}", sub_action),
            "details": details
        }));
        if events.len() > MOLTBOOK_ACTIVITY_LOG_LIMIT {
            let drop = events.len() - MOLTBOOK_ACTIVITY_LOG_LIMIT;
            events.drain(0..drop);
        }
        if let Ok(bytes) = serde_json::to_vec(&events) {
            let _ = self.storage.set(MOLTBOOK_ACTIVITY_LOG_KEY, &bytes).await;
        }
    }

    async fn fire_action_hook(
        &self,
        trigger: crate::hooks::HookTrigger,
        channel: &str,
        action_name: &str,
        message_hint: Option<&str>,
        response: Option<&str>,
        event_id: &str,
    ) {
        self.hooks
            .fire(
                trigger.clone(),
                crate::hooks::HookContext {
                    event_id: Some(event_id.to_string()),
                    trigger: match trigger {
                        crate::hooks::HookTrigger::PreMessage => "pre_message".to_string(),
                        crate::hooks::HookTrigger::PostMessage => "post_message".to_string(),
                        crate::hooks::HookTrigger::PreAction => "pre_action".to_string(),
                        crate::hooks::HookTrigger::PostAction => "post_action".to_string(),
                        crate::hooks::HookTrigger::OnConsolidate => "on_consolidate".to_string(),
                        crate::hooks::HookTrigger::OnError => "on_error".to_string(),
                    },
                    channel: channel.to_string(),
                    message: message_hint.map(|m| safe_truncate(m, 500)),
                    response: response.map(|r| safe_truncate(r, 1500)),
                    action: Some(action_name.to_string()),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                },
            )
            .await;
    }

    pub(crate) async fn execute_action_with_hooks(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        channel: &str,
        message_hint: Option<&str>,
    ) -> Result<String> {
        let event_id = uuid::Uuid::new_v4().to_string();
        self.fire_action_hook(
            crate::hooks::HookTrigger::PreAction,
            channel,
            action_name,
            message_hint,
            None,
            &event_id,
        )
        .await;

        match self.runtime.execute_action(action_name, arguments).await {
            Ok(result) => {
                self.fire_action_hook(
                    crate::hooks::HookTrigger::PostAction,
                    channel,
                    action_name,
                    message_hint,
                    Some(&result),
                    &event_id,
                )
                .await;
                Ok(result)
            }
            Err(e) => {
                let err_text = e.to_string();
                self.fire_action_hook(
                    crate::hooks::HookTrigger::OnError,
                    channel,
                    action_name,
                    message_hint,
                    Some(&err_text),
                    &event_id,
                )
                .await;
                Err(e)
            }
        }
    }

    fn sanitize_stream_preview(&self, text: &str) -> String {
        let filtered = self.security.filter_output(text);
        safe_truncate(&filtered.text, 300)
    }

    async fn load_public_base_url(&self) -> Option<String> {
        let config_base = self
            .config
            .public_apps
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.trim_end_matches('/').to_string());
        if config_base.is_some() {
            return config_base;
        }
        if self.config.deployment_mode == crate::core::config::DeploymentMode::InternetFacing {
            if let Some(bind_addr) = self
                .config
                .public_apps
                .bind_addr
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let normalized = if bind_addr.starts_with("0.0.0.0:") {
                    format!("localhost:{}", bind_addr.trim_start_matches("0.0.0.0:"))
                } else if bind_addr == "0.0.0.0" {
                    "localhost".to_string()
                } else if bind_addr.starts_with("[::]:") {
                    format!("localhost:{}", bind_addr.trim_start_matches("[::]:"))
                } else if bind_addr == "[::]" || bind_addr == "::" {
                    "localhost".to_string()
                } else if bind_addr.starts_with("127.0.0.1:") || bind_addr == "127.0.0.1" {
                    bind_addr.replacen("127.0.0.1", "localhost", 1)
                } else {
                    bind_addr.to_string()
                };
                return Some(format!("http://{}", normalized.trim_end_matches('/')));
            }
        }

        self.storage
            .get("public_base_url")
            .await
            .ok()
            .flatten()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                std::env::var("AGENTARK_PUBLIC_BASE_URL")
                    .ok()
                    .map(|s| s.trim().trim_end_matches('/').to_string())
                    .filter(|s| !s.is_empty())
            })
    }

    fn has_configured_public_base_url(&self) -> bool {
        self.config
            .public_apps
            .base_url
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
            || (self.config.deployment_mode == crate::core::config::DeploymentMode::InternetFacing
                && self
                    .config
                    .public_apps
                    .bind_addr
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|value| !value.is_empty()))
    }

    async fn load_public_selected_app_id(&self) -> Option<String> {
        self.storage
            .get("public_selected_app_id")
            .await
            .ok()
            .flatten()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    fn internal_api_base_url() -> String {
        crate::core::net::internal_api_base_url()
    }

    fn user_facing_local_base_url() -> String {
        let internal = Self::internal_api_base_url();
        let Ok(mut parsed) = reqwest::Url::parse(&internal) else {
            return internal;
        };
        if let Some(host) = parsed.host_str() {
            let normalized = host.trim().to_ascii_lowercase();
            if normalized == "0.0.0.0" || normalized == "::" || normalized == "127.0.0.1" {
                let _ = parsed.set_host(Some("localhost"));
            }
        }
        parsed.to_string().trim_end_matches('/').to_string()
    }

    fn build_internal_control_client() -> Result<reqwest::Client> {
        crate::core::net::build_internal_control_client(5)
    }

    async fn ensure_public_tunnel_base_url(
        &self,
        app_id: Option<&str>,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Option<String> {
        if let Some(existing) = self.load_public_base_url().await {
            if self.has_configured_public_base_url() {
                return Some(existing);
            }
            let selected_app_id = self.load_public_selected_app_id().await;
            let requested_app_id = app_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string());
            if requested_app_id.is_none() || selected_app_id == requested_app_id {
                return Some(existing);
            }
        }
        let client = match Self::build_internal_control_client() {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!("Tunnel client init failed: {}", e);
                return None;
            }
        };
        let base_url = Self::internal_api_base_url();

        let mut start_req = client.post(format!("{}/tunnel/start", base_url));
        if let Some(key) = self.api_key.as_ref().filter(|k| !k.trim().is_empty()) {
            start_req = start_req.bearer_auth(key);
        }
        let requested_app_id = app_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let start_payload = match requested_app_id.as_deref() {
            Some(app_id) => serde_json::json!({ "app_id": app_id }),
            None => serde_json::json!({}),
        };
        start_req = start_req.json(&start_payload);
        let start_accepted = match start_req.send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    tracing::debug!("Tunnel start request returned {}", resp.status());
                    false
                } else {
                    true
                }
            }
            Err(e) => {
                tracing::debug!("Tunnel start request failed: {}", e);
                return None;
            }
        };

        if start_accepted {
            if let Some(tx) = stream_tx {
                let _ = tx.try_send(StreamEvent::ToolProgress {
                    name: "app_deploy".to_string(),
                    content: "Starting public tunnel for app access...".to_string(),
                    payload: None,
                });
            }
        } else {
            return self.load_public_base_url().await;
        }

        for _ in 0..10 {
            let mut status_req = client.get(format!("{}/tunnel/status", base_url));
            if let Some(key) = self.api_key.as_ref().filter(|k| !k.trim().is_empty()) {
                status_req = status_req.bearer_auth(key);
            }
            if let Ok(resp) = status_req.send().await {
                if resp.status().is_success() {
                    if let Ok(payload) = resp.json::<serde_json::Value>().await {
                        let selected_app_matches = match requested_app_id.as_deref() {
                            Some(app_id) => {
                                payload
                                    .get("selected_app_id")
                                    .and_then(|v| v.as_str())
                                    .map(str::trim)
                                    == Some(app_id)
                            }
                            None => true,
                        };
                        if let Some(url) = payload
                            .get("url")
                            .and_then(|v| v.as_str())
                            .map(|v| v.trim().trim_end_matches('/').to_string())
                            .filter(|v| !v.is_empty())
                        {
                            if !selected_app_matches {
                                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                continue;
                            }
                            let _ = self.storage.set("public_base_url", url.as_bytes()).await;
                            return Some(url);
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        if let Some(existing) = self.load_public_base_url().await {
            if self.has_configured_public_base_url() {
                return Some(existing);
            }
            let selected_app_id = self.load_public_selected_app_id().await;
            if requested_app_id.is_none() || selected_app_id == requested_app_id {
                return Some(existing);
            }
        }
        None
    }

    fn trigger_arkpulse_refresh(&self, reason: &'static str) {
        let api_key = self.api_key.clone();
        let base_url = Self::internal_api_base_url();
        tokio::spawn(async move {
            let client = match crate::core::net::build_internal_control_client(4) {
                Ok(c) => c,
                Err(e) => {
                    tracing::debug!("ArkPulse refresh client init failed: {}", e);
                    return;
                }
            };
            let mut req = client.post(format!("{}/arkpulse/trigger", base_url));
            if let Some(key) = api_key.as_ref().filter(|k| !k.trim().is_empty()) {
                req = req.bearer_auth(key);
            }
            match req.send().await {
                Ok(resp) => {
                    tracing::debug!(
                        "ArkPulse refresh trigger after {} returned {}",
                        reason,
                        resp.status()
                    );
                }
                Err(e) => {
                    tracing::debug!("ArkPulse refresh trigger after {} failed: {}", reason, e);
                }
            }
        });
    }

    fn absolutize_public_url(public_base_url: Option<&str>, url: &str) -> String {
        if url.starts_with("http://")
            || url.starts_with("https://")
            || url.starts_with("data:")
            || url.starts_with("blob:")
        {
            return url.to_string();
        }
        if let Some(base) = public_base_url {
            if url.starts_with('/') {
                return format!("{}{}", base, url);
            }
            return format!("{}/{}", base, url);
        }
        url.to_string()
    }

    fn default_tool_integration_aliases() -> HashMap<String, String> {
        let mut aliases = HashMap::new();
        aliases.insert("github".to_string(), "github".to_string());
        aliases.insert("notion".to_string(), "notion".to_string());
        aliases.insert("twitter".to_string(), "twitter".to_string());
        aliases.insert("onepassword".to_string(), "onepassword".to_string());
        aliases.insert("places".to_string(), "google_places".to_string());
        aliases.insert("twilio".to_string(), "twilio".to_string());
        aliases.insert("ordering".to_string(), "ordering".to_string());
        aliases.insert("garmin".to_string(), "garmin".to_string());
        aliases.insert("whoop".to_string(), "whoop".to_string());
        aliases.insert("ga4".to_string(), "ga4".to_string());
        aliases.insert("gsc".to_string(), "gsc".to_string());
        aliases.insert(
            "social_analytics".to_string(),
            "social_analytics".to_string(),
        );
        aliases.insert("moltbook".to_string(), "moltbook".to_string());
        aliases
    }

    fn merge_tool_integration_aliases(
        aliases: &mut HashMap<String, String>,
        value: &serde_json::Value,
    ) {
        let Some(obj) = value.as_object() else {
            return;
        };
        for (tool_name, integration_id_value) in obj {
            let Some(integration_id) = integration_id_value.as_str() else {
                continue;
            };
            let tool_name = tool_name.trim();
            let integration_id = integration_id.trim();
            if tool_name.is_empty() || integration_id.is_empty() {
                continue;
            }
            aliases.insert(tool_name.to_string(), integration_id.to_string());
        }
    }

    async fn load_tool_integration_aliases(&self) -> HashMap<String, String> {
        let mut aliases = Self::default_tool_integration_aliases();

        if let Ok(raw_env) = std::env::var("AGENTARK_TOOL_INTEGRATION_ALIASES") {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw_env) {
                Self::merge_tool_integration_aliases(&mut aliases, &value);
            } else {
                tracing::warn!("Invalid AGENTARK_TOOL_INTEGRATION_ALIASES JSON ignored");
            }
        }

        if let Ok(Some(raw)) = self.storage.get(TOOL_INTEGRATION_ALIASES_KEY).await {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&raw) {
                Self::merge_tool_integration_aliases(&mut aliases, &value);
            } else {
                tracing::warn!("Invalid '{}' JSON ignored", TOOL_INTEGRATION_ALIASES_KEY);
            }
        }

        let enabled_ids: HashSet<String> = self.integrations.enabled_ids().into_iter().collect();
        for integration_id in &enabled_ids {
            aliases
                .entry(integration_id.clone())
                .or_insert_with(|| integration_id.clone());
        }
        aliases.retain(|_, integration_id| enabled_ids.contains(integration_id));

        aliases
    }

    async fn load_persisted_tool_integration_aliases(&self) -> HashMap<String, String> {
        let Ok(Some(raw)) = self.storage.get(TOOL_INTEGRATION_ALIASES_KEY).await else {
            return HashMap::new();
        };
        serde_json::from_slice::<HashMap<String, String>>(&raw).unwrap_or_default()
    }

    pub(crate) async fn register_tool_integration_alias(
        &self,
        tool_name: &str,
        integration_id: &str,
    ) -> Result<()> {
        let tool_name = tool_name.trim();
        let integration_id = integration_id.trim();
        if tool_name.is_empty() || integration_id.is_empty() {
            return Err(anyhow::anyhow!(
                "tool_name and integration_id must be non-empty"
            ));
        }
        let mut persisted = self.load_persisted_tool_integration_aliases().await;
        persisted.insert(tool_name.to_string(), integration_id.to_string());
        let raw = serde_json::to_vec(&persisted)?;
        self.storage.set(TOOL_INTEGRATION_ALIASES_KEY, &raw).await?;
        Ok(())
    }

    pub(crate) fn resolve_tool_integration_id(
        &self,
        tool_name: &str,
        aliases: &HashMap<String, String>,
    ) -> Option<String> {
        aliases.get(tool_name).cloned()
    }

    pub(crate) async fn execute_integration_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        trace_ref: &Arc<RwLock<ExecutionTrace>>,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
        request_channel: &str,
        integration_id: &str,
    ) -> String {
        let sub_action = call
            .arguments
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("default");
        let resolved_args = self
            .runtime
            .resolve_secret_placeholders(&call.name, &call.arguments)
            .unwrap_or_else(|_| call.arguments.clone());
        let hook_event_id = uuid::Uuid::new_v4().to_string();
        let hook_hint = action_message_hint(&resolved_args);
        self.fire_action_hook(
            crate::hooks::HookTrigger::PreAction,
            request_channel,
            &call.name,
            hook_hint.as_deref(),
            None,
            &hook_event_id,
        )
        .await;

        match self
            .integrations
            .execute(integration_id, sub_action, &resolved_args)
            .await
        {
            Ok(result) => {
                if integration_id == "moltbook" {
                    self.append_moltbook_tool_activity(
                        sub_action,
                        &resolved_args,
                        Some(&result),
                        None,
                    )
                    .await;
                    let (title, detail, step_type, data) = build_moltbook_trace_result_step(
                        sub_action,
                        &resolved_args,
                        Some(&result),
                        None,
                    );
                    trace_ref.write().await.steps.push(ExecutionStep {
                        icon: "[ok]".to_string(),
                        title,
                        detail,
                        step_type,
                        data,
                        timestamp: chrono::Utc::now(),
                        duration_ms: None,
                    });
                }
                let formatted =
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string());
                self.fire_action_hook(
                    crate::hooks::HookTrigger::PostAction,
                    request_channel,
                    &call.name,
                    hook_hint.as_deref(),
                    Some(&formatted),
                    &hook_event_id,
                )
                .await;
                if let Some(tx) = stream_tx {
                    let _ = tx.try_send(StreamEvent::ToolResult {
                        name: call.name.clone(),
                        content: self.sanitize_stream_preview(&formatted),
                    });
                }
                formatted
            }
            Err(e) => {
                if integration_id == "moltbook" {
                    self.append_moltbook_tool_activity(
                        sub_action,
                        &resolved_args,
                        None,
                        Some(&e.to_string()),
                    )
                    .await;
                    let error_text = e.to_string();
                    let (title, detail, step_type, data) = build_moltbook_trace_result_step(
                        sub_action,
                        &resolved_args,
                        None,
                        Some(&error_text),
                    );
                    trace_ref.write().await.steps.push(ExecutionStep {
                        icon: "[warn]".to_string(),
                        title,
                        detail,
                        step_type,
                        data,
                        timestamp: chrono::Utc::now(),
                        duration_ms: None,
                    });
                }
                tracing::error!("{} integration error: {}", call.name, e);
                self.fire_action_hook(
                    crate::hooks::HookTrigger::OnError,
                    request_channel,
                    &call.name,
                    hook_hint.as_deref(),
                    Some(&e.to_string()),
                    &hook_event_id,
                )
                .await;
                let formatted = format!("Error from {}: {}", call.name, e);
                if let Some(tx) = stream_tx {
                    let _ = tx.try_send(StreamEvent::ToolResult {
                        name: call.name.clone(),
                        content: formatted.clone(),
                    });
                }
                formatted
            }
        }
    }

    fn integration_capability_labels(caps: Vec<crate::integrations::Capability>) -> Vec<String> {
        caps.into_iter()
            .map(|cap| match cap {
                crate::integrations::Capability::Read => "read".to_string(),
                crate::integrations::Capability::Write => "write".to_string(),
                crate::integrations::Capability::Subscribe => "subscribe".to_string(),
                crate::integrations::Capability::Search => "search".to_string(),
                crate::integrations::Capability::Delete => "delete".to_string(),
                crate::integrations::Capability::Notify => "notify".to_string(),
            })
            .collect()
    }

    fn build_integration_action_def(
        &self,
        tool_name: &str,
        integration_id: &str,
        integration: &dyn crate::integrations::Integration,
    ) -> crate::actions::ActionDef {
        crate::actions::ActionDef {
            name: tool_name.to_string(),
            description: format!(
                "Integration tool '{}' routed to '{}'. {} Pass an 'action' field and any connector-specific parameters.",
                tool_name,
                integration_id,
                integration.description()
            ),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "description": "Connector operation to execute"
                    }
                },
                "additionalProperties": true
            }),
            capabilities: Self::integration_capability_labels(integration.capabilities()),
            sandbox_mode: None,
            source: crate::actions::ActionSource::System,
            file_path: None,
        }
    }

    pub(crate) async fn append_dynamic_integration_actions(
        &self,
        actions: &mut Vec<crate::actions::ActionDef>,
    ) {
        let mut existing: HashSet<String> = actions.iter().map(|a| a.name.clone()).collect();
        let integration_aliases = self.load_tool_integration_aliases().await;
        let enabled_ids: HashSet<String> = integration_aliases.values().cloned().collect();

        for integration_id in &enabled_ids {
            let Some(integration) = self.integrations.get(integration_id) else {
                continue;
            };
            if existing.insert(integration_id.to_string()) {
                actions.push(self.build_integration_action_def(
                    integration_id,
                    integration_id,
                    integration,
                ));
            }
        }

        for (tool_name, integration_id) in integration_aliases {
            if !enabled_ids.contains(&integration_id) {
                continue;
            }
            if !existing.insert(tool_name.clone()) {
                continue;
            }
            let Some(integration) = self.integrations.get(&integration_id) else {
                continue;
            };
            actions.push(self.build_integration_action_def(
                &tool_name,
                &integration_id,
                integration,
            ));
        }
    }

    pub(crate) async fn execute_single_tool_call_legacy(
        &self,
        call: &crate::core::llm::ToolCall,
        trace_ref: &Arc<RwLock<ExecutionTrace>>,
        stream_tx: Option<tokio::sync::mpsc::Sender<StreamEvent>>,
        request_channel: &str,
    ) -> Result<String> {
        let call_started = std::time::Instant::now();
        let synthetic = crate::core::llm::LlmResponse {
            content: String::new(),
            tool_calls: vec![call.clone()],
            reasoning: None,
            usage: None,
            provider: "internal".to_string(),
            model: "tool_dispatch".to_string(),
        };
        match self
            .execute_tool_calls_legacy(&synthetic, trace_ref, stream_tx, request_channel)
            .await
        {
            Ok(output) => {
                self.record_self_tune_tool_output(
                    &call.name,
                    &output,
                    call_started.elapsed().as_millis() as u64,
                )
                .await;
                Ok(output)
            }
            Err(error) => {
                self.record_self_tune_tool_outcome(
                    &call.name,
                    false,
                    call_started.elapsed().as_millis() as u64,
                )
                .await;
                Err(error)
            }
        }
    }

    pub(crate) async fn handle_generate_image_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
        request_channel: &str,
    ) -> Result<String> {
        self.execute_single_tool_call_legacy(
            call,
            &Arc::new(RwLock::new(ExecutionTrace::default())),
            stream_tx.cloned(),
            request_channel,
        )
        .await
    }

    pub(crate) async fn handle_generate_video_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
        request_channel: &str,
        _public_base_url: Option<&str>,
    ) -> Result<String> {
        self.execute_single_tool_call_legacy(
            call,
            &Arc::new(RwLock::new(ExecutionTrace::default())),
            stream_tx.cloned(),
            request_channel,
        )
        .await
    }

    pub(crate) async fn handle_browser_auto_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Result<String> {
        self.execute_single_tool_call_legacy(
            call,
            &Arc::new(RwLock::new(ExecutionTrace::default())),
            stream_tx.cloned(),
            "web",
        )
        .await
    }

    pub(crate) async fn restart_deployed_app_from_metadata(
        &self,
        app_id: &str,
    ) -> Result<serde_json::Value> {
        let app_id = app_id.trim();
        if app_id.is_empty() {
            anyhow::bail!("Missing app_id");
        }

        let app_dir = if let Some(path) = self.app_registry.get_dir(app_id).await {
            path
        } else {
            let fallback = self.data_dir.join("apps").join(app_id);
            if !fallback.exists() {
                anyhow::bail!("App '{}' not found", app_id);
            }
            fallback
        };

        let meta_path = app_dir.join(".app_meta.json");
        let mut meta: serde_json::Value = match tokio::fs::read(&meta_path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|_| serde_json::json!({})),
            Err(_) => serde_json::json!({}),
        };
        if !meta.is_object() {
            meta = serde_json::json!({});
        }

        let title = meta
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or(app_id)
            .to_string();
        let entry_command = meta
            .get("entry_command")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let install_command = meta
            .get("install_command")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let runtime_image = meta
            .get("runtime_image")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let runtime_preference = crate::actions::app::runtime_preference_from_opt(
            meta.get("runtime_preference").and_then(|v| v.as_str()),
        );
        let required_inputs = crate::actions::app::parse_required_inputs(&meta);
        let config_values: std::collections::HashMap<String, String> = meta
            .get("config_values")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| {
                        let value = match v {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Bool(b) => b.to_string(),
                            serde_json::Value::Number(n) => n.to_string(),
                            _ => return None,
                        };
                        Some((k.clone(), value))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let access_guard_enabled = meta
            .get("access_guard_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let access_key = if access_guard_enabled {
            meta.get("access_key")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(crate::actions::app::generate_access_key)
        } else {
            String::new()
        };

        if meta.get("access_guard_enabled").is_none()
            || (access_guard_enabled && meta.get("access_key").is_none())
        {
            meta["access_guard_enabled"] = serde_json::Value::Bool(access_guard_enabled);
            meta["access_key"] = serde_json::Value::String(access_key.clone());
            let _ = tokio::fs::write(
                &meta_path,
                serde_json::to_vec_pretty(&meta).unwrap_or_default(),
            )
            .await;
        }

        self.app_registry.stop_runtime(app_id).await?;

        let relative_url = format!("/apps/{}/", app_id);
        let local_base = Self::user_facing_local_base_url();
        let local_url = Self::absolutize_public_url(Some(local_base.as_str()), &relative_url);
        let relative_access_url = if access_guard_enabled {
            format!("/apps/{}/?key={}", app_id, access_key)
        } else {
            relative_url.clone()
        };
        let local_access_url =
            Self::absolutize_public_url(Some(local_base.as_str()), &relative_access_url);

        if let Some(entry_command) = entry_command {
            let Some(port) = self.app_registry.find_available_port().await else {
                anyhow::bail!("No available app port");
            };
            let llm_env = self.app_model_env_vars();

            let (resolved_env, missing_sensitive, missing_config) =
                crate::actions::app::resolve_required_env_values(
                    &self.config_dir,
                    &self.data_dir,
                    &required_inputs,
                    &llm_env,
                    &config_values,
                )
                .await?;

            if !missing_sensitive.is_empty() || !missing_config.is_empty() {
                let mut missing_all = missing_sensitive.clone();
                for item in &missing_config {
                    if !missing_all.iter().any(|existing| existing == item) {
                        missing_all.push(item.clone());
                    }
                }
                let required_secret_keys: Vec<String> = required_inputs
                    .iter()
                    .filter(|required| required.sensitive)
                    .map(|required| required.key.clone())
                    .collect();
                let required_config_keys: Vec<String> = required_inputs
                    .iter()
                    .filter(|required| !required.sensitive)
                    .map(|required| required.key.clone())
                    .collect();
                return Ok(serde_json::json!({
                    "status": "needs_secrets",
                    "app_id": app_id,
                    "title": title,
                    "url": relative_url,
                    "local_url": local_url,
                    "missing_env": missing_sensitive,
                    "missing_config": missing_config,
                    "missing_inputs": missing_all,
                    "required_inputs": required_inputs,
                    "required_secrets": required_secret_keys.clone(),
                    "required_env": required_secret_keys,
                    "required_config": required_config_keys,
                    "message": "Missing required inputs. Use set secret KEY=VALUE for sensitive values; provide config for non-sensitive values."
                }));
            }

            let runtime_handle = crate::actions::app::launch_dynamic_runtime(
                crate::actions::app::DynamicRuntimeLaunch {
                    app_id,
                    app_dir: &app_dir,
                    entry_command: &entry_command,
                    install_command: install_command.as_deref(),
                    port,
                    extra_env: &resolved_env,
                    runtime_image: runtime_image.as_deref(),
                    runtime_preference,
                    stream_tx: None,
                },
            )
            .await?;

            let (child, container_id, runtime_label) = match runtime_handle {
                crate::actions::app::DynamicRuntimeHandle::Container(container_id) => {
                    (None, Some(container_id), "container")
                }
                crate::actions::app::DynamicRuntimeHandle::Process(child) => {
                    (Some(*child), None, "local_process")
                }
            };
            let diagnostics_dir = app_dir.clone();

            self.app_registry
                .register_dynamic(
                    app_id.to_string(),
                    crate::actions::app::DynamicAppRegistration {
                        title: title.clone(),
                        app_dir,
                        child,
                        container_id,
                        port,
                        access_key: access_key.clone(),
                        access_guard_enabled,
                    },
                )
                .await;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if !self.app_registry.runtime_is_alive(app_id).await {
                let logs =
                    crate::actions::app::read_local_runtime_log_tail(&diagnostics_dir, 4096).await;
                if logs.is_empty() {
                    anyhow::bail!("App process stopped shortly after restart.");
                }
                anyhow::bail!(
                    "App process stopped shortly after restart. Recent runtime logs:\n{}",
                    logs
                );
            }

            return Ok(serde_json::json!({
                "status": "restarted",
                "type": "dynamic",
                "runtime": runtime_label,
                "app_id": app_id,
                "title": title,
                "url": relative_url,
                "local_url": local_url,
                "access_url": relative_access_url,
                "local_access_url": local_access_url,
                "access_guard_enabled": access_guard_enabled,
                "port": port,
                "runtime_preference": runtime_preference.as_str(),
            }));
        }

        self.app_registry
            .register_static(
                app_id.to_string(),
                title.clone(),
                app_dir,
                access_key.clone(),
                access_guard_enabled,
            )
            .await;
        Ok(serde_json::json!({
            "status": "restarted",
            "type": "static",
            "app_id": app_id,
            "title": title,
            "url": relative_url,
            "local_url": local_url,
            "access_url": relative_access_url,
            "local_access_url": local_access_url,
            "access_guard_enabled": access_guard_enabled,
            "runtime_preference": runtime_preference.as_str(),
        }))
    }

    pub(crate) async fn handle_app_restart_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
        _request_channel: &str,
        conversation_id: Option<&str>,
    ) -> Result<String> {
        let explicit_app_id = call
            .arguments
            .get("app_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let query = call
            .arguments
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        let mut resolved_app_id = explicit_app_id.clone();
        if resolved_app_id.is_empty() {
            let apps = self.app_registry.list().await;
            if apps.is_empty() {
                let out = serde_json::json!({
                    "app_id": serde_json::Value::Null,
                    "query": if query.is_empty() { serde_json::Value::Null } else { serde_json::json!(query) },
                    "status": "not_found",
                    "message": "No deployed apps are currently registered."
                });
                let formatted = serde_json::to_string_pretty(&out)?;
                if let Some(tx) = stream_tx {
                    let _ = tx.try_send(StreamEvent::ToolResult {
                        name: call.name.clone(),
                        content: formatted.clone(),
                    });
                }
                return Ok(formatted);
            }

            let ranked_apps = Self::rank_deployed_apps(&query, &apps);
            let best_match = Self::select_best_ranked_app(&query, &ranked_apps);
            if let Some((_, app_id, _, _)) = best_match {
                resolved_app_id = app_id.clone();
            } else {
                let app_summaries = ranked_apps
                    .iter()
                    .take(10)
                    .map(|(score, _, app, reason)| {
                        let relative_url = app.get("url").and_then(|v| v.as_str()).unwrap_or("/apps/");
                        let local_url = Self::absolutize_public_url(
                            Some(Self::user_facing_local_base_url().as_str()),
                            relative_url,
                        );
                        serde_json::json!({
                            "id": app.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                            "title": app.get("title").and_then(|v| v.as_str()).unwrap_or("App"),
                            "running": app.get("running").and_then(|v| v.as_bool()).unwrap_or(false),
                            "local_url": local_url,
                            "match_score": score,
                            "match_reason": reason,
                        })
                    })
                    .collect::<Vec<_>>();
                let out = serde_json::json!({
                    "app_id": serde_json::Value::Null,
                    "query": if query.is_empty() { serde_json::Value::Null } else { serde_json::json!(query) },
                    "status": "not_found",
                    "apps": app_summaries,
                    "message": if query.is_empty() {
                        "app_restart needs an app_id or query to identify which deployed app to restart."
                    } else {
                        "No single deployed app matched the restart request."
                    }
                });
                let formatted = serde_json::to_string_pretty(&out)?;
                if let Some(tx) = stream_tx {
                    let _ = tx.try_send(StreamEvent::ToolResult {
                        name: call.name.clone(),
                        content: formatted.clone(),
                    });
                }
                return Ok(formatted);
            }
        }

        let out = self
            .restart_deployed_app_from_metadata(&resolved_app_id)
            .await?;
        if out
            .get("status")
            .and_then(|v| v.as_str())
            .is_some_and(|status| status == "restarted")
        {
            self.trigger_arkpulse_refresh("app_restart");
            if let Some(cid) = conversation_id {
                let title = out.get("title").and_then(|v| v.as_str()).unwrap_or("App");
                let canonical_url = format!("/apps/{}/", resolved_app_id);
                self.persist_last_deployed_app_context(
                    cid,
                    &resolved_app_id,
                    title,
                    &canonical_url,
                )
                .await;
            }
        }

        let formatted = serde_json::to_string_pretty(&out)?;
        if let Some(tx) = stream_tx {
            let _ = tx.try_send(StreamEvent::ToolResult {
                name: call.name.clone(),
                content: formatted.clone(),
            });
        }
        Ok(formatted)
    }

    pub(crate) async fn handle_app_inspect_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
        _request_channel: &str,
        conversation_id: Option<&str>,
    ) -> Result<String> {
        let query = call
            .arguments
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let include_files = call
            .arguments
            .get("include_files")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let include_logs = call
            .arguments
            .get("include_logs")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let limit = call
            .arguments
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .clamp(1, 25) as usize;

        let apps = self.app_registry.list().await;
        if apps.is_empty() {
            let out = serde_json::json!({
                "query": if query.is_empty() { serde_json::Value::Null } else { serde_json::json!(query) },
                "total_apps": 0,
                "matched_app": serde_json::Value::Null,
                "apps": Vec::<serde_json::Value>::new(),
                "message": "No deployed apps are currently registered."
            });
            let formatted = serde_json::to_string_pretty(&out)?;
            if let Some(tx) = stream_tx {
                let _ = tx.try_send(StreamEvent::ToolResult {
                    name: call.name.clone(),
                    content: formatted.clone(),
                });
            }
            return Ok(formatted);
        }

        let ranked_apps = Self::rank_deployed_apps(&query, &apps);
        let best_match = Self::select_best_ranked_app(&query, &ranked_apps);

        let mut app_summaries = Vec::new();
        for (score, _app_id, app, reason) in ranked_apps.iter().take(limit) {
            let title = app.get("title").and_then(|v| v.as_str()).unwrap_or("App");
            let relative_url = app.get("url").and_then(|v| v.as_str()).unwrap_or("/apps/");
            let local_url = Self::absolutize_public_url(
                Some(Self::user_facing_local_base_url().as_str()),
                relative_url,
            );
            let mut summary = serde_json::json!({
                "id": app.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                "title": title,
                "running": app.get("running").and_then(|v| v.as_bool()).unwrap_or(false),
                "is_static": app.get("is_static").and_then(|v| v.as_bool()).unwrap_or(true),
                "runtime_mode": app.get("runtime_mode").and_then(|v| v.as_str()).unwrap_or("unknown"),
                "local_url": local_url,
            });
            if !query.is_empty() {
                if let Some(obj) = summary.as_object_mut() {
                    obj.insert("match_score".to_string(), serde_json::json!(score));
                    obj.insert("match_reason".to_string(), serde_json::json!(reason));
                }
            }
            app_summaries.push(summary);
        }

        let matched_app = if let Some((_, app_id, app, _)) = best_match {
            let inspection = self
                .build_deployed_app_inspection(app, include_files, include_logs)
                .await;
            if let (Some(cid), Some(details)) = (conversation_id, inspection.as_ref()) {
                let title = details
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("App");
                let canonical_url = format!("/apps/{}/", app_id);
                self.persist_last_deployed_app_context(cid, app_id, title, &canonical_url)
                    .await;
            }
            inspection.unwrap_or_else(|| {
                serde_json::json!({
                    "id": app_id,
                    "title": app.get("title").and_then(|v| v.as_str()).unwrap_or("App"),
                    "message": "Matched app but failed to read detailed app metadata."
                })
            })
        } else {
            serde_json::Value::Null
        };

        let message = if matched_app.is_null() {
            if query.is_empty() {
                "Listed deployed apps. Use a title or app ID in query to inspect one in detail."
                    .to_string()
            } else {
                format!(
                    "No single deployed app matched '{}'. Review the listed apps or refine the query.",
                    query
                )
            }
        } else {
            let title = matched_app
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("App");
            let app_dir = matched_app
                .get("app_dir")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!(
                "Matched deployed app '{}'. Use file_read/file_write on {} to inspect or repair it, then app_restart to apply changes.",
                title, app_dir
            )
        };

        let out = serde_json::json!({
            "query": if query.is_empty() { serde_json::Value::Null } else { serde_json::json!(query) },
            "total_apps": apps.len(),
            "matched_app": matched_app,
            "apps": app_summaries,
            "message": message,
        });
        let formatted = serde_json::to_string_pretty(&out)?;
        if let Some(tx) = stream_tx {
            let _ = tx.try_send(StreamEvent::ToolResult {
                name: call.name.clone(),
                content: formatted.clone(),
            });
        }
        Ok(formatted)
    }

    pub(crate) async fn handle_app_deploy_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
        request_channel: &str,
        conversation_id: Option<&str>,
        _public_base_url: Option<&str>,
    ) -> Result<String> {
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<StreamEvent>(256);
        let upstream_tx = stream_tx.cloned();
        let request_channel_owned = request_channel.to_string();
        let conversation_id_owned = conversation_id.map(str::to_string);
        let telegram_config = self.config.telegram.clone();
        let whatsapp_config = self.config.whatsapp.clone();
        let agent_name = self.config.name.clone();
        let relay_task = tokio::spawn(async move {
            let mut relay_state = AppDeployProgressRelayState::default();
            while let Some(ev) = progress_rx.recv().await {
                let chat_message = app_deploy_chat_progress_message(&ev, &mut relay_state);
                if let Some(tx) = upstream_tx.as_ref() {
                    let forwarded = match ev {
                        StreamEvent::ToolProgress {
                            name,
                            content,
                            payload,
                        } => StreamEvent::ToolProgress {
                            name,
                            content,
                            payload: if let Some(msg) = chat_message.as_ref() {
                                merge_chat_visible_progress_payload(payload, msg)
                            } else {
                                payload
                            },
                        },
                        other => other,
                    };
                    let _ = tx.send(forwarded).await;
                }
                if let Some(msg) = chat_message.as_ref() {
                    let request_channel = request_channel_owned.clone();
                    let conversation_id = conversation_id_owned.clone();
                    let telegram_config = telegram_config.clone();
                    let whatsapp_config = whatsapp_config.clone();
                    let agent_name = agent_name.clone();
                    let message = msg.clone();
                    tokio::spawn(async move {
                        send_app_deploy_progress_to_conversation(
                            &request_channel,
                            conversation_id.as_deref(),
                            telegram_config.as_ref(),
                            whatsapp_config.as_ref(),
                            &agent_name,
                            &message,
                        )
                        .await;
                    });
                }
            }
        });

        let result = self
            .execute_single_tool_call_legacy(
                call,
                &Arc::new(RwLock::new(ExecutionTrace::default())),
                Some(progress_tx.clone()),
                request_channel,
            )
            .await;
        drop(progress_tx);
        let _ = relay_task.await;
        result
    }

    pub(crate) async fn handle_memory_lookup_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
        request_channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<String> {
        if let Some(tx) = stream_tx {
            let _ = tx.try_send(StreamEvent::ToolStart {
                name: call.name.clone(),
                payload: None,
            });
        }

        let query = call
            .arguments
            .get("query")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("memory_lookup requires a non-empty 'query'"))?;
        let limit = call
            .arguments
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(1, 10) as usize)
            .unwrap_or(5);
        let include_semantic = call
            .arguments
            .get("include_semantic")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let include_structured = call
            .arguments
            .get("include_structured")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let mut sections: Vec<String> = Vec::new();

        if include_semantic {
            let mem0_scope =
                self.mem0_scope_for_request(request_channel, conversation_id, project_id);
            let semantic_lines = if self.mem0.is_available() {
                match self.mem0.search(query, &mem0_scope, limit).await {
                    Ok(memories) => memories
                        .into_iter()
                        .take(limit)
                        .map(|m| format!("- {}", safe_truncate(&m.memory, 220)))
                        .collect::<Vec<_>>(),
                    Err(e) => {
                        tracing::warn!("memory_lookup mem0 search failed: {}", e);
                        Vec::new()
                    }
                }
            } else {
                match self
                    .memory
                    .retrieve_relevant(query, limit.min(5), project_id)
                    .await
                {
                    Ok(memories) => memories
                        .into_iter()
                        .take(limit)
                        .map(|m| format!("- {}", safe_truncate(&m.content, 220)))
                        .collect::<Vec<_>>(),
                    Err(e) => {
                        tracing::warn!("memory_lookup built-in retrieval failed: {}", e);
                        Vec::new()
                    }
                }
            };

            if !semantic_lines.is_empty() {
                sections.push(format!("## Relevant Memory\n{}", semantic_lines.join("\n")));
            }
        }

        if include_structured {
            if let Some(domain_ctx) = self.build_memory_domain_context(query, project_id).await {
                sections.push(domain_ctx);
            }
        }

        let output = if sections.is_empty() {
            format!(
                "No relevant memory was found for `{}`.",
                safe_truncate(query, 120)
            )
        } else {
            format!(
                "Memory lookup for `{}`.\n\n{}",
                safe_truncate(query, 120),
                sections.join("\n\n")
            )
        };

        if let Some(tx) = stream_tx {
            let _ = tx.try_send(StreamEvent::ToolResult {
                name: call.name.clone(),
                content: output.clone(),
            });
        }

        Ok(output)
    }

    pub(crate) async fn handle_goal_manage_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Result<String> {
        if let Some(tx) = stream_tx {
            let _ = tx.try_send(StreamEvent::ToolStart {
                name: call.name.clone(),
                payload: None,
            });
        }

        let operation = call
            .arguments
            .get("operation")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("goal_manage requires an 'operation'"))?;

        let result = match operation {
            "list" => {
                let limit = call
                    .arguments
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|v| v.clamp(1, 50) as usize)
                    .unwrap_or(10);

                let mut goals = {
                    let tasks = self.tasks.read().await;
                    tasks
                        .all()
                        .iter()
                        .filter(|task| task.action == "goal")
                        .cloned()
                        .collect::<Vec<_>>()
                };
                goals.sort_by(|a, b| {
                    b.created_at
                        .cmp(&a.created_at)
                        .then_with(|| a.description.cmp(&b.description))
                });

                if goals.is_empty() {
                    "No goals are currently saved.".to_string()
                } else {
                    let mut lines = Vec::new();
                    for goal in goals.into_iter().take(limit) {
                        let goal_text = goal
                            .arguments
                            .get("goal")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or(goal.description.as_str());
                        let goal_id = goal
                            .arguments
                            .get("goal_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        let status = match &goal.status {
                            crate::core::TaskStatus::Pending => "pending",
                            crate::core::TaskStatus::AwaitingApproval => "awaiting approval",
                            crate::core::TaskStatus::Paused => "paused",
                            crate::core::TaskStatus::InProgress => "in progress",
                            crate::core::TaskStatus::Completed => "saved",
                            crate::core::TaskStatus::Failed { .. } => "failed",
                            crate::core::TaskStatus::Cancelled => "cancelled",
                        };
                        let mut line = format!("- {} [{}]", safe_truncate(goal_text, 160), status);
                        if let Some(due) = goal.scheduled_for {
                            line.push_str(&format!(" due {}", due.format("%Y-%m-%d")));
                        }
                        if !goal_id.is_empty() {
                            line.push_str(&format!(" | id `{}`", goal_id));
                        }
                        lines.push(line);
                    }
                    format!("Saved goals ({}):\n{}", lines.len(), lines.join("\n"))
                }
            }
            "create" => {
                let goal = call
                    .arguments
                    .get("goal")
                    .or_else(|| call.arguments.get("description"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        anyhow::anyhow!("goal_manage create requires a non-empty 'goal'")
                    })?;

                let due_date = if let Some(raw) = call
                    .arguments
                    .get("due_date")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
                        Some(dt.with_timezone(&chrono::Utc))
                    } else if let Ok(date) = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
                        Some(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                            date.and_hms_opt(23, 59, 59).unwrap(),
                            chrono::Utc,
                        ))
                    } else {
                        anyhow::bail!("Invalid due_date. Use YYYY-MM-DD or RFC3339");
                    }
                } else {
                    None
                };
                let allow_duplicate = call
                    .arguments
                    .get("allow_duplicate")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let goal_id = uuid::Uuid::new_v4().to_string();
                let mut task = crate::core::Task::new(
                    format!("Goal: {}", goal),
                    "goal".to_string(),
                    serde_json::json!({
                        "goal_id": goal_id.clone(),
                        "goal": goal,
                    }),
                );
                task.scheduled_for = due_date;
                task.status = crate::core::TaskStatus::Completed;
                task.result = Some("Goal registered.".to_string());
                if !allow_duplicate {
                    let existing_goal_id = {
                        let tasks = self.tasks.read().await;
                        tasks
                            .all()
                            .iter()
                            .filter(|existing| existing.action == "goal")
                            .find(|existing| {
                                crate::core::task::tasks_are_semantically_similar(existing, &task)
                            })
                            .and_then(|existing| {
                                existing
                                    .arguments
                                    .get("goal_id")
                                    .and_then(|value| value.as_str())
                                    .map(ToString::to_string)
                            })
                    };
                    if let Some(existing_goal_id) = existing_goal_id {
                        if let Some(args) = task.arguments.as_object_mut() {
                            args.insert(
                                "goal_id".to_string(),
                                serde_json::Value::String(existing_goal_id),
                            );
                        }
                    }
                }
                let (_, reused_existing, _) = self
                    .add_or_update_similar_task(task, allow_duplicate)
                    .await?;

                if !reused_existing {
                    if let Some(due) = due_date {
                        let now = chrono::Utc::now();
                        let days_until = (due - now).num_days();
                        let mut reminders = Vec::new();
                        if days_until > 1 {
                            let mut reminder = crate::core::Task::new(
                                format!("Reminder: \"{}\" is due tomorrow", goal),
                                "goal_reminder".to_string(),
                                serde_json::json!({
                                    "goal_id": goal_id.clone(),
                                    "goal": goal,
                                    "days_left": 1
                                }),
                            );
                            reminder.scheduled_for = Some(due - chrono::Duration::days(1));
                            reminders.push(reminder);
                        }
                        if days_until > 3 {
                            let mut reminder = crate::core::Task::new(
                                format!("Reminder: \"{}\" is due in 3 days", goal),
                                "goal_reminder".to_string(),
                                serde_json::json!({
                                    "goal_id": goal_id.clone(),
                                    "goal": goal,
                                    "days_left": 3
                                }),
                            );
                            reminder.scheduled_for = Some(due - chrono::Duration::days(3));
                            reminders.push(reminder);
                        }
                        for reminder in reminders {
                            let _ = self.add_task(reminder).await;
                        }
                    }
                }

                let mut message = if reused_existing {
                    format!("Updated existing goal `{}`.", safe_truncate(goal, 160))
                } else {
                    format!("Saved goal `{}`.", safe_truncate(goal, 160))
                };
                if let Some(due) = due_date.as_ref() {
                    message.push_str(&format!(" Due {}.", due.format("%Y-%m-%d")));
                }
                message.push_str(&format!(" Goal ID: `{}`.", goal_id));
                message
            }
            "delete" => {
                let target_goal_id = call
                    .arguments
                    .get("goal_id")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                let target_goal_text = call
                    .arguments
                    .get("goal")
                    .or_else(|| call.arguments.get("description"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);

                if target_goal_id.is_none() && target_goal_text.is_none() {
                    anyhow::bail!("goal_manage delete requires 'goal_id' or 'goal'");
                }

                let snapshot = {
                    let tasks = self.tasks.read().await;
                    tasks.all().to_vec()
                };

                let matching_goal_tasks = snapshot
                    .iter()
                    .filter(|task| {
                        if task.action != "goal" {
                            return false;
                        }
                        let task_goal_id = task
                            .arguments
                            .get("goal_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        let task_goal_text = task
                            .arguments
                            .get("goal")
                            .and_then(|v| v.as_str())
                            .unwrap_or(task.description.as_str());
                        target_goal_id
                            .as_ref()
                            .map(|id| task_goal_id == id || task.id.to_string() == *id)
                            .unwrap_or(false)
                            || target_goal_text
                                .as_ref()
                                .map(|goal| task_goal_text.eq_ignore_ascii_case(goal))
                                .unwrap_or(false)
                    })
                    .cloned()
                    .collect::<Vec<_>>();

                if matching_goal_tasks.is_empty() {
                    "No matching goal was found.".to_string()
                } else {
                    let goal_ids = matching_goal_tasks
                        .iter()
                        .filter_map(|task| {
                            task.arguments
                                .get("goal_id")
                                .and_then(|v| v.as_str())
                                .map(str::to_string)
                        })
                        .collect::<std::collections::BTreeSet<_>>();
                    let goal_texts = matching_goal_tasks
                        .iter()
                        .filter_map(|task| {
                            task.arguments
                                .get("goal")
                                .and_then(|v| v.as_str())
                                .or(Some(task.description.as_str()))
                                .map(str::to_string)
                        })
                        .collect::<std::collections::BTreeSet<_>>();

                    let to_delete = snapshot
                        .iter()
                        .filter(|task| {
                            let task_goal_id = task
                                .arguments
                                .get("goal_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default();
                            let task_goal_text = task
                                .arguments
                                .get("goal")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default();
                            matching_goal_tasks
                                .iter()
                                .any(|goal_task| goal_task.id == task.id)
                                || (!goal_ids.is_empty() && goal_ids.contains(task_goal_id))
                                || (task.action == "goal_reminder"
                                    && !goal_texts.is_empty()
                                    && goal_texts
                                        .iter()
                                        .any(|goal| task_goal_text.eq_ignore_ascii_case(goal)))
                        })
                        .cloned()
                        .collect::<Vec<_>>();

                    for task in &to_delete {
                        let _ = self.storage.delete_task(&task.id.to_string()).await;
                    }
                    let mut deleted = 0usize;
                    {
                        let mut tasks = self.tasks.write().await;
                        for task in &to_delete {
                            if tasks.remove(task.id) {
                                deleted += 1;
                            }
                        }
                    }

                    let deleted_goals = matching_goal_tasks
                        .iter()
                        .map(|task| {
                            task.arguments
                                .get("goal")
                                .and_then(|v| v.as_str())
                                .unwrap_or(task.description.as_str())
                                .to_string()
                        })
                        .collect::<std::collections::BTreeSet<_>>();
                    format!(
                        "Deleted goal(s): {}. Removed {} related item(s).",
                        deleted_goals.into_iter().collect::<Vec<_>>().join(", "),
                        deleted
                    )
                }
            }
            "report" => {
                let goal_id = call
                    .arguments
                    .get("goal_id")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                self.build_goal_progress_report(goal_id).await?
            }
            other => anyhow::bail!(
                "Unknown goal_manage operation '{}'. Use create, list, delete, or report.",
                other
            ),
        };

        if let Some(tx) = stream_tx {
            let _ = tx.try_send(StreamEvent::ToolResult {
                name: call.name.clone(),
                content: result.clone(),
            });
        }

        Ok(result)
    }

    pub(crate) async fn handle_list_integrations_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Result<String> {
        if let Some(tx) = stream_tx {
            let _ = tx.try_send(StreamEvent::ToolStart {
                name: call.name.clone(),
                payload: None,
            });
        }

        let include_disabled = call
            .arguments
            .get("include_disabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let only_connected = call
            .arguments
            .get("only_connected")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let integration_aliases = self.load_tool_integration_aliases().await;
        let mut tools_by_integration: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for (tool_name, integration_id) in integration_aliases {
            tools_by_integration
                .entry(integration_id)
                .or_default()
                .push(tool_name);
        }

        let mut enabled_actions = self
            .runtime
            .list_enabled_actions()
            .await
            .unwrap_or_default();
        self.append_dynamic_integration_actions(&mut enabled_actions)
            .await;
        let builtin_integration_actions = enabled_actions
            .iter()
            .filter_map(|action| match action.name.as_str() {
                "gmail_scan" | "gmail_reply" | "calendar_today" | "calendar_list"
                | "calendar_create" | "calendar_free" => Some(action.name.clone()),
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>();

        let mut lines = Vec::new();
        let mut infos = self.integrations.list().await;
        infos.sort_by(|a, b| a.id.cmp(&b.id));
        for info in infos {
            let enabled = self.integrations.is_enabled(&info.id);
            if !include_disabled && !enabled {
                continue;
            }
            let status = match &info.status {
                crate::integrations::IntegrationStatus::NotConfigured => "not configured",
                crate::integrations::IntegrationStatus::NeedsAuth => "needs auth",
                crate::integrations::IntegrationStatus::Connected => "connected",
                crate::integrations::IntegrationStatus::Error(_) => "error",
            };
            if only_connected && status != "connected" {
                continue;
            }
            let capabilities = info
                .capabilities
                .iter()
                .map(|cap| match cap {
                    crate::integrations::Capability::Read => "read",
                    crate::integrations::Capability::Write => "write",
                    crate::integrations::Capability::Subscribe => "subscribe",
                    crate::integrations::Capability::Search => "search",
                    crate::integrations::Capability::Delete => "delete",
                    crate::integrations::Capability::Notify => "notify",
                })
                .collect::<Vec<_>>();
            let mut line = format!(
                "- {} (`{}`): {} | {}",
                info.name,
                info.id,
                if enabled { "enabled" } else { "disabled" },
                status
            );
            if !capabilities.is_empty() {
                line.push_str(&format!(" | capabilities: {}", capabilities.join(", ")));
            }
            if let Some(tools) = tools_by_integration.get(&info.id) {
                if !tools.is_empty() {
                    line.push_str(&format!(" | tools: {}", tools.join(", ")));
                }
            }
            lines.push(line);
        }

        if !builtin_integration_actions.is_empty() {
            lines.push(format!(
                "- Built-in integration-backed actions: {}",
                builtin_integration_actions
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        let result = if lines.is_empty() {
            "No integrations matched the requested filter.".to_string()
        } else {
            format!("Integration inventory:\n{}", lines.join("\n"))
        };

        if let Some(tx) = stream_tx {
            let _ = tx.try_send(StreamEvent::ToolResult {
                name: call.name.clone(),
                content: result.clone(),
            });
        }

        Ok(result)
    }

    pub(crate) async fn handle_list_watchers_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Result<String> {
        if let Some(tx) = stream_tx {
            let _ = tx.try_send(StreamEvent::ToolStart {
                name: call.name.clone(),
                payload: None,
            });
        }

        let filter = call
            .arguments
            .get("filter")
            .and_then(|v| v.as_str())
            .unwrap_or("active")
            .to_ascii_lowercase();
        let limit = call
            .arguments
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(20) as usize;

        let mut watchers = self.watcher_manager.list().await;
        watchers.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        let filtered: Vec<_> = watchers
            .into_iter()
            .filter(|watcher| {
                let status = match &watcher.status {
                    crate::core::watcher::WatcherStatus::Active => "active",
                    crate::core::watcher::WatcherStatus::Paused => "paused",
                    crate::core::watcher::WatcherStatus::Triggered => "triggered",
                    crate::core::watcher::WatcherStatus::TimedOut => "timed_out",
                    crate::core::watcher::WatcherStatus::Cancelled => "cancelled",
                    crate::core::watcher::WatcherStatus::Failed { .. } => "failed",
                };
                filter == "all" || filter == status
            })
            .take(limit)
            .collect::<Vec<_>>();

        let result = if filtered.is_empty() {
            format!("No {} watcher(s) found.", filter)
        } else {
            let mut lines = vec![format!("Found {} {} watcher(s):", filtered.len(), filter)];
            for watcher in filtered {
                let status = match &watcher.status {
                    crate::core::watcher::WatcherStatus::Active => "active".to_string(),
                    crate::core::watcher::WatcherStatus::Paused => "paused".to_string(),
                    crate::core::watcher::WatcherStatus::Triggered => "triggered".to_string(),
                    crate::core::watcher::WatcherStatus::TimedOut => "timed_out".to_string(),
                    crate::core::watcher::WatcherStatus::Cancelled => "cancelled".to_string(),
                    crate::core::watcher::WatcherStatus::Failed { error } => {
                        format!(
                            "failed ({})",
                            crate::core::automation::truncate_text(error, 80)
                        )
                    }
                };
                let next_poll = watcher
                    .last_poll_at
                    .map(|last| last + chrono::Duration::seconds(watcher.interval_secs as i64))
                    .unwrap_or(watcher.created_at);
                lines.push(format!(
                    "- {} [{}] poll=`{}` every {}s timeout {}s polls={} next_poll={}",
                    watcher.description,
                    status,
                    watcher.poll_action,
                    watcher.interval_secs,
                    watcher.timeout_secs,
                    watcher.poll_count,
                    next_poll.to_rfc3339()
                ));
            }
            lines.join("\n")
        };

        if let Some(tx) = stream_tx {
            let _ = tx.try_send(StreamEvent::ToolResult {
                name: call.name.clone(),
                content: result.clone(),
            });
        }

        Ok(result)
    }

    pub(crate) async fn handle_runtime_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        trace_ref: &Arc<RwLock<ExecutionTrace>>,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
        request_channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<String> {
        if call.name == "schedule_task" {
            let result = self
                .handle_schedule_task(
                    &call.arguments,
                    request_channel,
                    conversation_id,
                    project_id,
                )
                .await
                .unwrap_or_else(|| "Failed to schedule task.".to_string());
            if let Some(tx) = stream_tx {
                let _ = tx.try_send(StreamEvent::ToolResult {
                    name: call.name.clone(),
                    content: result.clone(),
                });
            }
            return Ok(result);
        }

        if call.name == "watch" {
            let result = self
                .handle_watch(
                    &call.arguments,
                    request_channel,
                    conversation_id,
                    project_id,
                )
                .await
                .unwrap_or_else(|| "Failed to create watcher.".to_string());
            if let Some(tx) = stream_tx {
                let _ = tx.try_send(StreamEvent::ToolResult {
                    name: call.name.clone(),
                    content: result.clone(),
                });
            }
            return Ok(result);
        }

        self.execute_single_tool_call_legacy(call, trace_ref, stream_tx.cloned(), request_channel)
            .await
    }

    /// Take a screenshot of a URL using the Playwright sidecar.
    pub(crate) async fn handle_screenshot_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
        request_channel: &str,
    ) -> Result<String> {
        let url = call
            .arguments
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if url.is_empty() {
            return Ok(
                serde_json::json!({"status": "error", "message": "Missing required 'url' parameter"})
                    .to_string(),
            );
        }

        let wait_ms = call
            .arguments
            .get("wait_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(1500);

        if let Some(tx) = stream_tx {
            let _ = tx.try_send(StreamEvent::ToolStart {
                name: "page_screenshot".to_string(),
                payload: None,
            });
        }

        if !self.browser_sessions.is_available().await {
            return Ok(
                serde_json::json!({"status": "error", "message": "Playwright sidecar unavailable"})
                    .to_string(),
            );
        }

        let integration = self.browser_sessions.integration().clone();
        let session = integration
            .create_session()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create browser session: {}", e))?;

        let result: Result<String> = async {
            let _ = integration.navigate(&session, &url).await?;
            tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;

            let screenshot = integration.screenshot(&session).await?;
            if screenshot.is_empty() {
                anyhow::bail!("Empty screenshot returned");
            }

            let screenshot_url = self
                .persist_output_binary("screenshot", "png", &screenshot)
                .await?;

            // Send to channel if not web
            if request_channel != "web" {
                let _ = crate::channels::send_screenshot(
                    self,
                    request_channel,
                    &screenshot,
                    &format!("Screenshot of {}", url),
                    Some(&screenshot_url),
                )
                .await;
            }

            Ok(serde_json::json!({
                "status": "ok",
                "url": screenshot_url,
                "size_bytes": screenshot.len()
            })
            .to_string())
        }
        .await;

        let _ = integration.close_session(&session).await;

        let output = match result {
            Ok(json) => json,
            Err(e) => serde_json::json!({"status": "error", "message": e.to_string()}).to_string(),
        };

        if let Some(tx) = stream_tx {
            let _ = tx.try_send(StreamEvent::ToolResult {
                name: "page_screenshot".to_string(),
                content: output.clone(),
            });
        }

        Ok(output)
    }

    /// Compose a structured report as HTML or Markdown.
    pub(crate) async fn handle_compose_report_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Result<String> {
        let title = call
            .arguments
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Report")
            .to_string();
        let sections = call
            .arguments
            .get("sections")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let format = call
            .arguments
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("html")
            .to_string();

        if let Some(tx) = stream_tx {
            let _ = tx.try_send(StreamEvent::ToolStart {
                name: "compose_report".to_string(),
                payload: None,
            });
        }

        let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC").to_string();

        let output = if format == "markdown" {
            let mut md = format!("# {}\n\n*Generated: {}*\n\n", title, timestamp);
            for section in &sections {
                let header = section
                    .get("header")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Section");
                let content = section
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                md.push_str(&format!("## {}\n\n{}\n\n", header, content));
            }

            let report_url = self
                .persist_output_binary("report", "md", md.as_bytes())
                .await?;

            serde_json::json!({
                "status": "ok",
                "path": report_url,
                "format": "markdown"
            })
            .to_string()
        } else {
            // HTML report with dark-themed inline CSS
            let mut body_html = String::new();
            for section in &sections {
                let header = section
                    .get("header")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Section");
                let content = section
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                // Convert newlines in content to <br> for display
                let content_html = content
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('\n', "<br>");
                body_html.push_str(&format!(
                    "<section><h2>{}</h2><div class=\"content\">{}</div></section>\n",
                    header
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;"),
                    content_html
                ));
            }

            let html = format!(
                r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
  * {{ margin: 0; padding: 0; box-sizing: border-box; }}
  body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
         background: #0a0e1a; color: #e0e6f0; padding: 2rem; line-height: 1.6; }}
  .report {{ max-width: 800px; margin: 0 auto; }}
  h1 {{ font-size: 1.8rem; margin-bottom: 0.25rem; color: #fff; }}
  .timestamp {{ font-size: 0.85rem; color: #6b7a99; margin-bottom: 2rem; }}
  section {{ background: rgba(255,255,255,0.04); border: 1px solid rgba(255,255,255,0.08);
            border-radius: 12px; padding: 1.25rem 1.5rem; margin-bottom: 1rem; }}
  h2 {{ font-size: 1.15rem; color: #2fd4ff; margin-bottom: 0.75rem; }}
  .content {{ color: #c0c8d8; }}
</style>
</head>
<body>
<div class="report">
  <h1>{title}</h1>
  <div class="timestamp">{timestamp}</div>
  {body_html}
</div>
</body>
</html>"#,
                title = title
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;"),
                timestamp = timestamp,
                body_html = body_html,
            );

            let report_url = self
                .persist_output_binary("report", "html", html.as_bytes())
                .await?;

            serde_json::json!({
                "status": "ok",
                "path": report_url,
                "format": "html"
            })
            .to_string()
        };

        if let Some(tx) = stream_tx {
            let _ = tx.try_send(StreamEvent::ToolResult {
                name: "compose_report".to_string(),
                content: output.clone(),
            });
        }

        Ok(output)
    }

    /// Handle self-evolve tool call with policy-first evolution defaults.
    pub(crate) async fn handle_self_evolve_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        trace_ref: &Arc<RwLock<ExecutionTrace>>,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Result<String> {
        let request = call
            .arguments
            .get("request")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if request.is_empty() {
            return Ok(serde_json::json!({
                "status": "error",
                "message": "Missing 'request' parameter - describe what should evolve"
            })
            .to_string());
        }

        let mode = call
            .arguments
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("policy")
            .trim()
            .to_ascii_lowercase();
        let allow_code_writes = call
            .arguments
            .get("allow_code_writes")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let apply_promotion = call
            .arguments
            .get("apply_promotion")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let canary_rollout_percent = call
            .arguments
            .get("canary_rollout_percent")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(1, 100) as u8)
            .unwrap_or(20);
        let canary_min_samples_per_version = call
            .arguments
            .get("canary_min_samples_per_version")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(5, 20_000) as usize)
            .unwrap_or(25);
        let canary_min_success_gain = call
            .arguments
            .get("canary_min_success_gain")
            .and_then(|v| v.as_f64())
            .map(|v| v.clamp(0.0, 0.5))
            .unwrap_or(0.03);
        let canary_max_sign_test_p_value = call
            .arguments
            .get("canary_max_sign_test_p_value")
            .and_then(|v| v.as_f64())
            .map(|v| v.clamp(0.0001, 1.0))
            .unwrap_or(0.10);
        let replay_log_limit = call
            .arguments
            .get("replay_log_limit")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(100, 100_000))
            .unwrap_or(4_000);

        push_trace_step(
            trace_ref,
            "[evolve]",
            "Self-Evolve Request",
            format!(
                "Requested {} evolution for AgentArk.",
                if mode == "code" || mode == "codebase" {
                    "code"
                } else {
                    "policy"
                }
            ),
            "thinking",
            Some(serde_json::json!({
                "trace_kind": "self_evolve.request",
                "request": request.clone(),
                "mode": mode.clone(),
                "allow_code_writes": allow_code_writes,
                "apply_promotion": apply_promotion,
                "canary_rollout_percent": canary_rollout_percent,
                "canary_min_samples_per_version": canary_min_samples_per_version,
                "canary_min_success_gain": canary_min_success_gain,
                "canary_max_sign_test_p_value": canary_max_sign_test_p_value,
                "replay_log_limit": replay_log_limit,
            })),
            None,
        )
        .await;

        if let Some(tx) = stream_tx {
            let _ = tx.try_send(StreamEvent::ToolStart {
                name: "self_evolve".to_string(),
                payload: None,
            });
        }

        tracing::info!(
            "Self-evolve request mode={} request={}",
            mode,
            &request[..request.len().min(100)]
        );

        let project_root = self.find_project_root();
        let llm = self.llm.clone();

        match mode.as_str() {
            "policy" | "strategy" | "policy_strategy" => {
                let policy_start = std::time::Instant::now();
                let current_policy_raw = self
                    .storage
                    .get(crate::core::self_evolve::ROUTING_COMPLEXITY_POLICY_KEY)
                    .await
                    .ok()
                    .flatten();
                let config = crate::core::self_evolve::PolicyEvolutionConfig {
                    project_root,
                    ..Default::default()
                };
                let evolve_engine =
                    crate::core::self_evolve::PolicyEvolutionEngine::new(config, llm);
                let result = evolve_engine
                    .evolve_routing_policy(&request, current_policy_raw.as_deref())
                    .await?;

                let mut promotion_applied = false;
                let mut canary_state: Option<
                    crate::core::self_evolve::strategy_runtime::CanaryRolloutState,
                > = None;
                let mut replay_result: Option<
                    crate::core::self_evolve::strategy_runtime::ReplayEvaluationResult,
                > = None;
                let mut promoted_directly_to_baseline = false;
                if result.promoted && apply_promotion {
                    if let Some(policy_json) = result.promoted_policy.as_ref() {
                        let candidate_serialized = serde_json::to_vec(policy_json)?;
                        if let Some(existing_baseline) = current_policy_raw.as_ref() {
                            let _ = self
                                .storage
                                .set(
                                    crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_POLICY_BASELINE_SNAPSHOT_KEY,
                                    existing_baseline,
                                )
                                .await;
                        }
                        let baseline_version = self
                            .storage
                            .get(
                                crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY,
                            )
                            .await
                            .ok()
                            .flatten()
                            .and_then(|raw| {
                                serde_json::from_slice::<
                                    crate::core::self_evolve::strategy_runtime::CanaryRolloutState,
                                >(&raw)
                                .ok()
                                .map(|s| s.baseline_version)
                            })
                            .unwrap_or_else(|| "routing-policy-default-v1".to_string());
                        let candidate_version =
                            format!("routing-candidate-{}", result.lineage_entry_id);

                        self.storage
                            .set(
                                crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_POLICY_CANARY_KEY,
                                &candidate_serialized,
                            )
                            .await?;
                        let state =
                            crate::core::self_evolve::strategy_runtime::CanaryRolloutState {
                                enabled: true,
                                baseline_version: baseline_version.clone(),
                                candidate_version: candidate_version.clone(),
                                rollout_percent: canary_rollout_percent,
                                min_samples_per_version: canary_min_samples_per_version,
                                min_success_gain: canary_min_success_gain,
                                max_sign_test_p_value: canary_max_sign_test_p_value,
                                activated_at: Some(chrono::Utc::now().to_rfc3339()),
                            };
                        let state_bytes = serde_json::to_vec(&state)?;
                        self.storage
                            .set(
                                crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY,
                                &state_bytes,
                            )
                            .await?;
                        canary_state = Some(state.clone());

                        if let Ok(logs) = self
                            .storage
                            .list_operational_logs_by_event("tool_call", replay_log_limit)
                            .await
                        {
                            let replay_eval =
                                crate::core::self_evolve::strategy_runtime::evaluate_canary_by_policy_version(
                                    &logs,
                                    &state.baseline_version,
                                    &state.candidate_version,
                                    state.min_samples_per_version,
                                    state.min_success_gain,
                                    state.max_sign_test_p_value,
                                );
                            if replay_eval.promote {
                                self.storage
                                    .set(
                                        crate::core::self_evolve::ROUTING_COMPLEXITY_POLICY_KEY,
                                        &candidate_serialized,
                                    )
                                    .await?;
                                let mut disabled_state = state.clone();
                                disabled_state.enabled = false;
                                let disabled_bytes = serde_json::to_vec(&disabled_state)?;
                                self.storage
                                    .set(
                                        crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY,
                                        &disabled_bytes,
                                    )
                                    .await?;
                                promoted_directly_to_baseline = true;
                                canary_state = Some(disabled_state);
                            }
                            replay_result = Some(replay_eval);
                        }
                        promotion_applied = true;
                    }
                }

                if let Some(tx) = stream_tx {
                    let status_msg = if result.promoted {
                        if promotion_applied {
                            if promoted_directly_to_baseline {
                                format!(
                                    "Policy evolution complete: promoted candidate (gain {:.4}, p={:.4}), replay gate passed, baseline updated immediately",
                                    result.accuracy_gain, result.p_value
                                )
                            } else {
                                format!(
                                    "Policy evolution complete: promoted candidate (gain {:.4}, p={:.4}) activated in canary mode ({}%)",
                                    result.accuracy_gain,
                                    result.p_value,
                                    canary_state
                                        .as_ref()
                                        .map(|s| s.rollout_percent)
                                        .unwrap_or(canary_rollout_percent)
                                )
                            }
                        } else {
                            format!(
                                "Policy evolution complete: candidate passed promotion gate (gain {:.4}, p={:.4}) but not applied",
                                result.accuracy_gain, result.p_value
                            )
                        }
                    } else {
                        format!(
                            "Policy evolution complete: no promotion ({})",
                            result.promotion_gate
                        )
                    };
                    let _ = tx.try_send(StreamEvent::ToolResult {
                        name: "self_evolve".to_string(),
                        content: status_msg,
                    });
                }

                let changed_fields = result
                    .promoted_policy
                    .as_ref()
                    .map(|policy| json_changed_keys(current_policy_raw.as_deref(), policy))
                    .unwrap_or_default();
                let policy_step_type = if result.success && result.promoted {
                    "success"
                } else if result.success {
                    "info"
                } else {
                    "error"
                };
                let policy_detail = if result.success {
                    format!(
                        "Evaluated {} candidate policies. Accuracy {:.0}% → {:.0}% with gate `{}`.",
                        result.evaluated_candidates,
                        result.baseline_accuracy * 100.0,
                        result.best_candidate_accuracy * 100.0,
                        result.promotion_gate
                    )
                } else {
                    format!(
                        "Policy evolution failed: {}",
                        result.error.as_deref().unwrap_or("unknown error")
                    )
                };
                push_trace_step(
                    trace_ref,
                    "[evolve]",
                    "Policy Evolution Evaluated",
                    policy_detail,
                    policy_step_type,
                    Some(serde_json::json!({
                        "trace_kind": "self_evolve.policy.result",
                        "request": request.clone(),
                        "mode": "policy",
                        "target_key": result.target_key.clone(),
                        "success": result.success,
                        "promoted": result.promoted,
                        "evaluated_candidates": result.evaluated_candidates,
                        "baseline_accuracy": result.baseline_accuracy,
                        "best_candidate_accuracy": result.best_candidate_accuracy,
                        "accuracy_gain": result.accuracy_gain,
                        "wins": result.wins,
                        "losses": result.losses,
                        "p_value": result.p_value,
                        "candidate_source": result.candidate_source.clone(),
                        "promotion_gate": result.promotion_gate.clone(),
                        "lineage_entry_id": result.lineage_entry_id.clone(),
                        "lineage_archive_path": result.lineage_archive_path.clone(),
                        "notes": result.notes.clone(),
                        "error": result.error.clone(),
                        "changed_fields": changed_fields.clone(),
                        "promoted_policy": result.promoted_policy.clone(),
                    })),
                    Some(policy_start.elapsed().as_millis() as u64),
                )
                .await;

                let promotion_mode = if promoted_directly_to_baseline {
                    "baseline"
                } else if promotion_applied {
                    "canary"
                } else {
                    "none"
                };
                let promotion_detail = if promoted_directly_to_baseline {
                    "Replay evaluation promoted the candidate directly to baseline.".to_string()
                } else if promotion_applied {
                    format!(
                        "Candidate activated in canary mode at {}% rollout.",
                        canary_state
                            .as_ref()
                            .map(|state| state.rollout_percent)
                            .unwrap_or(canary_rollout_percent)
                    )
                } else if result.promoted {
                    "Candidate passed the promotion gate but was not applied.".to_string()
                } else {
                    format!("No promotion applied because `{}`.", result.promotion_gate)
                };
                push_trace_step(
                    trace_ref,
                    if promotion_applied { "[ok]" } else { "[info]" },
                    "Policy Promotion Decision",
                    promotion_detail,
                    if promotion_applied { "success" } else { "info" },
                    Some(serde_json::json!({
                        "trace_kind": "self_evolve.policy.promotion",
                        "request": request.clone(),
                        "promotion_applied": promotion_applied,
                        "apply_promotion_requested": apply_promotion,
                        "promotion_mode": promotion_mode,
                        "promoted_directly_to_baseline": promoted_directly_to_baseline,
                        "canary_state": canary_state.clone(),
                        "replay_evaluation": replay_result.clone(),
                    })),
                    None,
                )
                .await;

                let mut value = serde_json::to_value(&result)?;
                if let serde_json::Value::Object(obj) = &mut value {
                    obj.insert("mode".to_string(), serde_json::json!("policy"));
                    obj.insert(
                        "promotion_applied".to_string(),
                        serde_json::json!(promotion_applied),
                    );
                    obj.insert(
                        "apply_promotion_requested".to_string(),
                        serde_json::json!(apply_promotion),
                    );
                    obj.insert(
                        "promotion_mode".to_string(),
                        serde_json::json!(if promoted_directly_to_baseline {
                            "baseline"
                        } else if promotion_applied {
                            "canary"
                        } else {
                            "none"
                        }),
                    );
                    obj.insert(
                        "canary_state".to_string(),
                        serde_json::to_value(&canary_state).unwrap_or(serde_json::Value::Null),
                    );
                    obj.insert(
                        "replay_evaluation".to_string(),
                        serde_json::to_value(&replay_result).unwrap_or(serde_json::Value::Null),
                    );
                }
                if let Ok(last_bytes) = serde_json::to_vec(&value) {
                    let _ = self
                        .storage
                        .set(
                            crate::core::self_evolve::strategy_runtime::SELF_EVOLVE_LAST_RESULT_KEY,
                            &last_bytes,
                        )
                        .await;
                }
                // Return human-friendly summary instead of raw JSON
                let summary = if result.success {
                    if result.promoted {
                        let mode_label = if promoted_directly_to_baseline {
                            "applied immediately"
                        } else if promotion_applied {
                            "activated in canary mode for gradual rollout"
                        } else {
                            "ready but not yet applied"
                        };
                        format!(
                            "Self-evolution completed successfully.\n\n\
                            I evaluated {} candidate strategies and found an improvement.\n\
                            - Accuracy improved from {:.0}% to {:.0}% ({} wins, {} losses)\n\
                            - The improved strategy has been {}\n\n\
                            Your agent's decision-making is now more accurate.",
                            result.evaluated_candidates,
                            result.baseline_accuracy * 100.0,
                            result.best_candidate_accuracy * 100.0,
                            result.wins,
                            result.losses,
                            mode_label,
                        )
                    } else {
                        format!(
                            "Self-evolution completed. I evaluated {} candidate strategies \
                            but none outperformed the current approach (accuracy: {:.0}%). \
                            No changes were made.",
                            result.evaluated_candidates,
                            result.baseline_accuracy * 100.0,
                        )
                    }
                } else {
                    format!(
                        "Self-evolution ran but encountered an issue: {}",
                        result.error.as_deref().unwrap_or("unknown error")
                    )
                };
                Ok(summary)
            }
            "code" | "codebase" => {
                if !allow_code_writes {
                    push_trace_step(
                        trace_ref,
                        "[warn]",
                        "Code Evolution Blocked",
                        "Code evolution requires explicit `allow_code_writes=true` before AgentArk will modify its own code.",
                        "warning",
                        Some(serde_json::json!({
                            "trace_kind": "self_evolve.code.blocked",
                            "request": request.clone(),
                            "mode": "code",
                            "allow_code_writes": allow_code_writes,
                        })),
                        None,
                    )
                    .await;
                    return Ok(serde_json::json!({
                        "status": "blocked",
                        "mode": "code",
                        "message": "Code evolution is disabled by default. Re-run self_evolve with mode='code' and allow_code_writes=true after policy evolution is stable."
                    })
                    .to_string());
                }

                let code_start = std::time::Instant::now();
                let config = crate::core::self_evolve::SelfEvolveConfig {
                    max_iterations: 25,
                    max_build_fix_cycles: 5,
                    project_root,
                };
                let evolve_agent = crate::core::self_evolve::SelfEvolveAgent::new(config, llm);
                let result = evolve_agent.execute(&request).await?;

                if let Some(tx) = stream_tx {
                    let status_msg = if result.success {
                        let mut msg = format!(
                            "Code evolution complete: {} files changed in {} iterations",
                            result.files_changed.len(),
                            result.iterations_used
                        );
                        if result.push_recommended {
                            msg.push_str(
                                ". Local changes are ready; ask the user whether to push to remote.",
                            );
                        }
                        msg
                    } else {
                        format!(
                            "Code evolution failed: {}",
                            result.error.as_deref().unwrap_or("unknown error")
                        )
                    };
                    let _ = tx.try_send(StreamEvent::ToolResult {
                        name: "self_evolve".to_string(),
                        content: status_msg,
                    });
                }

                push_trace_step(
                    trace_ref,
                    if result.success { "[ok]" } else { "[error]" },
                    if result.success {
                        "Code Evolution Completed"
                    } else {
                        "Code Evolution Failed"
                    },
                    if result.success {
                        format!(
                            "Changed {} files over {} iteration(s).",
                            result.files_changed.len(),
                            result.iterations_used
                        )
                    } else {
                        format!(
                            "Code evolution failed after {} iteration(s).",
                            result.iterations_used
                        )
                    },
                    if result.success { "success" } else { "error" },
                    Some(serde_json::json!({
                        "trace_kind": "self_evolve.code.result",
                        "request": request.clone(),
                        "mode": "code",
                        "success": result.success,
                        "diff_summary": result.diff_summary.clone(),
                        "files_changed": result.files_changed.clone(),
                        "iterations_used": result.iterations_used,
                        "error": result.error.clone(),
                        "security_warnings": result.security_warnings.clone(),
                        "push_recommended": result.push_recommended,
                        "push_suggestion": result.push_suggestion.clone(),
                    })),
                    Some(code_start.elapsed().as_millis() as u64),
                )
                .await;

                Ok(serde_json::to_string_pretty(&result)?)
            }
            _ => {
                push_trace_step(
                    trace_ref,
                    "[error]",
                    "Self-Evolve Mode Rejected",
                    format!("Unsupported self_evolve mode '{}'.", mode),
                    "error",
                    Some(serde_json::json!({
                        "trace_kind": "self_evolve.mode_error",
                        "request": request.clone(),
                        "mode": mode.clone(),
                    })),
                    None,
                )
                .await;
                Ok(serde_json::json!({
                    "status": "error",
                    "message": format!(
                        "Unsupported self_evolve mode '{}'. Use mode='policy' (default) or mode='code'.",
                        mode
                    ),
                })
                .to_string())
            }
        }
    }
    /// Determine the project root (where Cargo.toml lives).
    fn find_project_root(&self) -> std::path::PathBuf {
        // In Docker, the app is at /app
        let app_path = std::path::Path::new("/app");
        if app_path.join("Cargo.toml").exists() {
            return app_path.to_path_buf();
        }
        // In development, walk up from current dir
        if let Ok(cwd) = std::env::current_dir() {
            let mut dir = cwd.as_path();
            loop {
                if dir.join("Cargo.toml").exists() {
                    return dir.to_path_buf();
                }
                match dir.parent() {
                    Some(parent) => dir = parent,
                    None => break,
                }
            }
        }
        // Fallback
        std::path::PathBuf::from(".")
    }

    /// Execute tool calls from LLM response using modular handler dispatch.
    pub(crate) async fn execute_tool_calls(
        &self,
        response: &crate::core::llm::LlmResponse,
        trace_ref: &Arc<RwLock<ExecutionTrace>>,
        stream_tx: Option<tokio::sync::mpsc::Sender<StreamEvent>>,
        ctx: ToolExecutionContext<'_>,
    ) -> Result<ToolExecutionBatch> {
        if response.tool_calls.is_empty() {
            return Ok(ToolExecutionBatch::default());
        }
        let request_channel = ctx.request_channel;
        let current_turn_is_explicit_approval = ctx.current_turn_is_explicit_approval;
        let trace_id = ctx.trace_id;
        let conversation_id = ctx.conversation_id;
        let project_id = ctx.project_id;
        let strategy_version = ctx.strategy_version;
        let policy_version = ctx.policy_version;
        let prompt_version = ctx.prompt_version;
        let model_slot = ctx.model_slot;

        let public_base_url = self.load_public_base_url().await;
        let integration_aliases = self.load_tool_integration_aliases().await;
        let handlers = default_tool_handlers();
        let action_map = self
            .runtime
            .list_actions()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|action| (action.name.to_ascii_lowercase(), action))
            .collect::<HashMap<_, _>>();
        let user_execution_constraints = self.load_user_execution_constraints(project_id).await;
        let mut side_effect_cache: HashMap<String, bool> = HashMap::new();

        let mut seen_signatures: HashSet<String> = HashSet::new();
        let mut unique_calls: Vec<&crate::core::llm::ToolCall> = Vec::new();
        for call in &response.tool_calls {
            let sig = Self::tool_call_signature(call);
            if seen_signatures.insert(sig) {
                unique_calls.push(call);
            }
        }

        let mut results: Vec<ToolCallOutput> = Vec::new();
        for call in unique_calls {
            let call_started = std::time::Instant::now();
            let action_def = action_map.get(&call.name.to_ascii_lowercase());
            let ctx = ToolHandlerContext {
                trace_ref,
                stream_tx: stream_tx.as_ref(),
                request_channel,
                conversation_id,
                project_id,
                public_base_url: public_base_url.as_deref(),
                integration_aliases: &integration_aliases,
            };

            let side_effecting = if let Some(cached) = side_effect_cache
                .get(&Self::tool_call_signature(call))
                .copied()
            {
                cached
            } else {
                let classified = self
                    .classify_tool_call_side_effecting(call, action_def)
                    .await;
                side_effect_cache.insert(Self::tool_call_signature(call), classified);
                classified
            };

            if !current_turn_is_explicit_approval
                && (user_execution_constraints.require_explicit_approval_before_side_effects
                    || user_execution_constraints.show_plan_before_side_effects)
                && side_effecting
            {
                let msg = blocked_by_saved_rule_message(&user_execution_constraints);
                let payload = serde_json::json!({
                    "handler": "user_execution_constraints",
                    "output_preview": safe_truncate(&msg, 260),
                });
                self.log_operational_event(super::operational::OperationalEvent {
                    event_type: "tool_call",
                    channel: request_channel,
                    success: false,
                    outcome: "blocked_by_saved_user_rule",
                    trace_id,
                    conversation_id,
                    tool_name: Some(&call.name),
                    latency_ms: Some(call_started.elapsed().as_millis() as u64),
                    arguments: Some(&call.arguments),
                    payload: Some(&payload),
                    strategy_version,
                    policy_version,
                    prompt_version,
                    model_slot,
                })
                .await;
                if let Some(ref tx) = stream_tx {
                    let _ = tx.try_send(StreamEvent::ToolResult {
                        name: call.name.clone(),
                        content: msg.clone(),
                    });
                }
                results.push(ToolCallOutput {
                    name: call.name.clone(),
                    content: msg,
                });
                continue;
            }

            let mut handled = false;
            for handler in &handlers {
                if !handler.can_handle(self, call, &ctx) {
                    continue;
                }
                tracing::debug!("Tool '{}' handled by '{}'", call.name, handler.id());
                match handler.handle(self, call, &ctx).await {
                    Ok(Some(output)) => {
                        let latency_ms = call_started.elapsed().as_millis() as u64;
                        let lowered = output.trim().to_ascii_lowercase();
                        let blocked = lowered.contains("blocked by safety policy");
                        let success = !(lowered.starts_with("error ")
                            || lowered.starts_with("error:")
                            || blocked);
                        let outcome = if blocked {
                            "blocked"
                        } else if success {
                            "ok"
                        } else {
                            "error_text"
                        };
                        let payload = serde_json::json!({
                            "handler": handler.id(),
                            "output_preview": safe_truncate(&output, 260),
                        });
                        self.log_operational_event(super::operational::OperationalEvent {
                            event_type: "tool_call",
                            channel: request_channel,
                            success,
                            outcome,
                            trace_id,
                            conversation_id,
                            tool_name: Some(&call.name),
                            latency_ms: Some(latency_ms),
                            arguments: Some(&call.arguments),
                            payload: Some(&payload),
                            strategy_version,
                            policy_version,
                            prompt_version,
                            model_slot,
                        })
                        .await;
                        if !blocked {
                            self.record_self_tune_tool_outcome(&call.name, success, latency_ms)
                                .await;
                        }
                        results.push(ToolCallOutput {
                            name: call.name.clone(),
                            content: output,
                        });
                        handled = true;
                        break;
                    }
                    Ok(None) => continue,
                    Err(e) => {
                        let latency_ms = call_started.elapsed().as_millis() as u64;
                        let error_text = e.to_string();
                        let payload = serde_json::json!({
                            "handler": handler.id(),
                            "error": safe_truncate(&error_text, 260),
                        });
                        self.log_operational_event(super::operational::OperationalEvent {
                            event_type: "tool_call",
                            channel: request_channel,
                            success: false,
                            outcome: "handler_error",
                            trace_id,
                            conversation_id,
                            tool_name: Some(&call.name),
                            latency_ms: Some(latency_ms),
                            arguments: Some(&call.arguments),
                            payload: Some(&payload),
                            strategy_version,
                            policy_version,
                            prompt_version,
                            model_slot,
                        })
                        .await;
                        self.record_self_tune_tool_outcome(&call.name, false, latency_ms)
                            .await;
                        return Err(e);
                    }
                }
            }

            if !handled {
                let latency_ms = call_started.elapsed().as_millis() as u64;
                let msg = format!("No handler registered for tool '{}'", call.name);
                let payload = serde_json::json!({
                    "handler": "none",
                    "output_preview": safe_truncate(&msg, 260),
                });
                self.log_operational_event(super::operational::OperationalEvent {
                    event_type: "tool_call",
                    channel: request_channel,
                    success: false,
                    outcome: "no_handler",
                    trace_id,
                    conversation_id,
                    tool_name: Some(&call.name),
                    latency_ms: Some(latency_ms),
                    arguments: Some(&call.arguments),
                    payload: Some(&payload),
                    strategy_version,
                    policy_version,
                    prompt_version,
                    model_slot,
                })
                .await;
                self.record_self_tune_tool_outcome(&call.name, false, latency_ms)
                    .await;
                if let Some(ref tx) = stream_tx {
                    let _ = tx.try_send(StreamEvent::ToolResult {
                        name: call.name.clone(),
                        content: msg.clone(),
                    });
                }
                results.push(ToolCallOutput {
                    name: call.name.clone(),
                    content: msg,
                });
            }
        }

        Ok(ToolExecutionBatch { outputs: results })
    }

    /// Legacy monolithic tool execution path. New dispatchers route through
    /// modular handlers and can gradually replace this implementation.
    pub(crate) async fn execute_tool_calls_legacy(
        &self,
        response: &crate::core::llm::LlmResponse,
        trace_ref: &Arc<RwLock<ExecutionTrace>>,
        stream_tx: Option<tokio::sync::mpsc::Sender<StreamEvent>>,
        request_channel: &str,
    ) -> Result<String> {
        if response.tool_calls.is_empty() {
            return Ok(response.content.clone());
        }

        let mut results = Vec::new();
        let conversation_id = self.last_conversation_id.read().await.clone();
        let conversation_id = conversation_id.as_deref();
        let sanitize_stream = |s: &str| -> String { self.sanitize_stream_preview(s) };
        let public_base_url = self.load_public_base_url().await;
        let integration_aliases = self.load_tool_integration_aliases().await;
        let absolutize_url =
            |url: &str| -> String { Self::absolutize_public_url(public_base_url.as_deref(), url) };

        // Deduplicate repeated tool calls (same name + identical args) so app_deploy
        // and other side-effecting actions do not run twice from merged paths.
        let mut seen_signatures: HashSet<String> = HashSet::new();
        let mut unique_calls: Vec<&crate::core::llm::ToolCall> = Vec::new();
        for call in &response.tool_calls {
            let sig = Self::tool_call_signature(call);
            if seen_signatures.insert(sig) {
                unique_calls.push(call);
            }
        }

        for call in unique_calls {
            if let Some(ref tx) = stream_tx {
                let payload = if call.name == "app_deploy" {
                    Some(Self::summarize_app_deploy_stream_payload(&call.arguments))
                } else {
                    None
                };
                let _ = tx.try_send(StreamEvent::ToolStart {
                    name: call.name.clone(),
                    payload,
                });
            }

            // Check safety policy
            let allowed = self.safety.is_allowed(&call.name, &call.arguments).await?;
            if !allowed {
                let blocked = format!("Tool '{}' blocked by safety policy", call.name);
                if let Some(ref tx) = stream_tx {
                    let _ = tx.try_send(StreamEvent::ToolResult {
                        name: call.name.clone(),
                        content: blocked.clone(),
                    });
                }
                results.push(blocked);
                continue;
            }

            // Handle generate_image via integrations (not runtime)
            if call.name == "generate_image" {
                // Inject configured model if not specified in the call
                let mut args = call.arguments.clone();
                if args.get("model").and_then(|v| v.as_str()).is_none() {
                    if let Some(ref model) = self.config.media_gen.image_model {
                        args["model"] = serde_json::Value::String(model.clone());
                    }
                }
                match self
                    .integrations
                    .execute("media_gen", "generate_image", &args)
                    .await
                {
                    Ok(result) => {
                        if let Some(url) = result.get("url").and_then(|v| v.as_str()) {
                            let provider = result
                                .get("provider")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            let time_ms = result
                                .get("generation_time_ms")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let formatted = format!(
                                "[IMAGE_RESULT]{}\n[/IMAGE_RESULT]\n*Generated by {} in {}ms*",
                                url, provider, time_ms
                            );
                            if let Some(ref tx) = stream_tx {
                                let _ = tx.try_send(StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: format!(
                                        "Generated image via {} ({}ms)",
                                        provider, time_ms
                                    ),
                                });
                            }
                            results.push(formatted);
                        } else {
                            let formatted = format!("Image generated: {}", result);
                            if let Some(ref tx) = stream_tx {
                                let _ = tx.try_send(StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: sanitize_stream(&formatted),
                                });
                            }
                            results.push(formatted);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Image generation error: {}", e);
                        let formatted = format!("Error generating image: {}", e);
                        if let Some(ref tx) = stream_tx {
                            let _ = tx.try_send(StreamEvent::ToolResult {
                                name: call.name.clone(),
                                content: formatted.clone(),
                            });
                        }
                        results.push(formatted);
                    }
                }
                continue;
            }

            // Handle provider-based video generation via integrations (not runtime)
            if call.name == "generate_video" {
                match self
                    .integrations
                    .execute("media_gen", "generate_video", &call.arguments)
                    .await
                {
                    Ok(result) => {
                        if let Some(url) = result.get("url").and_then(|v| v.as_str()) {
                            let provider = result
                                .get("provider")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            let model = result
                                .get("model")
                                .and_then(|v| v.as_str())
                                .unwrap_or("default");
                            let mut source_url = url.to_string();
                            let mut video_bytes: Option<Vec<u8>> = None;

                            // Convert data URLs into persisted output files so links remain usable.
                            if source_url.starts_with("data:") {
                                match self.load_video_bytes(&source_url, 80 * 1024 * 1024).await {
                                    Ok(bytes) => {
                                        video_bytes = Some(bytes.clone());
                                        match self
                                            .persist_output_binary("provider_video", "mp4", &bytes)
                                            .await
                                        {
                                            Ok(local_url) => source_url = local_url,
                                            Err(e) => tracing::warn!(
                                                "Failed to persist provider data URL video: {}",
                                                e
                                            ),
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to decode provider data URL video: {}",
                                            e
                                        );
                                    }
                                }
                            }

                            let rendered_url = absolutize_url(&source_url);
                            let mut preview_url: Option<String> = None;

                            // Build preview screenshot for all provider videos.
                            if video_bytes.is_none() {
                                if let Ok(bytes) =
                                    self.load_video_bytes(&source_url, 45 * 1024 * 1024).await
                                {
                                    video_bytes = Some(bytes);
                                }
                            }
                            if let Some(bytes) = video_bytes.as_ref() {
                                match self.extract_video_preview_from_bytes(bytes).await {
                                    Ok(preview_bytes) => {
                                        if let Ok(rel) = self
                                            .persist_output_binary(
                                                "provider_video_preview",
                                                "jpg",
                                                &preview_bytes,
                                            )
                                            .await
                                        {
                                            let abs = absolutize_url(&rel);
                                            preview_url = Some(abs.clone());
                                            if matches!(request_channel, "telegram" | "whatsapp") {
                                                let _ = crate::channels::send_screenshot(
                                                    self,
                                                    request_channel,
                                                    &preview_bytes,
                                                    "Video preview",
                                                    Some(&abs),
                                                )
                                                .await;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to extract provider video preview: {}",
                                            e
                                        );
                                    }
                                }
                            }

                            // Direct attachment where reliable: Telegram and WhatsApp(Baileys).
                            let mut delivered_to_channel = false;
                            let whatsapp_baileys = self
                                .config
                                .whatsapp
                                .as_ref()
                                .map(|cfg| {
                                    matches!(
                                        cfg.mode,
                                        crate::channels::whatsapp::WhatsAppMode::Baileys
                                    )
                                })
                                .unwrap_or(false);
                            let should_direct_send = request_channel == "telegram"
                                || (request_channel == "whatsapp" && whatsapp_baileys);

                            if should_direct_send {
                                if video_bytes.is_none() {
                                    if let Ok(bytes) =
                                        self.load_video_bytes(&source_url, 80 * 1024 * 1024).await
                                    {
                                        video_bytes = Some(bytes);
                                    }
                                }
                                if let Some(bytes) = video_bytes.as_ref() {
                                    let caption =
                                        format!("Video generated by {} ({})", provider, model);
                                    if crate::channels::send_video_to_channel(
                                        self,
                                        request_channel,
                                        bytes,
                                        &caption,
                                        Some(&rendered_url),
                                    )
                                    .await
                                    .is_ok()
                                    {
                                        delivered_to_channel = true;
                                    }
                                }
                            }

                            let preview_text = preview_url
                                .as_ref()
                                .map(|u| format!("\nPreview: {}", u))
                                .unwrap_or_default();
                            let formatted = if matches!(request_channel, "telegram" | "whatsapp") {
                                if delivered_to_channel {
                                    format!(
                                        "Video sent to this chat.\nDownload: {}{}",
                                        rendered_url, preview_text
                                    )
                                } else {
                                    format!(
                                        "Video generated via {} ({}): {}\n{}",
                                        provider,
                                        model,
                                        rendered_url,
                                        if let Some(p) = preview_url.as_ref() {
                                            format!("Preview: {}", p)
                                        } else {
                                            "Preview unavailable".to_string()
                                        }
                                    )
                                }
                            } else if let Some(preview) = preview_url.as_ref() {
                                format!(
                                    "[VIDEO_RESULT]{}\n[/VIDEO_RESULT]\n[IMAGE_RESULT]{}\n[/IMAGE_RESULT]\n*Generated by {} ({})*",
                                    rendered_url, preview, provider, model
                                )
                            } else {
                                format!(
                                    "[VIDEO_RESULT]{}\n[/VIDEO_RESULT]\n*Generated by {} ({})*",
                                    rendered_url, provider, model
                                )
                            };
                            if let Some(ref tx) = stream_tx {
                                let _ = tx.try_send(StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: format!("Generated video via {}", provider),
                                });
                            }
                            results.push(formatted);
                        } else {
                            let formatted = format!("Video generated: {}", result);
                            if let Some(ref tx) = stream_tx {
                                let _ = tx.try_send(StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: sanitize_stream(&formatted),
                                });
                            }
                            results.push(formatted);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Provider video generation error: {}", e);
                        let formatted = format!("Error generating video: {}", e);
                        if let Some(ref tx) = stream_tx {
                            let _ = tx.try_send(StreamEvent::ToolResult {
                                name: call.name.clone(),
                                content: formatted.clone(),
                            });
                        }
                        results.push(formatted);
                    }
                }
                continue;
            }

            // Handle browser automation - starts a background session
            if call.name == "browser_auto" {
                let sub_action = call
                    .arguments
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("start_session");

                if sub_action == "start_session" {
                    let task_desc = call
                        .arguments
                        .get("task")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Browse the web");
                    let channel = call
                        .arguments
                        .get("channel")
                        .and_then(|v| v.as_str())
                        .unwrap_or("web");

                    if !self.browser_sessions.is_available().await {
                        tracing::warn!(
                            "Browser automation unavailable: Playwright sidecar not reachable"
                        );
                        let formatted = r#"{"error": "browser_unavailable", "detail": "Playwright sidecar is not running"}"#.to_string();
                        if let Some(ref tx) = stream_tx {
                            let _ = tx.try_send(StreamEvent::ToolResult {
                                name: call.name.clone(),
                                content: formatted.clone(),
                            });
                        }
                        results.push(formatted);
                        continue;
                    }

                    if self.browser_sessions.active_count() >= 2 {
                        tracing::warn!("Browser session limit reached: 2 active sessions");
                        let formatted = r#"{"error": "session_limit", "detail": "Maximum 2 concurrent browser sessions"}"#.to_string();
                        if let Some(ref tx) = stream_tx {
                            let _ = tx.try_send(StreamEvent::ToolResult {
                                name: call.name.clone(),
                                content: formatted.clone(),
                            });
                        }
                        results.push(formatted);
                        continue;
                    }

                    // Create a notification callback that sends messages to the user's channel
                    let chat_id = call
                        .arguments
                        .get("chat_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let notify_channel = channel.to_string();
                    let agent_config = self.config.clone();
                    let storage_clone = self.storage.clone();
                    let encrypted_storage_clone = self.encrypted_storage.clone();
                    let notify_conversation_id = call
                        .arguments
                        .get("conversation_id")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    let notify_fn: std::sync::Arc<dyn Fn(String, Option<Vec<u8>>) + Send + Sync> =
                        std::sync::Arc::new(move |msg: String, screenshot: Option<Vec<u8>>| {
                            let config = agent_config.clone();
                            let channel = notify_channel.clone();
                            let chat_id = chat_id.clone();
                            let storage = storage_clone.clone();
                            let encrypted_storage = encrypted_storage_clone.clone();
                            let conversation_id = notify_conversation_id.clone();
                            let _screenshot = screenshot; // screenshots sent via channel-specific methods
                            tokio::spawn(async move {
                                // Store as notification in DB so it appears in web UI
                                let notif = crate::storage::entities::notification::Model {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    title: "Browser Automation".to_string(),
                                    body: msg.clone(),
                                    level: "info".to_string(),
                                    source: "browser".to_string(),
                                    read: false,
                                    created_at: chrono::Utc::now().to_rfc3339(),
                                };
                                let _ = storage.insert_notification(&notif).await;

                                // Also append to conversation so browser prompts are visible in chat thread.
                                if let Some(cid) = conversation_id.as_deref() {
                                    let body = format!(
                                        "[Browser automation] {}\nReply here in chat to continue.",
                                        msg
                                    );
                                    let asst_msg = crate::storage::entities::message::Model {
                                        id: uuid::Uuid::new_v4().to_string(),
                                        conversation_id: cid.to_string(),
                                        role: "assistant".to_string(),
                                        content: body,
                                        timestamp: chrono::Utc::now().to_rfc3339(),
                                        model_used: Some("browser_auto".to_string()),
                                        trace_id: None,
                                    };
                                    let _ =
                                        encrypted_storage.insert_message_encrypted(&asst_msg).await;
                                }

                                // Send to Telegram if configured
                                #[cfg(feature = "telegram")]
                                if channel == "telegram" {
                                    if let Some(tg) = &config.telegram {
                                        if !tg.bot_token.is_empty() {
                                            let target = if !chat_id.is_empty() {
                                                chat_id.parse::<i64>().unwrap_or(0)
                                            } else if let Some(first) = tg.allowed_users.first() {
                                                *first
                                            } else {
                                                0
                                            };
                                            if target != 0 {
                                                use teloxide::requests::Requester;
                                                let bot = teloxide::Bot::new(&tg.bot_token);
                                                let _ = bot
                                                    .send_message(
                                                        teloxide::types::ChatId(target),
                                                        &msg,
                                                    )
                                                    .await;
                                            }
                                        }
                                    }
                                }
                                let _ = channel; // suppress unused warning on non-telegram builds
                            });
                        });

                    let llm_clone = self.llm.clone();
                    match self
                        .browser_sessions
                        .start_session(task_desc, channel, "", llm_clone, notify_fn)
                        .await
                    {
                        Ok(session_id) => {
                            tracing::info!(
                                "Browser session started: session={}, task_len={}",
                                &session_id[..8],
                                task_desc.len()
                            );
                            // Return structured data - let the LLM craft the user message
                            if let Some(ref tx) = stream_tx {
                                let _ = tx.try_send(StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: format!(
                                        "Browser session started: {}",
                                        &session_id[..8]
                                    ),
                                });
                            }
                            results.push(format!(
                                r#"{{"status": "session_started", "session_id": "{}", "task": "{}"}}"#,
                                session_id, task_desc.replace('"', "'")
                            ));
                        }
                        Err(e) => {
                            tracing::error!("Browser session start failed: error={}", e);
                            if let Some(ref tx) = stream_tx {
                                let _ = tx.try_send(StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: format!("Browser session start failed: {}", e),
                                });
                            }
                            results.push(format!(
                                r#"{{"error": "session_start_failed", "detail": "{}"}}"#,
                                e
                            ));
                        }
                    }
                } else {
                    // Direct browser actions (for manual control)
                    let integration = self.browser_sessions.integration();
                    let resolved_args = self
                        .runtime
                        .resolve_secret_placeholders(&call.name, &call.arguments)
                        .unwrap_or_else(|_| call.arguments.clone());
                    match self
                        .integrations
                        .execute("browser", sub_action, &resolved_args)
                        .await
                    {
                        Ok(result) => {
                            let formatted = serde_json::to_string_pretty(&result)
                                .unwrap_or_else(|_| result.to_string());
                            if let Some(ref tx) = stream_tx {
                                let _ = tx.try_send(StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: sanitize_stream(&formatted),
                                });
                            }
                            results.push(formatted);
                        }
                        Err(e) => {
                            // Try via direct integration
                            let _ = integration; // used in future expansion
                            let formatted = format!("Browser action error: {}", e);
                            if let Some(ref tx) = stream_tx {
                                let _ = tx.try_send(StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: formatted.clone(),
                                });
                            }
                            results.push(formatted);
                        }
                    }
                }
                continue;
            }

            // Handle integration-backed tools via dynamic routing aliases + direct integration IDs.
            if let Some(integration_id) =
                self.resolve_tool_integration_id(&call.name, &integration_aliases)
            {
                let formatted = self
                    .execute_integration_tool_call(
                        call,
                        trace_ref,
                        stream_tx.as_ref(),
                        request_channel,
                        &integration_id,
                    )
                    .await;
                results.push(formatted);
                continue;
            }

            // Handle app deployment - needs AppRegistry from agent
            if call.name == "app_deploy" {
                let normalized_args = Self::normalize_app_deploy_arguments(&call.arguments);
                // Resolve secret placeholders for deployment-time env injection, without mutating
                // the original tool arguments (so traces stay safe).
                let mut resolved_args = self
                    .runtime
                    .resolve_secret_placeholders(&call.name, &normalized_args)
                    .unwrap_or(normalized_args);
                if resolved_args
                    .get("access_guard")
                    .and_then(|v| v.as_bool())
                    .is_none()
                {
                    let guard_default = self
                        .storage
                        .get(
                            crate::core::self_evolve::strategy_runtime::APP_DEPLOY_ACCESS_GUARD_DEFAULT_KEY,
                        )
                        .await
                        .ok()
                        .flatten()
                        .and_then(|raw| String::from_utf8(raw).ok())
                        .map(|s| s.trim().eq_ignore_ascii_case("true"))
                        .unwrap_or(false);
                    if let Some(obj) = resolved_args.as_object_mut() {
                        obj.insert("access_guard".to_string(), serde_json::json!(guard_default));
                    }
                }
                if resolved_args
                    .get("runtime_preference")
                    .and_then(|v| v.as_str())
                    .is_none()
                {
                    if let Some(obj) = resolved_args.as_object_mut() {
                        obj.insert("runtime_preference".to_string(), serde_json::json!("local"));
                    }
                }
                if resolved_args
                    .get("expose_public")
                    .and_then(|v| v.as_bool())
                    .is_none()
                {
                    if let Some(obj) = resolved_args.as_object_mut() {
                        obj.insert("expose_public".to_string(), serde_json::json!(true));
                    }
                }
                let expose_public_requested = resolved_args
                    .get("expose_public")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let replace_existing_requested = resolved_args
                    .get("replace_existing")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if !replace_existing_requested {
                    if let Some(duplicate_match) =
                        self.find_existing_duplicate_app(&resolved_args).await
                    {
                        let existing = &duplicate_match.app;
                        let existing_id = existing
                            .get("id")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|id| !id.is_empty())
                            .unwrap_or("app");
                        let existing_id_for_cleanup = existing
                            .get("id")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|id| !id.is_empty());
                        let existing_title = existing
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Existing app");
                        let existing_running = existing
                            .get("running")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let existing_url = existing
                            .get("access_url")
                            .and_then(|v| v.as_str())
                            .or_else(|| existing.get("url").and_then(|v| v.as_str()))
                            .unwrap_or("/apps/");
                        let local_base = Self::user_facing_local_base_url();
                        let local_url =
                            Self::absolutize_public_url(Some(local_base.as_str()), existing_url);
                        let public_url = if expose_public_requested {
                            self.load_public_base_url().await.map(|base| {
                                Self::absolutize_public_url(Some(base.as_str()), existing_url)
                            })
                        } else {
                            None
                        };
                        let mut duplicate_msg_lines = vec![
                            format!(
                                "Found an existing deployed app for this request: **{}** (`{}`).",
                                existing_title, existing_id
                            ),
                            format!(
                                "- Similarity: {} ({:.0}% confidence, {})",
                                duplicate_match.match_kind,
                                duplicate_match.score * 100.0,
                                duplicate_match.reason
                            ),
                            format!("- Local: {}", local_url),
                        ];
                        if let Some(public_url) = public_url {
                            duplicate_msg_lines.push(format!("- Public: {}", public_url));
                        }

                        match Self::resolve_duplicate_app(
                            duplicate_match.match_kind,
                            existing_running,
                        ) {
                            DuplicateAppResolution::ReuseExisting => {
                                duplicate_msg_lines.push(
                                    "Auto-resolution: reusing the existing deployment (exact file match + healthy runtime)."
                                        .to_string(),
                                );
                                let duplicate_msg = duplicate_msg_lines.join("\n");
                                if let Some(ref tx) = stream_tx {
                                    let _ = tx.try_send(StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: duplicate_msg.clone(),
                                    });
                                }
                                results.push(duplicate_msg);
                                continue;
                            }
                            DuplicateAppResolution::ReplaceExisting => {
                                let cleanup_note = if let Some(app_id) = existing_id_for_cleanup {
                                    match self
                                        .stop_and_remove_existing_app(app_id, Some(existing_title))
                                        .await
                                    {
                                        Ok(_) => {
                                            "Auto-resolution: replacing existing deployment before redeploy."
                                                .to_string()
                                        }
                                        Err(error) => {
                                            tracing::warn!(
                                                "failed to cleanup duplicate app {} before redeploy: {}",
                                                app_id,
                                                error
                                            );
                                            format!(
                                                "Auto-resolution: continuing with redeploy; cleanup of prior deployment failed: {}",
                                                error
                                            )
                                        }
                                    }
                                } else {
                                    "Auto-resolution: existing app id missing, continuing with redeploy."
                                        .to_string()
                                };
                                if let Some(obj) = resolved_args.as_object_mut() {
                                    obj.insert(
                                        "replace_existing".to_string(),
                                        serde_json::json!(true),
                                    );
                                }
                                duplicate_msg_lines.push(cleanup_note);
                                let duplicate_msg = duplicate_msg_lines.join("\n");
                                if let Some(ref tx) = stream_tx {
                                    let _ = tx.try_send(StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: duplicate_msg,
                                    });
                                }
                            }
                        }
                    }
                }
                let hook_event_id = uuid::Uuid::new_v4().to_string();
                let hook_hint = action_message_hint(&resolved_args);
                self.fire_action_hook(
                    crate::hooks::HookTrigger::PreAction,
                    request_channel,
                    &call.name,
                    hook_hint.as_deref(),
                    None,
                    &hook_event_id,
                )
                .await;
                let llm_env = self.app_model_env_vars();
                match crate::actions::app::app_deploy(
                    &self.config_dir,
                    &self.data_dir,
                    &resolved_args,
                    &self.app_registry,
                    &llm_env,
                    stream_tx.clone(),
                )
                .await
                {
                    Ok(result) => {
                        self.trigger_arkpulse_refresh("app_deploy");
                        self.fire_action_hook(
                            crate::hooks::HookTrigger::PostAction,
                            request_channel,
                            &call.name,
                            hook_hint.as_deref(),
                            Some(&result),
                            &hook_event_id,
                        )
                        .await;
                        // Parse result to extract URL for a nice response
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&result) {
                            if parsed
                                .get("status")
                                .and_then(|v| v.as_str())
                                .is_some_and(|s| s == "needs_secrets")
                            {
                                let title = parsed
                                    .get("title")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("App");
                                let app_id = parsed
                                    .get("app_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("app");
                                let missing = parsed
                                    .get("missing_env")
                                    .and_then(|v| v.as_array())
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|v| v.as_str())
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    })
                                    .unwrap_or_else(|| "unknown".to_string());
                                let missing_config = parsed
                                    .get("missing_config")
                                    .and_then(|v| v.as_array())
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|v| v.as_str())
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    })
                                    .unwrap_or_default();
                                let llm_reuse_candidates = parsed
                                    .get("llm_reuse_candidates")
                                    .and_then(|v| v.as_array())
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|v| v.as_str())
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    })
                                    .unwrap_or_default();
                                let reuse_option = if llm_reuse_candidates.is_empty() {
                                    "1) Use an existing model key: not available for current missing keys.".to_string()
                                } else {
                                    format!(
                                        "1) Reuse your current model key for: {}.\n   Reply: use current llm key for <KEY>",
                                        llm_reuse_candidates
                                    )
                                };
                                let public_access_note = if expose_public_requested {
                                    match self
                                        .ensure_public_tunnel_base_url(Some(&app_id), stream_tx.as_ref())
                                        .await
                                    {
                                        Some(base) => {
                                            format!("\nPublic access URL: {}/apps/{}/", base, app_id)
                                        }
                                        None => "\nPublic tunnel URL is pending; I started tunnel setup and will use it once available.".to_string(),
                                    }
                                } else {
                                    String::new()
                                };
                                let msg = format!(
                                    "App '{}' is ready, but I need your approval/input for credentials before I continue.\n\
                                      Missing sensitive keys: {}{}\n\n\
                                      Choose one option:\n\
                                      {}\n\
                                     2) Provide your own key securely.\n\
                                        Reply: set secret <KEY>=<VALUE>\n\n\
                                      Why I'm asking: credentials are stored encrypted and handled outside model generation to reduce leak risk.\n\
                                      For non-sensitive config values, redeploy/restart with config.{{KEY}}=value.\n\
                                      Then restart app '{}'.{}",
                                     title,
                                     if missing.is_empty() { "none" } else { &missing },
                                     if missing_config.is_empty() { "".to_string() } else { format!("\nMissing config values: {}", missing_config) },
                                     reuse_option,
                                     app_id,
                                     public_access_note
                                );
                                if let Some(cid) =
                                    conversation_id.filter(|value| !value.trim().is_empty())
                                {
                                    self.remember_pending_secret_followup(
                                        cid,
                                        PendingSecretFollowupKind::RestartApp {
                                            app_id: app_id.to_string(),
                                            title: title.to_string(),
                                            missing_env: parsed
                                                .get("missing_env")
                                                .and_then(|v| v.as_array())
                                                .map(|arr| {
                                                    arr.iter()
                                                        .filter_map(|v| v.as_str())
                                                        .map(|value| value.to_string())
                                                        .collect::<Vec<_>>()
                                                })
                                                .unwrap_or_default(),
                                        },
                                    )
                                    .await;
                                }
                                if let Some(ref tx) = stream_tx {
                                    let _ = tx.try_send(StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: msg.clone(),
                                    });
                                }
                                results.push(msg);
                                continue;
                            }
                            if parsed.get("url").is_some() || parsed.get("app_id").is_some() {
                                let title = parsed
                                    .get("title")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("App")
                                    .to_string();
                                let app_type = parsed
                                    .get("type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("static")
                                    .to_string();
                                let app_id_raw = parsed
                                    .get("app_id")
                                    .and_then(|v| v.as_str())
                                    .map(|v| v.trim())
                                    .unwrap_or("");
                                let app_id = if app_id_raw.is_empty() {
                                    "app".to_string()
                                } else {
                                    app_id_raw.to_string()
                                };
                                let access_key = parsed
                                    .get("access_key")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let access_guard_enabled = parsed
                                    .get("access_guard_enabled")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(!access_key.is_empty());
                                let canonical_relative_url = parsed
                                    .get("url")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| {
                                        if !app_id_raw.is_empty() {
                                            format!("/apps/{}/", app_id_raw)
                                        } else {
                                            "/apps/".to_string()
                                        }
                                    });
                                let mut url_with_key = parsed
                                    .get("access_url")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| canonical_relative_url.clone());
                                if access_guard_enabled
                                    && !access_key.is_empty()
                                    && !url_with_key.contains("key=")
                                {
                                    let separator =
                                        if url_with_key.contains('?') { '&' } else { '?' };
                                    url_with_key.push(separator);
                                    url_with_key.push_str("key=");
                                    url_with_key.push_str(&access_key);
                                }
                                let mut public_base_for_app = if expose_public_requested {
                                    self.ensure_public_tunnel_base_url(
                                        Some(&app_id),
                                        stream_tx.as_ref(),
                                    )
                                    .await
                                    .or_else(|| public_base_url.clone())
                                } else {
                                    public_base_url.clone()
                                };

                                let (preview_url, verified, verify_attempts, verify_detail) = self
                                    .validate_and_capture_app_preview(
                                        &url_with_key,
                                        &app_id,
                                        stream_tx.as_ref(),
                                    )
                                    .await
                                    .unwrap_or_else(|e| {
                                        (None, false, 0, format!("Validation helper error: {}", e))
                                    });

                                // App self-heal retries intentionally disabled by user request.
                                let had_public_base = public_base_for_app.is_some();
                                if expose_public_requested && public_base_for_app.is_none() {
                                    // Tunnel URL can appear shortly after initial startup polling.
                                    // Re-run discovery here so the final chat reply includes the
                                    // public link whenever it is already available.
                                    public_base_for_app = self
                                        .ensure_public_tunnel_base_url(Some(&app_id), None)
                                        .await
                                        .or_else(|| public_base_url.clone());
                                }
                                if expose_public_requested && !had_public_base {
                                    if let (Some(public_base), Some(tx)) =
                                        (public_base_for_app.as_deref(), stream_tx.as_ref())
                                    {
                                        let public_open_url = Self::absolutize_public_url(
                                            Some(public_base),
                                            &canonical_relative_url,
                                        );
                                        let _ = tx.try_send(StreamEvent::ToolResult {
                                            name: call.name.clone(),
                                            content: format!(
                                                "Public URL ready: {}",
                                                public_open_url
                                            ),
                                        });
                                    }
                                }
                                let local_base_url = Self::user_facing_local_base_url();
                                let local_open_url = Self::absolutize_public_url(
                                    Some(local_base_url.as_str()),
                                    &canonical_relative_url,
                                );
                                if let Some(cid) = call
                                    .arguments
                                    .get("conversation_id")
                                    .and_then(|v| v.as_str())
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                                {
                                    let canonical_url = if app_id.is_empty() {
                                        "/apps/".to_string()
                                    } else {
                                        format!("/apps/{}/", app_id)
                                    };
                                    self.persist_last_deployed_app_context(
                                        cid,
                                        &app_id,
                                        &title,
                                        &canonical_url,
                                    )
                                    .await;
                                }
                                if let Some(ref tx) = stream_tx {
                                    let _ = tx.try_send(StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: if verified {
                                            format!(
                                                "App deployed + validated: {} ({}) [{} attempt{}]",
                                                title,
                                                app_type,
                                                verify_attempts,
                                                if verify_attempts == 1 { "" } else { "s" }
                                            )
                                        } else {
                                            format!(
                                                "App deployed, validation incomplete: {} ({}) - {}",
                                                title, app_type, verify_detail
                                            )
                                        },
                                    });
                                }

                                let mut app_message_lines: Vec<String> = Vec::new();
                                if verified {
                                    app_message_lines.push(format!(
                                        "I have deployed **{}** ({} app), and I validated that it is running.",
                                        title, app_type
                                    ));
                                } else {
                                    app_message_lines.push(format!(
                                        "I have deployed **{}** ({} app), but validation has not passed yet.",
                                        title, app_type
                                    ));
                                }

                                if verified {
                                    app_message_lines.push(format!(
                                        "- Local: [Open local app]({})",
                                        local_open_url
                                    ));
                                } else {
                                    app_message_lines.push(format!(
                                        "- Local (unverified): [Open local app]({})",
                                        local_open_url
                                    ));
                                }
                                if let Some(public_base) = public_base_for_app.as_deref() {
                                    let public_open_url = Self::absolutize_public_url(
                                        Some(public_base),
                                        &canonical_relative_url,
                                    );
                                    if public_open_url != local_open_url {
                                        if verified {
                                            app_message_lines.push(format!(
                                                "- Public: [Open public app]({})",
                                                public_open_url
                                            ));
                                        } else {
                                            app_message_lines.push(format!(
                                                "- Public (unverified): [Open public app]({})",
                                                public_open_url
                                            ));
                                        }
                                    }
                                } else if expose_public_requested {
                                    app_message_lines.push(
                                        "- Public: pending tunnel readiness for this app."
                                            .to_string(),
                                    );
                                }

                                if access_guard_enabled {
                                    app_message_lines.push("- Access guard: enabled.".to_string());
                                    if !access_key.trim().is_empty() {
                                        app_message_lines
                                            .push(format!("- Access key: `{}`", access_key.trim()));
                                        app_message_lines.push(
                                            "- Open the link above and enter the access key if prompted."
                                                .to_string(),
                                        );
                                    }
                                } else {
                                    app_message_lines
                                        .push("- Access guard: not enabled.".to_string());
                                }

                                app_message_lines.push(format!(
                                    "- Webpage status: {}",
                                    if verified {
                                        "reachable and validated."
                                    } else {
                                        "deployed, but validation has not passed yet."
                                    }
                                ));
                                app_message_lines.push(format!(
                                    "- Deployment validation: {} (attempts: {}).",
                                    if verified { "passed" } else { "failed" },
                                    verify_attempts
                                ));
                                if !verified && !verify_detail.trim().is_empty() {
                                    app_message_lines.push(format!(
                                        "- Validation issue: {}",
                                        verify_detail.trim()
                                    ));
                                }

                                if let Some(preview) = preview_url {
                                    app_message_lines.push(format!("![App Preview]({})", preview));
                                }
                                let app_message = app_message_lines.join("\n");
                                results.push(app_message);
                                continue;
                            }
                        }
                        if let Some(ref tx) = stream_tx {
                            let _ = tx.try_send(StreamEvent::ToolResult {
                                name: call.name.clone(),
                                content: sanitize_stream(&result),
                            });
                        }
                        results.push(result);
                    }
                    Err(e) => {
                        tracing::error!("App deployment error: {}", e);
                        self.fire_action_hook(
                            crate::hooks::HookTrigger::OnError,
                            request_channel,
                            &call.name,
                            hook_hint.as_deref(),
                            Some(&e.to_string()),
                            &hook_event_id,
                        )
                        .await;
                        let error_text = e.to_string();
                        let formatted = if error_text.contains("Missing 'files'")
                            || error_text.contains("provide an object mapping filename to content")
                        {
                            "Error deploying app: app_deploy was called without a valid `files` object. This is a malformed tool payload, not your app code. Retrying the same request should regenerate a valid deploy payload.".to_string()
                        } else {
                            format!("Error deploying app: {}", error_text)
                        };
                        if let Some(ref tx) = stream_tx {
                            let _ = tx.try_send(StreamEvent::ToolResult {
                                name: call.name.clone(),
                                content: formatted.clone(),
                            });
                        }
                        results.push(formatted);
                    }
                }
                continue;
            }

            // Execute in sandbox (runtime will resolve secret placeholders at execution time)
            let call_message_hint = action_message_hint(&call.arguments);
            match self
                .execute_action_with_hooks(
                    &call.name,
                    &call.arguments,
                    request_channel,
                    call_message_hint.as_deref(),
                )
                .await
            {
                Ok(result) => {
                    let mut result = result;
                    if call.name.starts_with("mcp_") {
                        result = self.sanitize_mcp_output(&result);
                    }
                    // Special handling for schedule_task - actually create the task
                    if call.name == "schedule_task" && result.starts_with("Task scheduled:") {
                        if let Some(schedule_result) = self
                            .handle_schedule_task(&call.arguments, request_channel, None, None)
                            .await
                        {
                            if let Some(ref tx) = stream_tx {
                                let _ = tx.try_send(StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: sanitize_stream(&schedule_result),
                                });
                            }
                            results.push(schedule_result);
                            continue;
                        }
                    }

                    // Special handling for watch - spawn background watcher
                    if call.name == "watch" && result.starts_with("Watch created:") {
                        if let Some(watch_result) = self
                            .handle_watch(&call.arguments, request_channel, None, None)
                            .await
                        {
                            if let Some(ref tx) = stream_tx {
                                let _ = tx.try_send(StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: sanitize_stream(&watch_result),
                                });
                            }
                            results.push(watch_result);
                            continue;
                        }
                    }

                    // Format code_execute results with self-heal retry on errors
                    if call.name == "code_execute" {
                        let language = call
                            .arguments
                            .get("language")
                            .and_then(|v| v.as_str())
                            .unwrap_or("code")
                            .to_string();
                        let mut current_code = call
                            .arguments
                            .get("code")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let mut current_result = result.clone();
                        let mut current_args = call.arguments.clone();

                        // Self-heal loop: retry on execution errors
                        const MAX_SAME_ERROR_RETRIES: usize = 2;
                        const MAX_TOTAL_RETRIES: usize = 3;
                        let mut total_retries = 0usize;
                        let mut last_error_sig = String::new();
                        let mut same_error_count = 0usize;
                        let mut self_heal_stop_reason: Option<String> = None;
                        let mut self_heal_error_signatures: Vec<String> = Vec::new();
                        let code_signature = |code: &str| -> String {
                            let mut normalized = code
                                .lines()
                                .map(|line| line.trim())
                                .filter(|line| !line.is_empty())
                                .collect::<Vec<_>>()
                                .join("\n");
                            if normalized.len() > 4096 {
                                normalized.truncate(4096);
                            }
                            normalized
                        };
                        let mut seen_code_signatures: HashSet<String> = HashSet::new();
                        let initial_sig = code_signature(&current_code);
                        if !initial_sig.is_empty() {
                            seen_code_signatures.insert(initial_sig);
                        }

                        loop {
                            let parsed = match serde_json::from_str::<serde_json::Value>(
                                &current_result,
                            ) {
                                Ok(parsed) => parsed,
                                Err(_) => {
                                    if total_retries > 0 {
                                        self_heal_stop_reason = Some(
                                            "runtime response was not structured JSON; stopped auto-fix"
                                                .to_string(),
                                        );
                                    }
                                    break;
                                }
                            };
                            let exit_code = parsed
                                .get("exit_code")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0);
                            let should_retry = exit_code != 0 && !current_code.trim().is_empty();

                            if !should_retry {
                                break;
                            }

                            if total_retries >= MAX_TOTAL_RETRIES {
                                self_heal_stop_reason =
                                    Some(format!("maximum attempts reached ({MAX_TOTAL_RETRIES})"));
                                break;
                            }

                            let error_text =
                                parsed.get("error").and_then(|v| v.as_str()).unwrap_or("");
                            let output_text =
                                parsed.get("output").and_then(|v| v.as_str()).unwrap_or("");

                            // Bail immediately on sandbox-environment errors that retries cannot fix
                            let combined_for_check = format!("{}\n{}", error_text, output_text);
                            let is_sandbox_unreachable = combined_for_check
                                .contains("No such file or directory")
                                && (combined_for_check.contains("/app/data/")
                                    || combined_for_check.contains("os.chdir"));
                            if is_sandbox_unreachable {
                                self_heal_stop_reason = Some(
                                    "sandbox cannot access app data paths — use file_write/file_read tools instead".to_string(),
                                );
                                break;
                            }

                            // Build error signature for same-error detection
                            let error_combined = format!("{}\n{}", error_text, output_text);
                            let error_sig = error_combined
                                .lines()
                                .take(5)
                                .collect::<Vec<_>>()
                                .join("\n");
                            if !error_sig.is_empty()
                                && !self_heal_error_signatures.iter().any(|s| s == &error_sig)
                                && self_heal_error_signatures.len() < 4
                            {
                                self_heal_error_signatures.push(error_sig.clone());
                            }

                            if error_sig == last_error_sig {
                                same_error_count += 1;
                                if same_error_count >= MAX_SAME_ERROR_RETRIES {
                                    tracing::warn!(
                                        "Self-heal: same error repeated {} times, giving up",
                                        same_error_count
                                    );
                                    self_heal_stop_reason = Some(format!(
                                        "same failure repeated {} times",
                                        same_error_count
                                    ));
                                    break;
                                }
                            } else {
                                same_error_count = 1;
                                last_error_sig = error_sig;
                            }

                            total_retries += 1;
                            tracing::info!("Self-heal: code execution failed (attempt {}/{}), asking LLM to fix", total_retries, MAX_TOTAL_RETRIES);

                            // Emit trace step
                            {
                                let mut trace = trace_ref.write().await;
                                trace.steps.push(ExecutionStep {
                                    icon: "[fix]".to_string(),
                                    title: format!(
                                        "Self-Heal: Fixing Code (attempt {})",
                                        total_retries
                                    ),
                                    detail: format!(
                                        "Error: {}",
                                        error_text.chars().take(100).collect::<String>()
                                    ),
                                    step_type: "thinking".to_string(),
                                    data: None,
                                    timestamp: chrono::Utc::now(),
                                    duration_ms: None,
                                });
                            }

                            // Ask LLM to fix the code
                            let fix_prompt = format!(
                                "The following {} code failed to execute. Fix the code and return ONLY the corrected code, no explanation.\n\n\
                                Code:\n```{}\n{}\n```\n\n\
                                Error output:\n```\n{}\n{}\n```\n\n\
                                Return only the fixed code, nothing else.",
                                language, language, current_code.trim(), error_text, output_text
                            );

                            let empty_actions: Vec<crate::actions::ActionDef> = Vec::new();
                            match self.llm.chat(
                                "You are a code fixer. Return ONLY the corrected code. No markdown fences, no explanations.",
                                &fix_prompt,
                                &[],
                                &empty_actions,
                            ).await {
                                Ok(fix_response) => {
                                    self.record_llm_usage(request_channel, "self_heal", &fix_response).await;
                                    // Extract code from response (strip markdown fences if present)
                                    let fixed = fix_response.content.trim().to_string();
                                    let fixed = if fixed.starts_with("```") {
                                        // Strip opening ```lang and closing ```
                                        let lines: Vec<&str> = fixed.lines().collect();
                                        let start = if lines.first().is_some_and(|l| l.starts_with("```")) { 1 } else { 0 };
                                        let end = if lines.last().is_some_and(|l| l.trim() == "```") { lines.len() - 1 } else { lines.len() };
                                        lines[start..end].join("\n")
                                    } else {
                                        fixed
                                    };
                                    let fixed_sig = code_signature(&fixed);
                                    let current_sig = code_signature(&current_code);
                                    if fixed_sig.is_empty() {
                                        tracing::warn!(
                                            "Self-heal: LLM returned empty code, giving up"
                                        );
                                        self_heal_stop_reason =
                                            Some("LLM returned empty patch".to_string());
                                        break;
                                    }
                                    if fixed_sig == current_sig {
                                        tracing::warn!("Self-heal: LLM returned identical code, giving up");
                                        self_heal_stop_reason =
                                            Some("LLM returned no meaningful code change".to_string());
                                        break;
                                    }
                                    if seen_code_signatures.contains(&fixed_sig) {
                                        tracing::warn!(
                                            "Self-heal: repeated patch detected, giving up"
                                        );
                                        self_heal_stop_reason = Some(
                                            "repeated patch detected (loop prevention)".to_string(),
                                        );
                                        break;
                                    }
                                    seen_code_signatures.insert(fixed_sig);

                                    current_code = fixed.clone();
                                    current_args["code"] = serde_json::Value::String(fixed);

                                    // Re-execute with fixed code
                                    let retry_hint = action_message_hint(&current_args);
                                    match self
                                        .execute_action_with_hooks(
                                            "code_execute",
                                            &current_args,
                                            request_channel,
                                            retry_hint.as_deref(),
                                        )
                                        .await
                                    {
                                        Ok(new_result) => {
                                            current_result = new_result;
                                        }
                                        Err(e) => {
                                            tracing::error!("Self-heal re-execution error: {}", e);
                                            self_heal_stop_reason = Some(format!(
                                                "re-execution failed: {}",
                                                safe_truncate(&e.to_string(), 180)
                                            ));
                                            break;
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("Self-heal LLM call failed: {}", e);
                                    self_heal_stop_reason = Some(format!(
                                        "LLM fixer failed: {}",
                                        safe_truncate(&e.to_string(), 180)
                                    ));
                                    break;
                                }
                            }
                        }

                        // Format the final result (after retries or on first success)
                        let formatted = if let Ok(parsed) =
                            serde_json::from_str::<serde_json::Value>(&current_result)
                        {
                            let output =
                                parsed.get("output").and_then(|v| v.as_str()).unwrap_or("");
                            let error = parsed.get("error").and_then(|v| v.as_str());
                            let exit_code = parsed
                                .get("exit_code")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(-1);
                            let files = parsed.get("files").and_then(|v| v.as_array());

                            let mut parts = Vec::new();

                            if total_retries > 0 {
                                let status = if exit_code == 0 {
                                    "fixed"
                                } else {
                                    "still failing"
                                };
                                parts.push(format!(
                                    "*Self-healed after {} attempt{} ({})*",
                                    total_retries,
                                    if total_retries == 1 { "" } else { "s" },
                                    status
                                ));
                                if exit_code != 0 {
                                    if let Some(reason) = &self_heal_stop_reason {
                                        parts.push(format!("**Self-heal stopped:** {}", reason));
                                    }
                                    if !self_heal_error_signatures.is_empty() {
                                        let signatures = self_heal_error_signatures
                                            .iter()
                                            .map(|s| format!("- `{}`", safe_truncate(s, 220)))
                                            .collect::<Vec<_>>()
                                            .join("\n");
                                        parts.push(format!(
                                            "**Observed failure signatures:**\n{}",
                                            signatures
                                        ));
                                    }
                                }
                            }

                            // Show the code with download link if available
                            if let Some(file_list) = &files {
                                let code_file = file_list
                                    .iter()
                                    .filter_map(|f| f.as_str())
                                    .find(|f| f.contains("code."));
                                if let Some(cf) = code_file {
                                    parts.push(format!(
                                        "```{}\n{}\n```\n[Download code]({})",
                                        language,
                                        current_code.trim(),
                                        cf
                                    ));
                                } else {
                                    parts.push(format!(
                                        "```{}\n{}\n```",
                                        language,
                                        current_code.trim()
                                    ));
                                }
                            } else {
                                parts.push(format!(
                                    "```{}\n{}\n```",
                                    language,
                                    current_code.trim()
                                ));
                            }

                            if !output.is_empty() {
                                parts.push(format!("**Output:**\n```\n{}\n```", output.trim()));
                            }

                            if let Some(err) = error {
                                if !err.is_empty() {
                                    parts.push(format!("**Errors:**\n```\n{}\n```", err.trim()));
                                }
                            }

                            if exit_code != 0 {
                                parts.push(format!("Exit code: {}", exit_code));
                            }

                            if let Some(file_list) = files {
                                let output_files: Vec<&str> = file_list
                                    .iter()
                                    .filter_map(|f| f.as_str())
                                    .filter(|f| !f.contains("code."))
                                    .collect();
                                if !output_files.is_empty() {
                                    let mut file_parts = Vec::new();
                                    for file_path in &output_files {
                                        let filename =
                                            file_path.rsplit('/').next().unwrap_or(file_path);
                                        let ext = filename
                                            .rsplit('.')
                                            .next()
                                            .unwrap_or("")
                                            .to_lowercase();
                                        let image_exts =
                                            ["png", "jpg", "jpeg", "gif", "svg", "webp", "bmp"];
                                        if image_exts.contains(&ext.as_str()) {
                                            file_parts
                                                .push(format!("![{}]({})", filename, file_path));
                                        } else {
                                            file_parts.push(format!(
                                                "[Download {}]({})",
                                                filename, file_path
                                            ));
                                        }
                                    }
                                    parts.push(format!(
                                        "**Generated Files:**\n{}",
                                        file_parts.join("\n")
                                    ));
                                }
                            }

                            parts.join("\n\n")
                        } else {
                            let mut prefix = String::new();
                            if total_retries > 0 {
                                let mut line = format!(
                                    "*Self-healed after {} attempt{} (still failing)*",
                                    total_retries,
                                    if total_retries == 1 { "" } else { "s" }
                                );
                                if let Some(reason) = &self_heal_stop_reason {
                                    line.push_str(&format!("\n**Self-heal stopped:** {}", reason));
                                }
                                prefix.push_str(&line);
                                prefix.push_str("\n\n");
                            }
                            format!(
                                "{}```{}\n{}\n```\n\n{}",
                                prefix,
                                language,
                                current_code.trim(),
                                current_result
                            )
                        };

                        results.push(formatted);
                        continue;
                    }

                    // Format video_generate results with inline player + download
                    if call.name == "video_generate" {
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&result) {
                            if let Some(url) = parsed.get("url").and_then(|v| v.as_str()) {
                                let duration = parsed
                                    .get("duration_seconds")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let resolution = parsed
                                    .get("resolution")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let size = parsed
                                    .get("file_size_bytes")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let output_id = parsed
                                    .get("output_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let filename = parsed
                                    .get("filename")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let size_mb = size as f64 / 1_048_576.0;
                                let rendered_url = absolutize_url(url);
                                let mut delivered_to_channel = false;
                                let caption = format!(
                                    "{}s video, {}, {:.1}MB",
                                    duration, resolution, size_mb
                                );

                                if matches!(request_channel, "telegram" | "whatsapp")
                                    && !output_id.is_empty()
                                    && !filename.is_empty()
                                {
                                    let output_path = self
                                        .data_dir
                                        .join("outputs")
                                        .join(output_id)
                                        .join(filename);
                                    match tokio::fs::read(&output_path).await {
                                        Ok(video_bytes) => {
                                            match crate::channels::send_video_to_channel(
                                                self,
                                                request_channel,
                                                &video_bytes,
                                                &caption,
                                                Some(&rendered_url),
                                            )
                                            .await
                                            {
                                                Ok(_) => {
                                                    delivered_to_channel = true;
                                                }
                                                Err(e) => {
                                                    tracing::warn!(
                                                        "Failed to send generated video to {}: {}",
                                                        request_channel,
                                                        e
                                                    );
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                "Failed reading rendered video file {}: {}",
                                                output_path.display(),
                                                e
                                            );
                                        }
                                    }
                                }

                                let formatted = if delivered_to_channel {
                                    format!("Video sent to this chat.\nDownload: {}", rendered_url)
                                } else if matches!(request_channel, "telegram" | "whatsapp") {
                                    format!(
                                        "Video generated ({}s, {:.1}MB): {}",
                                        duration, size_mb, rendered_url
                                    )
                                } else {
                                    format!(
                                        "[VIDEO_RESULT]{}\n[/VIDEO_RESULT]\n*{}s video, {}, {:.1}MB*",
                                        rendered_url, duration, resolution, size_mb
                                    )
                                };
                                if let Some(ref tx) = stream_tx {
                                    let _ = tx.try_send(StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: format!(
                                            "Video generated ({}s, {:.1}MB)",
                                            duration, size_mb
                                        ),
                                    });
                                }
                                results.push(formatted);
                                continue;
                            }
                        }
                        if let Some(ref tx) = stream_tx {
                            let _ = tx.try_send(StreamEvent::ToolResult {
                                name: call.name.clone(),
                                content: sanitize_stream(&result),
                            });
                        }
                        results.push(result);
                        continue;
                    }

                    // Format gmail_scan results with LLM classification + summary
                    if call.name == "gmail_scan" {
                        let email_format_hint = {
                            let profile = self.user_profile.read().await;
                            profile.email_format.clone().unwrap_or_default()
                        };
                        let format_extra = if email_format_hint.is_empty() {
                            String::new()
                        } else {
                            format!("\nUser preference: {}", email_format_hint)
                        };

                        let format_prompt = format!(
                            "Here are raw email results from Gmail. Classify, summarize, and format them.\n\
                            Rules:\n\
                            - Group into categories with **bold** headers: Action Needed, Security Alerts, Receipts & Orders, Newsletters & Promotions, Other\n\
                            - Skip empty categories\n\
                            - For each email: show sender name (not full email address), subject, and a brief one-line summary/gist\n\
                            - Flag anything time-sensitive or requiring action\n\
                            - Use markdown: **bold** for headers, bullet points for items\n\
                            - Be concise - no raw headers, no IDs, no label dumps\n\
                            {}\n\n\
                            Raw email data:\n{}",
                            format_extra, result
                        );

                        let empty_actions: Vec<crate::actions::ActionDef> = Vec::new();
                        match self.llm.chat(
                            "You are a concise email assistant. Format email summaries with clear categorization. Use markdown.",
                            &format_prompt,
                            &[],
                            &empty_actions,
                        ).await {
                            Ok(formatted) => {
                                self.record_llm_usage(request_channel, "gmail_format", &formatted).await;
                                if let Some(ref tx) = stream_tx {
                                    let _ = tx.try_send(StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: "Gmail scan summarized".to_string(),
                                    });
                                }
                                results.push(formatted.content);
                            }
                            Err(e) => {
                                tracing::warn!("Gmail format LLM pass failed, using raw: {}", e);
                                if let Some(ref tx) = stream_tx {
                                    let _ = tx.try_send(StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: "Gmail scan returned raw results".to_string(),
                                    });
                                }
                                results.push(result);
                            }
                        }
                        continue;
                    }

                    if let Some(payload) = parse_workflow_missing_inputs_marker(&result) {
                        if !Self::sensitive_like_input_keys(&payload.missing).is_empty() {
                            if let Some(cid) =
                                conversation_id.filter(|value| !value.trim().is_empty())
                            {
                                self.remember_pending_secret_followup(
                                    cid,
                                    PendingSecretFollowupKind::RetryWorkflow {
                                        payload: payload.clone(),
                                    },
                                )
                                .await;
                            }
                        }
                        let prompt = Self::format_missing_inputs_prompt(&payload);
                        if let Some(ref tx) = stream_tx {
                            let _ = tx.try_send(StreamEvent::ToolResult {
                                name: call.name.clone(),
                                content: prompt.clone(),
                            });
                        }
                        results.push(prompt);
                        continue;
                    }

                    // Check if this is a workflow action that needs LLM orchestration
                    if let Some((action_name, user_query)) = parse_workflow_action_marker(&result) {
                        match self
                            .execute_workflow_marker_action(&action_name, &user_query)
                            .await
                        {
                            Ok(llm_result) => {
                                if let Some(ref tx) = stream_tx {
                                    let _ = tx.try_send(StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: format!("Workflow '{}' completed", action_name),
                                    });
                                }
                                results.push(llm_result);
                            }
                            Err(e) => {
                                tracing::error!("Workflow action execution error: {}", e);
                                let formatted =
                                    format!("Error executing workflow '{}': {}", action_name, e);
                                if let Some(ref tx) = stream_tx {
                                    let _ = tx.try_send(StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: formatted.clone(),
                                    });
                                }
                                results.push(formatted);
                            }
                        }
                        continue;
                    }

                    if let Some(ref tx) = stream_tx {
                        let _ = tx.try_send(StreamEvent::ToolResult {
                            name: call.name.clone(),
                            content: sanitize_stream(&result),
                        });
                    }
                    results.push(result);
                }
                Err(e) => {
                    tracing::error!("Action execution error: {}", e);
                    if call.name == "browse" {
                        let target = call
                            .arguments
                            .get("url")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim();
                        let query_hint = if target.is_empty() {
                            call.arguments
                                .get("query")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .trim()
                        } else {
                            target
                        };

                        if !query_hint.is_empty() {
                            let fallback_args = serde_json::json!({
                                "query": query_hint,
                                "num_results": 5
                            });
                            match self
                                .runtime
                                .execute_action("web_search", &fallback_args)
                                .await
                            {
                                Ok(search_out) => {
                                    let healed = format!(
                                        "Browse failed ({})\n\nSelf-heal fallback: searched the web instead.\n{}",
                                        e, search_out
                                    );
                                    if let Some(ref tx) = stream_tx {
                                        let _ = tx.try_send(StreamEvent::ToolResult {
                                            name: call.name.clone(),
                                            content: "Browse failed; used search fallback"
                                                .to_string(),
                                        });
                                    }
                                    results.push(healed);
                                    continue;
                                }
                                Err(search_err) => {
                                    tracing::warn!(
                                        "Browse self-heal fallback failed for '{}': {}",
                                        query_hint,
                                        search_err
                                    );
                                }
                            }
                        }
                    }
                    let formatted = format!("Error executing '{}': {}", call.name, e);
                    if let Some(ref tx) = stream_tx {
                        let _ = tx.try_send(StreamEvent::ToolResult {
                            name: call.name.clone(),
                            content: formatted.clone(),
                        });
                    }
                    results.push(formatted);
                }
            }
        }

        // If there's content plus tool results, combine them
        if response.content.is_empty() {
            Ok(results.join("\n"))
        } else {
            Ok(format!("{}\n\n{}", response.content, results.join("\n")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(id: &str, name: &str, arguments: serde_json::Value) -> crate::core::llm::ToolCall {
        crate::core::llm::ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments,
        }
    }

    #[test]
    fn tool_call_signature_ignores_object_key_order() {
        let a = call(
            "1",
            "app_deploy",
            json!({
                "files": {"index.html": "<h1>ok</h1>"},
                "title": "demo",
                "config": {"a": 1, "b": 2}
            }),
        );
        let b = call(
            "2",
            "app_deploy",
            json!({
                "config": {"b": 2, "a": 1},
                "title": "demo",
                "files": {"index.html": "<h1>ok</h1>"}
            }),
        );

        assert_eq!(
            Agent::tool_call_signature(&a),
            Agent::tool_call_signature(&b)
        );
    }

    #[test]
    fn tool_call_signature_preserves_array_order() {
        let a = call("1", "code_execute", json!({ "args": [1, 2, 3] }));
        let b = call("2", "code_execute", json!({ "args": [3, 2, 1] }));

        assert_ne!(
            Agent::tool_call_signature(&a),
            Agent::tool_call_signature(&b)
        );
    }

    #[test]
    fn action_has_dangerous_capabilities_uses_permission_metadata() {
        let read_only = crate::actions::ActionDef {
            name: "app_inspect".to_string(),
            description: "Inspect deployed apps and return status.".to_string(),
            capabilities: vec![],
            ..Default::default()
        };
        let mutating = crate::actions::ActionDef {
            name: "schedule_task".to_string(),
            description: "Schedule a recurring task to run automatically.".to_string(),
            capabilities: vec!["scheduler".to_string()],
            ..Default::default()
        };

        assert!(!action_has_dangerous_capabilities(Some(&read_only)));
        assert!(action_has_dangerous_capabilities(Some(&mutating)));
    }

    #[test]
    fn normalize_app_deploy_arguments_unwraps_double_encoded_payload() {
        let payload = "\"{\\\"title\\\":\\\"Demo\\\",\\\"files\\\":{\\\"index.html\\\":\\\"<h1>ok</h1>\\\"}}\"";
        let input = json!({
            "name": "app_deploy",
            "payload": payload,
            "runtime_preference": "local"
        });

        let normalized = Agent::normalize_app_deploy_arguments(&input);
        let files = normalized
            .get("files")
            .and_then(|v| v.as_object())
            .expect("files object should be recovered");
        assert_eq!(
            files.get("index.html").and_then(|v| v.as_str()),
            Some("<h1>ok</h1>")
        );
        assert_eq!(
            normalized
                .get("runtime_preference")
                .and_then(|v| v.as_str()),
            Some("local")
        );
    }

    #[test]
    fn normalize_app_deploy_arguments_converts_file_array_to_files_map() {
        let input = json!({
            "payload": {
                "title": "Demo",
                "project_files": [
                    { "name": "index.html", "content": "<h1>x</h1>" },
                    { "name": "app.js", "content": "console.log('ok')" }
                ]
            }
        });

        let normalized = Agent::normalize_app_deploy_arguments(&input);
        let files = normalized
            .get("files")
            .and_then(|v| v.as_object())
            .expect("files map should be built from project_files");
        assert_eq!(files.len(), 2);
        assert_eq!(
            files.get("index.html").and_then(|v| v.as_str()),
            Some("<h1>x</h1>")
        );
        assert_eq!(
            files.get("app.js").and_then(|v| v.as_str()),
            Some("console.log('ok')")
        );
    }

    #[test]
    fn resolve_duplicate_app_reuses_only_exact_files_with_live_runtime() {
        assert_eq!(
            Agent::resolve_duplicate_app("exact_files", true),
            DuplicateAppResolution::ReuseExisting
        );
        assert_eq!(
            Agent::resolve_duplicate_app("exact_files", false),
            DuplicateAppResolution::ReplaceExisting
        );
    }

    #[test]
    fn resolve_duplicate_app_replaces_non_exact_matches() {
        assert_eq!(
            Agent::resolve_duplicate_app("exact_title", true),
            DuplicateAppResolution::ReplaceExisting
        );
        assert_eq!(
            Agent::resolve_duplicate_app("fuzzy", true),
            DuplicateAppResolution::ReplaceExisting
        );
    }
}
