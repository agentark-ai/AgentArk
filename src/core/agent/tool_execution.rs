use super::*;

pub(crate) struct ToolExecutionContext<'a> {
    pub request_channel: &'a str,
    pub trace_id: Option<&'a str>,
    pub conversation_id: Option<&'a str>,
    pub strategy_version: Option<&'a str>,
    pub policy_version: Option<&'a str>,
    pub prompt_version: Option<&'a str>,
    pub model_slot: Option<&'a str>,
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

    fn tool_call_signature(call: &crate::core::llm::ToolCall) -> String {
        let canonical_args = Self::canonicalize_json_value(&call.arguments);
        let args = serde_json::to_string(&canonical_args).unwrap_or_else(|_| "{}".to_string());
        format!("{}:{}", call.name, args)
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

            summary.insert("file_count".to_string(), serde_json::json!(total_file_count));
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
                let _ = tx.try_send(StreamEvent::ToolResult {
                    name: "app_deploy".to_string(),
                    content: format!(
                        "Validating deployed app (attempt {}/{})",
                        attempt, MAX_APP_VERIFY_ATTEMPTS
                    ),
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
                                    return Ok((
                                        None,
                                        true,
                                        attempt,
                                        format!(
                                            "HTTP probe passed on attempt {} (status {}, preview unavailable: {})",
                                            attempt, status, e
                                        ),
                                    ));
                                }
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

    fn internal_api_base_url() -> String {
        let bind_addr =
            std::env::var("AGENTARK_BIND").unwrap_or_else(|_| "127.0.0.1:8990".to_string());
        let tls_enabled = std::env::var("AGENTARK_TLS_CERT")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .is_some()
            && std::env::var("AGENTARK_TLS_KEY")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .is_some();
        let scheme = if tls_enabled { "https" } else { "http" };
        format!("{}://{}", scheme, bind_addr)
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
        Ok(reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(std::time::Duration::from_secs(5))
            .build()?)
    }

    async fn ensure_public_tunnel_base_url(
        &self,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Option<String> {
        if let Some(existing) = self.load_public_base_url().await {
            return Some(existing);
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
        match start_req.send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    tracing::debug!("Tunnel start request returned {}", resp.status());
                }
            }
            Err(e) => {
                tracing::debug!("Tunnel start request failed: {}", e);
                return None;
            }
        }

        if let Some(tx) = stream_tx {
            let _ = tx.try_send(StreamEvent::ToolResult {
                name: "app_deploy".to_string(),
                content: "Starting Cloudflare tunnel for public app access...".to_string(),
            });
        }

        for _ in 0..10 {
            let mut status_req = client.get(format!("{}/tunnel/status", base_url));
            if let Some(key) = self.api_key.as_ref().filter(|k| !k.trim().is_empty()) {
                status_req = status_req.bearer_auth(key);
            }
            if let Ok(resp) = status_req.send().await {
                if resp.status().is_success() {
                    if let Ok(payload) = resp.json::<serde_json::Value>().await {
                        if let Some(url) = payload
                            .get("url")
                            .and_then(|v| v.as_str())
                            .map(|v| v.trim().trim_end_matches('/').to_string())
                            .filter(|v| !v.is_empty())
                        {
                            let _ = self.storage.set("public_base_url", url.as_bytes()).await;
                            return Some(url);
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        self.load_public_base_url().await
    }

    fn trigger_arkpulse_refresh(&self, reason: &'static str) {
        let api_key = self.api_key.clone();
        let base_url = Self::internal_api_base_url();
        tokio::spawn(async move {
            let client = match reqwest::Client::builder()
                .danger_accept_invalid_certs(true)
                .timeout(std::time::Duration::from_secs(4))
                .build()
            {
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
        let synthetic = crate::core::llm::LlmResponse {
            content: String::new(),
            tool_calls: vec![call.clone()],
            reasoning: None,
            usage: None,
            provider: "internal".to_string(),
            model: "tool_dispatch".to_string(),
        };
        self.execute_tool_calls_legacy(&synthetic, trace_ref, stream_tx, request_channel)
            .await
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

    pub(crate) async fn handle_app_deploy_tool_call(
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

    pub(crate) async fn handle_runtime_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        trace_ref: &Arc<RwLock<ExecutionTrace>>,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
        request_channel: &str,
        _public_base_url: Option<&str>,
    ) -> Result<String> {
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
                let mut canary_state: Option<crate::core::self_evolve::strategy_runtime::CanaryRolloutState> =
                    None;
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
                        let candidate_version = format!("routing-candidate-{}", result.lineage_entry_id);

                        self.storage
                            .set(
                                crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_POLICY_CANARY_KEY,
                                &candidate_serialized,
                            )
                            .await?;
                        let state = crate::core::self_evolve::strategy_runtime::CanaryRolloutState {
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
                    return Ok(serde_json::json!({
                        "status": "blocked",
                        "mode": "code",
                        "message": "Code evolution is disabled by default. Re-run self_evolve with mode='code' and allow_code_writes=true after policy evolution is stable."
                    })
                    .to_string());
                }

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

                Ok(serde_json::to_string_pretty(&result)?)
            }
            _ => Ok(serde_json::json!({
                "status": "error",
                "message": format!(
                    "Unsupported self_evolve mode '{}'. Use mode='policy' (default) or mode='code'.",
                    mode
                ),
            })
            .to_string()),
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
    ) -> Result<String> {
        if response.tool_calls.is_empty() {
            return Ok(response.content.clone());
        }
        let request_channel = ctx.request_channel;
        let trace_id = ctx.trace_id;
        let conversation_id = ctx.conversation_id;
        let strategy_version = ctx.strategy_version;
        let policy_version = ctx.policy_version;
        let prompt_version = ctx.prompt_version;
        let model_slot = ctx.model_slot;

        let public_base_url = self.load_public_base_url().await;
        let integration_aliases = self.load_tool_integration_aliases().await;
        let handlers = default_tool_handlers();

        let mut seen_signatures: HashSet<String> = HashSet::new();
        let mut unique_calls: Vec<&crate::core::llm::ToolCall> = Vec::new();
        for call in &response.tool_calls {
            let sig = Self::tool_call_signature(call);
            if seen_signatures.insert(sig) {
                unique_calls.push(call);
            }
        }

        let mut results = Vec::new();
        for call in unique_calls {
            let call_started = std::time::Instant::now();
            let ctx = ToolHandlerContext {
                trace_ref,
                stream_tx: stream_tx.as_ref(),
                request_channel,
                public_base_url: public_base_url.as_deref(),
                integration_aliases: &integration_aliases,
            };

            let mut handled = false;
            for handler in &handlers {
                if !handler.can_handle(self, call, &ctx) {
                    continue;
                }
                tracing::debug!("Tool '{}' handled by '{}'", call.name, handler.id());
                match handler.handle(self, call, &ctx).await {
                    Ok(Some(output)) => {
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
                            latency_ms: Some(call_started.elapsed().as_millis() as u64),
                            arguments: Some(&call.arguments),
                            payload: Some(&payload),
                            strategy_version,
                            policy_version,
                            prompt_version,
                            model_slot,
                        })
                        .await;
                        results.push(output);
                        handled = true;
                        break;
                    }
                    Ok(None) => continue,
                    Err(e) => {
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
                            latency_ms: Some(call_started.elapsed().as_millis() as u64),
                            arguments: Some(&call.arguments),
                            payload: Some(&payload),
                            strategy_version,
                            policy_version,
                            prompt_version,
                            model_slot,
                        })
                        .await;
                        return Err(e);
                    }
                }
            }

            if !handled {
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
                results.push(msg);
            }
        }

        if response.content.is_empty() {
            Ok(results.join("\n"))
        } else {
            Ok(format!("{}\n\n{}", response.content, results.join("\n")))
        }
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
            let allowed = if self.should_auto_approve_action(&call.name) {
                tracing::info!(
                    "Auto-approving command-like action '{}' for AgentArk",
                    call.name
                );
                true
            } else {
                self.safety.is_allowed(&call.name, &call.arguments).await?
            };
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
                    let notify_fn: std::sync::Arc<dyn Fn(String, Option<Vec<u8>>) + Send + Sync> =
                        std::sync::Arc::new(move |msg: String, screenshot: Option<Vec<u8>>| {
                            let config = agent_config.clone();
                            let channel = notify_channel.clone();
                            let chat_id = chat_id.clone();
                            let storage = storage_clone.clone();
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
                let llm_env = self
                    .model_pool
                    .get(&self.primary_model_id)
                    .map(|(slot, _)| slot.provider.app_env_vars())
                    .filter(|env| {
                        env.iter().any(|(k, v)| {
                            if v.trim().is_empty() || v == "[ENCRYPTED]" {
                                return false;
                            }
                            k.ends_with("_API_KEY")
                                || (k == "LLM_PROVIDER" && v.eq_ignore_ascii_case("ollama"))
                        })
                    })
                    .unwrap_or_else(|| self.config.llm.app_env_vars());
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
                                    match self.ensure_public_tunnel_base_url(stream_tx.as_ref()).await {
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
                                    .unwrap_or("App");
                                let app_type = parsed
                                    .get("type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("static");
                                let app_id_raw = parsed
                                    .get("app_id")
                                    .and_then(|v| v.as_str())
                                    .map(|v| v.trim())
                                    .unwrap_or("");
                                let app_id = if app_id_raw.is_empty() {
                                    "app"
                                } else {
                                    app_id_raw
                                };
                                let access_key = parsed
                                    .get("access_key")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let access_guard_enabled = parsed
                                    .get("access_guard_enabled")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(!access_key.is_empty());
                                let mut url_with_key = parsed
                                    .get("access_url")
                                    .and_then(|v| v.as_str())
                                    .or_else(|| parsed.get("url").and_then(|v| v.as_str()))
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| {
                                        if !app_id_raw.is_empty() {
                                            format!("/apps/{}/", app_id_raw)
                                        } else {
                                            "/apps/".to_string()
                                        }
                                    });
                                if access_guard_enabled
                                    && !access_key.is_empty()
                                    && !url_with_key.contains("key=")
                                {
                                    let separator =
                                        if url_with_key.contains('?') { '&' } else { '?' };
                                    url_with_key.push(separator);
                                    url_with_key.push_str("key=");
                                    url_with_key.push_str(access_key);
                                }
                                let mut public_base_for_app = if expose_public_requested {
                                    self.ensure_public_tunnel_base_url(stream_tx.as_ref())
                                        .await
                                        .or_else(|| public_base_url.clone())
                                } else {
                                    public_base_url.clone()
                                };

                                let (preview_url, verified, verify_attempts, verify_detail) = self
                                    .validate_and_capture_app_preview(
                                        &url_with_key,
                                        app_id,
                                        stream_tx.as_ref(),
                                    )
                                    .await
                                    .unwrap_or_else(|e| {
                                        (None, false, 0, format!("Validation helper error: {}", e))
                                    });
                                if expose_public_requested && public_base_for_app.is_none() {
                                    public_base_for_app = self.load_public_base_url().await;
                                }
                                let local_base_url = Self::user_facing_local_base_url();
                                let local_access_url = Self::absolutize_public_url(
                                    Some(local_base_url.as_str()),
                                    &url_with_key,
                                );
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
                                        local_access_url
                                    ));
                                    if let Some(public_base) = public_base_for_app.as_deref() {
                                        let public_access_url = Self::absolutize_public_url(
                                            Some(public_base),
                                            &url_with_key,
                                        );
                                        if public_access_url != local_access_url {
                                            app_message_lines.push(format!(
                                                "- Public: [Open public app]({})",
                                                public_access_url
                                            ));
                                        }
                                    } else if expose_public_requested {
                                        app_message_lines.push(
                                            "- Public: still pending. I started tunnel setup and it should appear shortly."
                                                .to_string(),
                                        );
                                    }
                                } else if expose_public_requested {
                                    app_message_lines.push(
                                        "- Public: withheld until validation passes.".to_string(),
                                    );
                                } else {
                                    app_message_lines.push(
                                        "- Access link: withheld until validation passes."
                                            .to_string(),
                                    );
                                }

                                if access_guard_enabled {
                                    app_message_lines.push(
                                        "- Access guard: enabled.".to_string(),
                                    );
                                } else {
                                    app_message_lines.push(
                                        "- Access guard: not enabled.".to_string(),
                                    );
                                }

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

                                if verified {
                                    if let Some(preview) = preview_url {
                                        app_message_lines
                                            .push(format!("![App Preview]({})", preview));
                                    }
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
                        if let Some(schedule_result) =
                            self.handle_schedule_task(&call.arguments).await
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
                        if let Some(watch_result) = self.handle_watch(&call.arguments).await {
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
                        const MAX_SAME_ERROR_RETRIES: usize = 3;
                        const MAX_TOTAL_RETRIES: usize = 7;
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
}
