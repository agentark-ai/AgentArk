use super::super::*;

impl ActionRuntime {
    pub(in crate::runtime) async fn execute_pipeline_compile(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let spec_value = arguments
            .get("spec")
            .ok_or_else(|| anyhow::anyhow!("Missing spec"))?;
        let spec: crate::core::orchestration::pipeline::PipelineSpec =
            serde_json::from_value(spec_value.clone())
                .map_err(|e| anyhow::anyhow!("Invalid pipeline spec: {}", e))?;
        let compiled = crate::core::orchestration::pipeline::compile_pipeline(&spec)?;

        let save = arguments
            .get("save")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let mut persisted = false;
        let mut warnings = compiled.warnings.clone();
        if save {
            if let Some(storage) = self.storage.as_ref() {
                let key = format!("pipeline:spec:{}", Self::pipeline_key_slug(&spec.name));
                storage.set(&key, &serde_json::to_vec(&spec)?).await?;
                persisted = true;
            } else {
                warnings.push("storage unavailable; pipeline spec not persisted".to_string());
            }
        }

        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "compiled",
            "pipeline": compiled.name,
            "node_count": compiled.node_count,
            "ordered_nodes": compiled.ordered_nodes,
            "warnings": warnings,
            "persisted": persisted,
        }))?)
    }

    pub(in crate::runtime) async fn execute_signal_consensus(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let request: crate::core::orchestration::pipeline::SignalConsensusRequest =
            serde_json::from_value(arguments.clone())
                .map_err(|e| anyhow::anyhow!("Invalid signal_consensus arguments: {}", e))?;
        let result = crate::core::orchestration::pipeline::run_signal_consensus(&request)?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    pub(in crate::runtime) async fn execute_connector_request(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let spec: crate::core::connectivity::connector::ConnectorRequestSpec =
            serde_json::from_value(arguments.clone())
                .map_err(|e| anyhow::anyhow!("Invalid connector_request arguments: {}", e))?;

        if spec.url.trim().is_empty() {
            return Err(anyhow::anyhow!("connector_request requires non-empty url"));
        }
        self.validate_connector_request_url(&spec.url).await?;

        let retry = spec.retry.normalized();
        let timeout_secs = spec.timeout_secs.clamp(1, 300);
        let client = reqwest::Client::builder()
            .user_agent(crate::branding::user_agent_with_suffix(
                "(AI Agent Browser)",
            ))
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {}", e))?;

        let mut pages = Vec::new();
        let mut items = Vec::new();
        let mut total_requests = 0usize;

        let pagination = &spec.pagination;
        let max_pages = pagination.max_pages.clamp(1, 1_000);
        let mut page_no = pagination.start_page.max(1);
        let mut cursor: Option<String> = None;

        for _ in 0..max_pages {
            let mut query = spec.query.clone();
            if let Some(page_size) = pagination.page_size {
                if page_size > 0 {
                    query.insert(pagination.page_size_param.clone(), page_size.to_string());
                }
            }
            match pagination.mode {
                crate::core::connectivity::connector::PaginationMode::Page => {
                    query.insert(pagination.page_param.clone(), page_no.to_string());
                }
                crate::core::connectivity::connector::PaginationMode::Cursor => {
                    if let Some(ref c) = cursor {
                        query.insert(pagination.cursor_param.clone(), c.clone());
                    }
                }
                crate::core::connectivity::connector::PaginationMode::None => {}
            }

            let mut attempt = 1u32;
            let mut backoff_ms = retry.initial_backoff_ms;
            let mut refreshed = false;
            let (status, body_text, request_url) = loop {
                total_requests += 1;
                match self.connector_send_once(&client, &spec, &query).await {
                    Ok((status, body_text, request_url)) => {
                        if !(200..300).contains(&status) {
                            if let Some(refresh) = spec.auth_refresh.as_ref() {
                                if refresh.retry_statuses.contains(&status) && !refreshed {
                                    if refresh.action.eq_ignore_ascii_case("connector_request") {
                                        return Err(anyhow::anyhow!(
                                            "auth_refresh.action cannot be connector_request"
                                        ));
                                    }
                                    // Break async recursion cycle:
                                    // execute_action -> execute_native -> execute_connector_request -> execute_action
                                    std::pin::Pin::from(Box::new(
                                        self.execute_action(&refresh.action, &refresh.arguments),
                                    ))
                                    .await?;
                                    refreshed = true;
                                    continue;
                                }
                            }
                            if attempt < retry.max_attempts
                                && retry.retry_on_status.contains(&status)
                            {
                                Self::sleep_with_backoff(backoff_ms, retry.jitter_ratio).await;
                                backoff_ms =
                                    (backoff_ms.saturating_mul(2)).min(retry.max_backoff_ms);
                                attempt += 1;
                                continue;
                            }
                            let snippet = if body_text.len() > 500 {
                                format!("{}...", &body_text[..500])
                            } else {
                                body_text.clone()
                            };
                            return Err(anyhow::anyhow!(
                                "Connector request failed (status {}): {}",
                                status,
                                snippet
                            ));
                        }
                        break (status, body_text, request_url);
                    }
                    Err(e) => {
                        if attempt < retry.max_attempts {
                            Self::sleep_with_backoff(backoff_ms, retry.jitter_ratio).await;
                            backoff_ms = (backoff_ms.saturating_mul(2)).min(retry.max_backoff_ms);
                            attempt += 1;
                            continue;
                        }
                        return Err(e);
                    }
                }
            };

            let payload: serde_json::Value = serde_json::from_str(&body_text)
                .unwrap_or_else(|_| serde_json::json!({ "raw_body": body_text }));

            let mut page_items = crate::core::connectivity::connector::extract_items(
                &payload,
                &pagination.items_path,
            );
            if page_items.is_empty()
                && pagination.mode == crate::core::connectivity::connector::PaginationMode::None
            {
                page_items = match &payload {
                    serde_json::Value::Array(arr) => arr.clone(),
                    other => vec![other.clone()],
                };
            }

            let next_cursor = crate::core::connectivity::connector::extract_next_cursor(
                &payload,
                &pagination.next_cursor_path,
            );

            pages.push(crate::core::connectivity::connector::ConnectorPageResult {
                request_url,
                status,
                item_count: page_items.len(),
                next_cursor: next_cursor.clone(),
            });
            items.extend(page_items);

            let done = match pagination.mode {
                crate::core::connectivity::connector::PaginationMode::None => true,
                crate::core::connectivity::connector::PaginationMode::Page => {
                    if pages.last().map(|p| p.item_count == 0).unwrap_or(true) {
                        true
                    } else {
                        page_no = page_no.saturating_add(1);
                        false
                    }
                }
                crate::core::connectivity::connector::PaginationMode::Cursor => {
                    if next_cursor.as_ref().is_none() || next_cursor.as_ref() == cursor.as_ref() {
                        true
                    } else {
                        cursor = next_cursor;
                        false
                    }
                }
            };
            if done {
                break;
            }

            if spec.rate_limit_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(spec.rate_limit_ms)).await;
            }
        }

        let out = crate::core::connectivity::connector::ConnectorRunResult {
            method: spec.method.as_str().to_string(),
            total_requests,
            total_items: items.len(),
            pages,
            items,
        };
        Ok(serde_json::to_string_pretty(&out)?)
    }
}
