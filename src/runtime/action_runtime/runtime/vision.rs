use super::super::*;

impl ActionRuntime {
    pub(in crate::runtime) async fn execute_home_assistant(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let operation = arguments
            .get("operation")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("home_assistant requires an operation"))?;
        if !matches!(
            operation,
            "list_entities" | "search_entities" | "get_state" | "get_services"
        ) {
            anyhow::bail!("home_assistant only supports read-only operations");
        }
        let manager = crate::integrations::IntegrationManager::new(&self.config_dir);
        let result = manager
            .execute("home_assistant", operation, arguments)
            .await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    pub(in crate::runtime) async fn execute_home_assistant_call_service(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let manager = crate::integrations::IntegrationManager::new(&self.config_dir);
        let result = manager
            .execute("home_assistant", "call_service", arguments)
            .await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    pub(in crate::runtime) fn session_search_terms(query: Option<&str>) -> Vec<String> {
        query
            .unwrap_or_default()
            .split(|ch: char| !ch.is_alphanumeric())
            .map(|part| part.trim().to_ascii_lowercase())
            .filter(|part| part.chars().count() >= 2)
            .collect()
    }

    pub(in crate::runtime) fn session_search_score(text: &str, terms: &[String]) -> usize {
        if terms.is_empty() {
            return 1;
        }
        let haystack = text.to_ascii_lowercase();
        terms
            .iter()
            .filter(|term| haystack.contains(term.as_str()))
            .count()
    }

    pub(in crate::runtime) fn session_search_snippet(text: &str, max_chars: usize) -> String {
        let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if compact.chars().count() <= max_chars {
            compact
        } else {
            format!(
                "{}...",
                compact
                    .chars()
                    .take(max_chars.saturating_sub(3))
                    .collect::<String>()
            )
        }
    }

    pub(in crate::runtime) async fn execute_session_search(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let storage = self.runtime_storage()?;
        let query = arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let terms = Self::session_search_terms(query);
        let scope = arguments
            .get("scope")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("all");
        if !matches!(scope, "all" | "conversations" | "messages" | "traces") {
            anyhow::bail!("session_search scope must be all, conversations, messages, or traces");
        }
        let limit = arguments
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(8)
            .clamp(1, 25);
        let conversation_id = arguments
            .get("conversation_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let scan_limit = (limit * 12).clamp(40, 300);
        let mut hits: Vec<(usize, String, serde_json::Value)> = Vec::new();

        if matches!(scope, "all" | "conversations") {
            if let Some(conversation_id) = conversation_id {
                if let Some(conversation) = storage.get_conversation(conversation_id).await? {
                    let score = Self::session_search_score(&conversation.title, &terms);
                    hits.push((
                        score,
                        conversation.updated_at.clone(),
                        serde_json::json!({
                            "type": "conversation",
                            "id": conversation.id,
                            "title": conversation.title,
                            "channel": conversation.channel,
                            "message_count": conversation.message_count,
                            "updated_at": conversation.updated_at,
                            "match_score": score,
                        }),
                    ));
                }
            } else {
                for conversation in storage
                    .list_conversations(scan_limit, 0, None, &[], None)
                    .await?
                {
                    let text = format!("{} {}", conversation.title, conversation.channel);
                    let score = Self::session_search_score(&text, &terms);
                    hits.push((
                        score,
                        conversation.updated_at.clone(),
                        serde_json::json!({
                            "type": "conversation",
                            "id": conversation.id,
                            "title": conversation.title,
                            "channel": conversation.channel,
                            "message_count": conversation.message_count,
                            "updated_at": conversation.updated_at,
                            "match_score": score,
                        }),
                    ));
                }
            }
        }

        if matches!(scope, "all" | "messages") {
            let messages = if let Some(conversation_id) = conversation_id {
                storage
                    .get_recent_messages(conversation_id, scan_limit)
                    .await?
            } else {
                storage
                    .get_recent_messages_across_conversations(scan_limit)
                    .await?
            };
            for message in messages {
                let score = Self::session_search_score(&message.content, &terms);
                hits.push((
                    score,
                    message.timestamp.clone(),
                    serde_json::json!({
                        "type": "message",
                        "id": message.id,
                        "conversation_id": message.conversation_id,
                        "role": message.role,
                        "timestamp": message.timestamp,
                        "trace_id": message.trace_id,
                        "snippet": Self::session_search_snippet(&message.content, 420),
                        "match_score": score,
                    }),
                ));
            }
        }

        if matches!(scope, "all" | "traces") && conversation_id.is_none() {
            for trace in storage
                .list_execution_trace_summaries(None, scan_limit, 0)
                .await?
            {
                let text = format!(
                    "{} {} {}",
                    trace.message,
                    trace.steps_json,
                    trace.model.clone().unwrap_or_default()
                );
                let score = Self::session_search_score(&text, &terms);
                hits.push((
                    score,
                    trace.created_at.clone(),
                    serde_json::json!({
                        "type": "trace",
                        "id": trace.id,
                        "message": Self::session_search_snippet(&trace.message, 300),
                        "channel": trace.channel,
                        "started_at": trace.started_at,
                        "completed_at": trace.completed_at,
                        "duration_ms": trace.duration_ms,
                        "step_count": trace.step_count,
                        "total_tokens": trace.total_tokens,
                        "cost_usd": trace.cost_usd,
                        "complexity": trace.complexity,
                        "created_at": trace.created_at,
                        "match_score": score,
                    }),
                ));
            }
        }

        hits.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
        let results = hits
            .into_iter()
            .take(limit as usize)
            .map(|(_, _, payload)| payload)
            .collect::<Vec<_>>();

        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "query": query.unwrap_or(""),
            "scope": scope,
            "conversation_id": conversation_id,
            "result_count": results.len(),
            "results": results,
        }))?)
    }

    pub(in crate::runtime) fn media_provider_secret(
        config: &AgentConfig,
        provider: crate::integrations::media_gen::MediaProvider,
    ) -> Option<String> {
        let explicit = config
            .media_gen
            .provider_api_keys
            .iter()
            .find_map(|(raw_provider, key)| {
                let configured =
                    crate::integrations::media_gen::MediaProvider::parse(raw_provider)?;
                let trimmed = key.trim();
                if configured == provider && !trimmed.is_empty() && trimmed != "[ENCRYPTED]" {
                    Some(trimmed.to_string())
                } else {
                    None
                }
            });
        if explicit.is_some() {
            return explicit;
        }
        if provider == crate::integrations::media_gen::MediaProvider::OpenAiDalle {
            if let Some(key) = config.model_pool.slots.iter().find_map(|slot| {
                let crate::core::LlmProvider::OpenAI {
                    api_key, base_url, ..
                } = &slot.provider
                else {
                    return None;
                };
                if !slot.enabled
                    || api_key.trim().is_empty()
                    || api_key.trim() == "[ENCRYPTED]"
                    || crate::core::model::llm_provider::openai_provider_label(base_url.as_deref())
                        != "openai"
                {
                    return None;
                }
                Some(api_key.trim().to_string())
            }) {
                return Some(key);
            }
            if let crate::core::LlmProvider::OpenAI {
                api_key, base_url, ..
            } = &config.llm
            {
                if !api_key.trim().is_empty()
                    && api_key.trim() != "[ENCRYPTED]"
                    && crate::core::model::llm_provider::openai_provider_label(base_url.as_deref())
                        == "openai"
                {
                    return Some(api_key.trim().to_string());
                }
            }
        }
        None
    }

    pub(in crate::runtime) fn media_provider_base_url(
        config: &AgentConfig,
        provider: crate::integrations::media_gen::MediaProvider,
    ) -> String {
        let explicit =
            config
                .media_gen
                .provider_base_urls
                .iter()
                .find_map(|(raw_provider, base_url)| {
                    let configured =
                        crate::integrations::media_gen::MediaProvider::parse(raw_provider)?;
                    if configured == provider {
                        let trimmed = base_url.trim().trim_end_matches('/');
                        if !trimmed.is_empty() {
                            return Some(trimmed.to_string());
                        }
                    }
                    None
                });
        if let Some(base_url) = explicit {
            return base_url;
        }
        if provider == crate::integrations::media_gen::MediaProvider::OpenAiDalle {
            if let Some(base_url) = config.model_pool.slots.iter().find_map(|slot| {
                let crate::core::LlmProvider::OpenAI { base_url, .. } = &slot.provider else {
                    return None;
                };
                if !slot.enabled
                    || crate::core::model::llm_provider::openai_provider_label(base_url.as_deref())
                        != "openai"
                {
                    return None;
                }
                base_url
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.trim_end_matches('/').to_string())
            }) {
                return base_url;
            }
            if let crate::core::LlmProvider::OpenAI { base_url, .. } = &config.llm {
                if crate::core::model::llm_provider::openai_provider_label(base_url.as_deref())
                    == "openai"
                {
                    if let Some(base_url) = base_url
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|value| value.trim_end_matches('/').to_string())
                    {
                        return base_url;
                    }
                }
            }
        }
        provider.default_base_url().to_string()
    }

    pub(in crate::runtime) fn select_vision_provider(
        config: &AgentConfig,
        requested: Option<&str>,
    ) -> Result<crate::integrations::media_gen::MediaProvider> {
        use crate::integrations::media_gen::MediaProvider;

        let supports_vision = |provider: MediaProvider| {
            matches!(
                provider,
                MediaProvider::OpenAiDalle | MediaProvider::GoogleGemini
            )
        };

        if let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) {
            let provider = MediaProvider::parse(requested)
                .ok_or_else(|| anyhow::anyhow!("Unknown vision provider '{}'", requested))?;
            if !supports_vision(provider) {
                anyhow::bail!(
                    "Provider '{}' is not available for vision_ocr. Configure OpenAI Images or Google Gemini.",
                    provider.id()
                );
            }
            return Ok(provider);
        }

        if let Some(default_provider) = config
            .media_gen
            .default_image_provider
            .as_deref()
            .and_then(MediaProvider::parse)
        {
            if supports_vision(default_provider)
                && Self::media_provider_secret(config, default_provider).is_some()
            {
                return Ok(default_provider);
            }
        }

        [MediaProvider::OpenAiDalle, MediaProvider::GoogleGemini]
            .iter()
            .copied()
            .find(|provider| Self::media_provider_secret(config, *provider).is_some())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "vision_ocr needs a configured OpenAI Images or Google Gemini provider key in Settings > Media."
                )
            })
    }

    pub(in crate::runtime) fn default_vision_model(
        provider: crate::integrations::media_gen::MediaProvider,
    ) -> &'static str {
        use crate::integrations::media_gen::MediaProvider;
        match provider {
            MediaProvider::OpenAiDalle => "gpt-4.1-mini",
            MediaProvider::GoogleGemini => "gemini-2.5-flash",
            _ => "gpt-4.1-mini",
        }
    }

    pub(in crate::runtime) fn vision_instruction(
        task: &str,
        question: Option<&str>,
    ) -> Result<String> {
        let extra = question
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("");
        let base = match task {
            "extract_text" => {
                "Extract all visible text from the image. Preserve reading order, line breaks, tables, labels, and important layout cues where possible."
            }
            "describe" => {
                "Describe the image accurately and concisely. Include objects, visible UI, scene context, and any notable text."
            }
            "answer_question" => {
                if extra.is_empty() {
                    anyhow::bail!("vision_ocr answer_question requires a question");
                }
                "Answer the user's question using only what can be seen in the image. Note uncertainty when the image is ambiguous."
            }
            "analyze_document" => {
                "Analyze the visual document. Extract text, identify structure, summarize key fields, and call out unclear or missing values."
            }
            other => anyhow::bail!("Unsupported vision_ocr task '{}'", other),
        };
        if extra.is_empty() {
            Ok(base.to_string())
        } else {
            Ok(format!(
                "{base}\n\nUser question or extra instructions: {extra}"
            ))
        }
    }

    pub(in crate::runtime) fn normalized_vision_mime(
        filename: &str,
        content_type: Option<&str>,
        bytes: &[u8],
    ) -> Result<String> {
        let signature = Self::upload_signature(filename, content_type, bytes);
        let signature_mime = signature
            .get("mime")
            .and_then(|value| value.as_str())
            .and_then(|value| value.split(';').next())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let content_mime = content_type
            .and_then(|value| value.split(';').next())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let selected = signature_mime
            .filter(|mime| mime.starts_with("image/"))
            .or_else(|| signature_mime.filter(|mime| *mime == "application/pdf"))
            .or_else(|| {
                content_mime
                    .filter(|mime| mime.starts_with("image/") || *mime == "application/pdf")
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "vision_ocr accepts image or PDF uploads/URLs. Detected payload is not a supported vision input."
                )
            })?;
        Ok(selected.to_ascii_lowercase())
    }

    pub(in crate::runtime) fn vision_inline_max_bytes(mime_type: &str) -> usize {
        if mime_type == "application/pdf" {
            VISION_DOCUMENT_INLINE_MAX_BODY_BYTES
        } else {
            VISION_IMAGE_INLINE_MAX_BODY_BYTES
        }
    }

    pub(in crate::runtime) async fn load_vision_input(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<(String, String, String, Vec<u8>)> {
        let upload_id = arguments
            .get("upload_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let image_url = arguments
            .get("image_url")
            .and_then(|value| value.as_str())
            .or_else(|| arguments.get("file_url").and_then(|value| value.as_str()))
            .map(str::trim)
            .filter(|value| !value.is_empty());

        match (upload_id, image_url) {
            (Some(_), Some(_)) => {
                anyhow::bail!("vision_ocr accepts either upload_id or a URL, not both")
            }
            (Some(upload_id), None) => {
                let file = self.resolve_upload_for_sandbox(upload_id).await?;
                let mime = Self::normalized_vision_mime(
                    &file.filename,
                    file.content_type.as_deref(),
                    &file.bytes,
                )?;
                let max_bytes = Self::vision_inline_max_bytes(&mime);
                if file.bytes.len() > max_bytes {
                    anyhow::bail!(
                        "vision_ocr input is too large: {} bytes (max {})",
                        file.bytes.len(),
                        max_bytes
                    );
                }
                Ok((
                    format!("upload:{}", file.filename),
                    file.filename,
                    mime,
                    file.bytes,
                ))
            }
            (None, Some(raw_url)) => {
                let url = self.validate_http_get_url(raw_url).await?;
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(45))
                    .redirect(reqwest::redirect::Policy::limited(5))
                    .user_agent(crate::branding::user_agent_with_suffix(
                        "(Vision OCR Fetch)",
                    ))
                    .build()
                    .map_err(|error| {
                        anyhow::anyhow!("Failed to build vision fetch client: {}", error)
                    })?;
                let response = client.get(url.clone()).send().await?;
                let status = response.status();
                let content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                if !status.is_success() {
                    let body = response.text().await.unwrap_or_default();
                    anyhow::bail!("vision_ocr input fetch returned {}: {}", status, body);
                }
                let bytes = response.bytes().await?;
                let filename = url
                    .path_segments()
                    .and_then(|mut segments| segments.next_back())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("image");
                let bytes = bytes.to_vec();
                let mime = Self::normalized_vision_mime(filename, content_type.as_deref(), &bytes)?;
                let max_bytes = Self::vision_inline_max_bytes(&mime);
                if bytes.len() > max_bytes {
                    anyhow::bail!(
                        "vision_ocr input is too large: {} bytes (max {})",
                        bytes.len(),
                        max_bytes
                    );
                }
                Ok((url.as_str().to_string(), filename.to_string(), mime, bytes))
            }
            (None, None) => anyhow::bail!("vision_ocr requires upload_id, image_url, or file_url"),
        }
    }

    pub(in crate::runtime) fn openai_response_output_text(
        value: &serde_json::Value,
    ) -> Option<String> {
        if let Some(text) = value
            .get("output_text")
            .and_then(|text| text.as_str())
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            return Some(text.to_string());
        }

        let mut parts = Vec::new();
        if let Some(outputs) = value.get("output").and_then(|output| output.as_array()) {
            for output in outputs {
                let Some(content) = output.get("content").and_then(|content| content.as_array())
                else {
                    continue;
                };
                for item in content {
                    if let Some(text) = item
                        .get("text")
                        .and_then(|text| text.as_str())
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                    {
                        parts.push(text.to_string());
                    }
                }
            }
        }

        let joined = parts.join("\n").trim().to_string();
        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    }

    pub(in crate::runtime) fn openai_chat_vision_response_output_text(
        value: &serde_json::Value,
    ) -> Option<String> {
        if let Some(text) = value
            .pointer("/choices/0/message/content")
            .and_then(|content| {
                content.as_str().map(ToString::to_string).or_else(|| {
                    let mut parts = Vec::new();
                    for item in content.as_array()? {
                        if let Some(text) = item
                            .get("text")
                            .and_then(|text| text.as_str())
                            .map(str::trim)
                            .filter(|text| !text.is_empty())
                        {
                            parts.push(text.to_string());
                        }
                    }
                    let joined = parts.join("\n").trim().to_string();
                    (!joined.is_empty()).then_some(joined)
                })
            })
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
        {
            return Some(text);
        }
        Self::openai_response_output_text(value)
    }

    pub(in crate::runtime) fn gemini_response_output_text(
        value: &serde_json::Value,
    ) -> Option<String> {
        let mut parts = Vec::new();
        let candidates = value.get("candidates").and_then(|value| value.as_array())?;
        for candidate in candidates {
            let Some(content_parts) = candidate
                .pointer("/content/parts")
                .and_then(|value| value.as_array())
            else {
                continue;
            };
            for part in content_parts {
                if let Some(text) = part
                    .get("text")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    parts.push(text.to_string());
                }
            }
        }
        let joined = parts.join("\n").trim().to_string();
        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    }

    pub(in crate::runtime) async fn execute_openai_vision(
        &self,
        api_key: &str,
        base_url: &str,
        model: &str,
        detail: &str,
        instruction: &str,
        filename: &str,
        mime_type: &str,
        bytes: &[u8],
    ) -> Result<String> {
        let encoded_data =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
        let media_input = if mime_type == "application/pdf" {
            serde_json::json!({
                "type": "input_file",
                "filename": filename,
                "file_data": encoded_data,
            })
        } else {
            let image_url = format!("data:{mime_type};base64,{encoded_data}");
            serde_json::json!({
                "type": "input_image",
                "image_url": image_url,
                "detail": detail,
            })
        };
        let body = serde_json::json!({
            "model": model,
            "input": [{
                "role": "user",
                "content": [
                    { "type": "input_text", "text": instruction },
                    media_input
                ]
            }]
        });
        let response = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(90))
            .build()
            .map_err(|error| anyhow::anyhow!("Failed to build OpenAI vision client: {}", error))?
            .post(format!("{}/responses", base_url.trim_end_matches('/')))
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        let value: serde_json::Value = response
            .json()
            .await
            .unwrap_or_else(|_| serde_json::json!({}));
        if !status.is_success() {
            anyhow::bail!("OpenAI vision error {}: {}", status, value);
        }
        Self::openai_response_output_text(&value)
            .ok_or_else(|| anyhow::anyhow!("OpenAI vision response did not include text output"))
    }

    pub(in crate::runtime) async fn execute_openai_chat_vision(
        &self,
        api_key: &str,
        base_url: Option<&str>,
        model: &str,
        detail: &str,
        instruction: &str,
        filename: &str,
        mime_type: &str,
        bytes: &[u8],
    ) -> Result<String> {
        if mime_type == "application/pdf" {
            anyhow::bail!(
                "Primary chat vision fallback supports image uploads. Configure OpenAI Images or Google Gemini media vision for PDF analysis."
            );
        }
        let encoded_data =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
        let image_url = format!("data:{mime_type};base64,{encoded_data}");
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(90))
            .build()
            .map_err(|error| {
                anyhow::anyhow!("Failed to build OpenAI-compatible vision client: {}", error)
            })?;
        let request_config = crate::core::model::llm_provider::resolve_openai_request_config(
            &client, api_key, base_url, model,
        )
        .await?;
        if request_config.uses_codex_cli_oauth {
            anyhow::bail!(
                "OpenAI Subscription Codex backend is not available for uploaded image analysis. Configure a media vision provider or an OpenAI-compatible vision chat model."
            );
        }
        let body = serde_json::json!({
            "model": model,
            "stream": false,
            "max_tokens": 1800,
            "messages": [
                {
                    "role": "system",
                    "content": "You are AgentArk's chat visual-analysis tool. Analyze only observable content in the supplied upload. Return concise, user-facing text suitable for later answer synthesis or memory extraction. Do not infer sensitive traits, credentials, hidden data, or facts not visible in the image."
                },
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": format!("{instruction}\n\nFilename: {filename}")
                        },
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": image_url,
                                "detail": detail
                            }
                        }
                    ]
                }
            ]
        });
        let endpoint = format!("{}/chat/completions", request_config.base_url);
        let mut request = client
            .post(endpoint)
            .header(reqwest::header::CONTENT_TYPE, "application/json");
        if !request_config.api_key.is_empty() {
            request = request.bearer_auth(&request_config.api_key);
        }
        if request_config.is_openrouter {
            request = request
                .header("HTTP-Referer", crate::branding::REPOSITORY_URL)
                .header("X-Title", crate::branding::PRODUCT_NAME);
        }
        let response = request.json(&body).send().await?;
        let status = response.status();
        let value: serde_json::Value = response
            .json()
            .await
            .unwrap_or_else(|_| serde_json::json!({}));
        if !status.is_success() {
            anyhow::bail!("OpenAI-compatible vision error {}: {}", status, value);
        }
        Self::openai_chat_vision_response_output_text(&value).ok_or_else(|| {
            anyhow::anyhow!("OpenAI-compatible vision response did not include text output")
        })
    }

    pub(in crate::runtime) async fn execute_gemini_vision(
        &self,
        api_key: &str,
        base_url: &str,
        model: &str,
        instruction: &str,
        mime_type: &str,
        bytes: &[u8],
    ) -> Result<String> {
        let image_data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
        let body = serde_json::json!({
            "contents": [{
                "parts": [
                    {
                        "inline_data": {
                            "mime_type": mime_type,
                            "data": image_data
                        }
                    },
                    { "text": instruction }
                ]
            }]
        });
        let url = reqwest::Url::parse(&format!(
            "{}/models/{}:generateContent",
            base_url.trim_end_matches('/'),
            model.trim().trim_start_matches("models/")
        ))?;
        let response = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(90))
            .build()
            .map_err(|error| anyhow::anyhow!("Failed to build Gemini vision client: {}", error))?
            .post(url)
            .header("x-goog-api-key", api_key)
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        let value: serde_json::Value = response
            .json()
            .await
            .unwrap_or_else(|_| serde_json::json!({}));
        if !status.is_success() {
            anyhow::bail!("Gemini vision error {}: {}", status, value);
        }
        Self::gemini_response_output_text(&value)
            .ok_or_else(|| anyhow::anyhow!("Gemini vision response did not include text output"))
    }

    pub(in crate::runtime) fn openai_chat_vision_candidate_is_usable(
        api_key: &str,
        model: &str,
        base_url: Option<&str>,
    ) -> bool {
        let model = model.trim();
        if model.is_empty() {
            return false;
        }
        if base_url.is_some_and(crate::core::model::llm_provider::is_codex_cli_base_url) {
            return false;
        }

        let provider_label = crate::core::model::llm_provider::openai_provider_label(base_url);
        let missing_api_key = {
            let trimmed = api_key.trim();
            trimmed.is_empty() || trimmed == "[ENCRYPTED]"
        };
        if missing_api_key {
            !matches!(
                provider_label,
                crate::core::model::llm_provider::OPENAI_PROVIDER_ID
                    | crate::core::model::llm_provider::OPENROUTER_PROVIDER_ID
            )
        } else {
            true
        }
    }

    pub(in crate::runtime) fn push_openai_chat_vision_candidate(
        candidates: &mut Vec<OpenAiChatVisionCandidate>,
        provider: &crate::core::LlmProvider,
    ) {
        let crate::core::LlmProvider::OpenAI {
            api_key,
            model,
            base_url,
        } = provider
        else {
            return;
        };
        if Self::openai_chat_vision_candidate_is_usable(api_key, model, base_url.as_deref()) {
            candidates.push(OpenAiChatVisionCandidate {
                api_key: api_key.clone(),
                model: model.clone(),
                base_url: base_url.clone(),
            });
        }
    }

    pub(in crate::runtime) fn openai_compatible_chat_vision_candidates(
        config: &AgentConfig,
    ) -> Vec<OpenAiChatVisionCandidate> {
        let mut candidates = Vec::new();

        if !config.model_pool.slots.is_empty() {
            for slot in config.model_pool.slots.iter().filter(|slot| {
                slot.enabled && slot.role == crate::core::runtime::config::ModelRole::Primary
            }) {
                Self::push_openai_chat_vision_candidate(&mut candidates, &slot.provider);
            }
            for slot in config.model_pool.slots.iter().filter(|slot| {
                slot.enabled && slot.role != crate::core::runtime::config::ModelRole::Primary
            }) {
                Self::push_openai_chat_vision_candidate(&mut candidates, &slot.provider);
            }
        }

        Self::push_openai_chat_vision_candidate(&mut candidates, &config.llm);
        Self::dedupe_openai_chat_vision_candidates(candidates)
    }

    pub(in crate::runtime) fn dedupe_openai_chat_vision_candidates(
        candidates: Vec<OpenAiChatVisionCandidate>,
    ) -> Vec<OpenAiChatVisionCandidate> {
        let mut seen = HashSet::new();
        let mut deduped = Vec::new();
        for candidate in candidates {
            let key = format!(
                "{}\n{}\n{}",
                candidate.provider_label(),
                candidate.base_url.as_deref().unwrap_or("").trim(),
                candidate.model.trim().to_ascii_lowercase()
            );
            if seen.insert(key) {
                deduped.push(candidate);
            }
        }
        deduped
    }

    pub(in crate::runtime) fn compact_vision_error(error: &anyhow::Error) -> String {
        let mut parts = Vec::new();
        for cause in error.chain() {
            let redacted = crate::security::redact_secret_input(&cause.to_string()).text;
            let collapsed = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
            if !collapsed.is_empty() && parts.last() != Some(&collapsed) {
                parts.push(collapsed);
            }
        }
        let mut text = parts.join(": ");
        const MAX_ERROR_CHARS: usize = 900;
        if text.chars().count() > MAX_ERROR_CHARS {
            text = text.chars().take(MAX_ERROR_CHARS).collect::<String>();
            text.push_str("...");
        }
        text
    }

    pub(in crate::runtime) async fn execute_vision_ocr(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let settings = self.settings_manager()?.load()?;
        let requested_provider = arguments.get("provider").and_then(|value| value.as_str());
        let task = arguments
            .get("task")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("extract_text");
        let detail = arguments
            .get("detail")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("auto");
        if !matches!(detail, "auto" | "low" | "high") {
            anyhow::bail!("vision_ocr detail must be auto, low, or high");
        }
        let instruction = Self::vision_instruction(
            task,
            arguments.get("question").and_then(|value| value.as_str()),
        )?;
        let (source, filename, mime_type, bytes) = self.load_vision_input(arguments).await?;
        let selected_media_provider = Self::select_vision_provider(&settings, requested_provider);
        let model_override = arguments
            .get("model")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if requested_provider.is_some() {
            let provider = selected_media_provider?;
            let api_key = Self::media_provider_secret(&settings, provider).ok_or_else(|| {
                anyhow::anyhow!(
                    "Provider '{}' is selected for vision_ocr but has no configured API key.",
                    provider.id()
                )
            })?;
            let base_url = Self::media_provider_base_url(&settings, provider);
            let model = arguments
                .get("model")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| Self::default_vision_model(provider))
                .to_string();

            let text = match provider {
                crate::integrations::media_gen::MediaProvider::OpenAiDalle => {
                    self.execute_openai_vision(
                        &api_key,
                        &base_url,
                        &model,
                        detail,
                        &instruction,
                        &filename,
                        &mime_type,
                        &bytes,
                    )
                    .await?
                }
                crate::integrations::media_gen::MediaProvider::GoogleGemini => {
                    self.execute_gemini_vision(
                        &api_key,
                        &base_url,
                        &model,
                        &instruction,
                        &mime_type,
                        &bytes,
                    )
                    .await?
                }
                _ => unreachable!("select_vision_provider only returns supported vision providers"),
            };

            return Ok(serde_json::to_string_pretty(&serde_json::json!({
                "provider": provider.id(),
                "mode": "media_vision",
                "model": model,
                "task": task,
                "source": source,
                "mime_type": mime_type,
                "text": text,
            }))?);
        }

        let media_provider_error = selected_media_provider
            .as_ref()
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();

        let mut failures = Vec::new();
        if mime_type != "application/pdf" {
            let mut attempted_chat = HashSet::new();
            for candidate in Self::openai_compatible_chat_vision_candidates(&settings) {
                let model = model_override
                    .unwrap_or(candidate.model.as_str())
                    .to_string();
                let attempt_key = format!(
                    "{}\n{}\n{}",
                    candidate.provider_label(),
                    candidate.base_url.as_deref().unwrap_or("").trim(),
                    model.trim().to_ascii_lowercase()
                );
                if !attempted_chat.insert(attempt_key.clone()) {
                    continue;
                }
                match self
                    .execute_openai_chat_vision(
                        candidate.api_key.as_str(),
                        candidate.base_url.as_deref(),
                        &model,
                        detail,
                        &instruction,
                        &filename,
                        &mime_type,
                        &bytes,
                    )
                    .await
                {
                    Ok(text) => {
                        return Ok(serde_json::to_string_pretty(&serde_json::json!({
                            "provider": candidate.provider_label(),
                            "mode": "configured_chat_vision",
                            "model": model,
                            "task": task,
                            "source": source,
                            "mime_type": mime_type,
                            "text": text,
                        }))?);
                    }
                    Err(error) => {
                        failures.push(format!(
                            "{} model '{}' failed: {}",
                            candidate.provider_label(),
                            model,
                            Self::compact_vision_error(&error)
                        ));
                    }
                }
            }
        }

        if let Ok(provider) = selected_media_provider {
            let api_key = Self::media_provider_secret(&settings, provider).ok_or_else(|| {
                anyhow::anyhow!(
                    "Provider '{}' is selected for vision_ocr but has no configured API key.",
                    provider.id()
                )
            })?;
            let base_url = Self::media_provider_base_url(&settings, provider);
            let model = model_override
                .unwrap_or_else(|| Self::default_vision_model(provider))
                .to_string();

            let result = match provider {
                crate::integrations::media_gen::MediaProvider::OpenAiDalle => {
                    self.execute_openai_vision(
                        &api_key,
                        &base_url,
                        &model,
                        detail,
                        &instruction,
                        &filename,
                        &mime_type,
                        &bytes,
                    )
                    .await
                }
                crate::integrations::media_gen::MediaProvider::GoogleGemini => {
                    self.execute_gemini_vision(
                        &api_key,
                        &base_url,
                        &model,
                        &instruction,
                        &mime_type,
                        &bytes,
                    )
                    .await
                }
                _ => unreachable!("select_vision_provider only returns supported vision providers"),
            };

            match result {
                Ok(text) => {
                    return Ok(serde_json::to_string_pretty(&serde_json::json!({
                        "provider": provider.id(),
                        "mode": "media_vision",
                        "model": model,
                        "task": task,
                        "source": source,
                        "mime_type": mime_type,
                        "text": text,
                    }))?);
                }
                Err(error) => {
                    failures.push(format!(
                        "{} model '{}' failed: {}",
                        provider.id(),
                        model,
                        Self::compact_vision_error(&error)
                    ));
                }
            }
        }

        if failures.is_empty() {
            if mime_type == "application/pdf" {
                anyhow::bail!(
                    "vision_ocr could not analyze this PDF because no dedicated media vision provider is configured. Configure OpenAI Images or Google Gemini in Settings > Media."
                );
            }
            anyhow::bail!(
                "vision_ocr could not analyze this image because no usable configured chat vision model was available and no dedicated media vision provider is configured ({media_provider_error})."
            );
        }

        anyhow::bail!(
            "vision_ocr could not analyze this upload. Attempts: {}{}",
            failures.join(" | "),
            if media_provider_error.is_empty() {
                String::new()
            } else {
                format!(" | media provider selection: {media_provider_error}")
            }
        )
    }
}
