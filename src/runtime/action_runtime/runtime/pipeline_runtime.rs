use super::super::*;

impl ActionRuntime {
    pub(in crate::runtime) async fn connector_send_once(
        &self,
        client: &reqwest::Client,
        spec: &crate::core::connectivity::connector::ConnectorRequestSpec,
        query: &BTreeMap<String, String>,
    ) -> Result<(u16, String, String)> {
        let method = reqwest::Method::from_bytes(spec.method.as_str().as_bytes())
            .map_err(|e| anyhow::anyhow!("Invalid HTTP method: {}", e))?;
        let mut req = client.request(method.clone(), &spec.url);
        for (k, v) in &spec.headers {
            req = req.header(k, v);
        }
        if !query.is_empty() {
            req = req.query(query);
        }
        if method != reqwest::Method::GET && method != reqwest::Method::DELETE {
            if let Some(body) = spec.body.as_ref() {
                req = req.json(body);
            }
        }

        let resp = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Connector request network error: {}", e))?;
        let status = resp.status().as_u16();
        let request_url = resp.url().to_string();
        let body = resp.text().await.unwrap_or_default();
        Ok((status, body, request_url))
    }

    pub(in crate::runtime) async fn execute_pipeline_run(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        struct IdempotencyRecord {
            pipeline: String,
            node: String,
            status: String,
            stored_at: String,
            expires_at: String,
            output: serde_json::Value,
        }

        let allow_privileged = arguments
            .get("allow_privileged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let dry_run = arguments
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let spec = if let Some(spec_value) = arguments.get("spec") {
            serde_json::from_value::<crate::core::orchestration::pipeline::PipelineSpec>(
                spec_value.clone(),
            )
            .map_err(|e| anyhow::anyhow!("Invalid pipeline spec: {}", e))?
        } else {
            let pipeline_name = arguments
                .get("pipeline_name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing spec or pipeline_name"))?;
            self.load_saved_pipeline_spec(pipeline_name).await?
        };

        let compiled = crate::core::orchestration::pipeline::compile_pipeline(&spec)?;

        if dry_run {
            return Ok(serde_json::to_string_pretty(&serde_json::json!({
                "status": "dry_run",
                "pipeline": spec.name,
                "node_count": compiled.node_count,
                "ordered_nodes": compiled.ordered_nodes,
                "warnings": compiled.warnings,
            }))?);
        }

        let mut context = Self::context_map_from_json(arguments.get("context"));
        let now = chrono::Utc::now();
        context.insert("pipeline".to_string(), spec.name.clone());
        context.insert("date".to_string(), now.format("%Y-%m-%d").to_string());
        context.insert("timestamp".to_string(), now.to_rfc3339());

        let nodes_by_id: HashMap<String, crate::core::orchestration::pipeline::PipelineNode> = spec
            .nodes
            .iter()
            .cloned()
            .map(|n| (n.id.clone(), n))
            .collect();

        let run_id = uuid::Uuid::new_v4().to_string();
        let started_at = chrono::Utc::now();

        let mut outputs: HashMap<String, serde_json::Value> = HashMap::new();
        let mut failed_nodes: HashSet<String> = HashSet::new();
        let mut node_reports: Vec<serde_json::Value> = Vec::new();

        for node_id in &compiled.ordered_nodes {
            let node = nodes_by_id
                .get(node_id)
                .ok_or_else(|| anyhow::anyhow!("Missing compiled node '{}'", node_id))?;

            if let Some(dep) = node
                .depends_on
                .iter()
                .find(|dep| failed_nodes.contains(*dep))
                .cloned()
            {
                let msg = format!("Skipped: dependency '{}' failed", dep);
                node_reports.push(serde_json::json!({
                    "node_id": node.id,
                    "status": "skipped_dependency_failed",
                    "reason": msg,
                }));
                failed_nodes.insert(node.id.clone());
                if node.on_error == crate::core::orchestration::pipeline::NodeErrorMode::Fail {
                    let failed = serde_json::json!({
                        "status": "failed",
                        "pipeline": spec.name.clone(),
                        "run_id": run_id.clone(),
                        "started_at": started_at.to_rfc3339(),
                        "finished_at": chrono::Utc::now().to_rfc3339(),
                        "node_reports": node_reports,
                    });
                    self.persist_pipeline_run(&spec.name, &run_id, &failed)
                        .await?;
                    return Err(anyhow::anyhow!(
                        "Pipeline '{}' failed: node '{}' blocked by failed dependency '{}'",
                        spec.name,
                        node.id,
                        dep
                    ));
                }
                continue;
            }

            let mut node_ctx = context.clone();
            node_ctx.insert("node".to_string(), node.id.clone());
            for dep in &node.depends_on {
                if let Some(dep_out) = outputs.get(dep) {
                    node_ctx.insert(format!("output_{}", dep), dep_out.to_string());
                }
            }

            let rendered_args = Self::render_json_templates(&node.arguments, &node_ctx);
            let action_name = match node.kind {
                crate::core::orchestration::pipeline::NodeKind::Action => node.action.clone(),
                crate::core::orchestration::pipeline::NodeKind::ConnectorRequest => {
                    "connector_request".to_string()
                }
                crate::core::orchestration::pipeline::NodeKind::SignalConsensus => {
                    "signal_consensus".to_string()
                }
            };
            if action_name.trim().is_empty() {
                return Err(anyhow::anyhow!(
                    "Node '{}' resolved to empty action",
                    node.id
                ));
            }
            if action_name == "pipeline_run" || action_name == "pipeline_compile" {
                return Err(anyhow::anyhow!(
                    "Node '{}' uses forbidden nested orchestration action '{}'",
                    node.id,
                    action_name
                ));
            }
            if !allow_privileged && self.action_requires_privileged_allow(&action_name).await {
                let msg = format!(
                    "Node '{}' requires privileged action '{}'; set allow_privileged=true to run",
                    node.id, action_name
                );
                node_reports.push(serde_json::json!({
                    "node_id": node.id,
                    "action": action_name,
                    "status": "blocked_privileged",
                    "error": msg,
                }));
                failed_nodes.insert(node.id.clone());
                if node.on_error == crate::core::orchestration::pipeline::NodeErrorMode::Fail {
                    let failed = serde_json::json!({
                        "status": "failed",
                        "pipeline": spec.name.clone(),
                        "run_id": run_id.clone(),
                        "started_at": started_at.to_rfc3339(),
                        "finished_at": chrono::Utc::now().to_rfc3339(),
                        "node_reports": node_reports,
                    });
                    self.persist_pipeline_run(&spec.name, &run_id, &failed)
                        .await?;
                    return Err(anyhow::anyhow!("{}", msg));
                }
                continue;
            }

            let mut idempotent_hit = false;
            if let (Some(storage), Some(idem)) = (self.storage.as_ref(), node.idempotency.as_ref())
            {
                let mut idem_ctx = node_ctx.clone();
                idem_ctx.insert("pipeline".to_string(), spec.name.clone());
                idem_ctx.insert("node".to_string(), node.id.clone());
                let idem_key = crate::core::orchestration::pipeline::render_template(
                    &idem.key_template,
                    &idem_ctx,
                );
                let storage_key = format!("pipeline:idem:{}", idem_key);
                if let Some(raw) = storage.get(&storage_key).await? {
                    if let Ok(record) = serde_json::from_slice::<IdempotencyRecord>(&raw) {
                        if let Ok(expires) =
                            chrono::DateTime::parse_from_rfc3339(&record.expires_at)
                        {
                            if expires.with_timezone(&chrono::Utc) > chrono::Utc::now()
                                && record.status == "completed"
                            {
                                idempotent_hit = true;
                                outputs.insert(node.id.clone(), record.output.clone());
                                node_reports.push(serde_json::json!({
                                    "node_id": node.id,
                                    "action": action_name,
                                    "status": "idempotent_hit",
                                    "attempts": 0,
                                }));
                            } else if expires.with_timezone(&chrono::Utc) <= chrono::Utc::now() {
                                let _ = storage.delete(&storage_key).await;
                            }
                        }
                    }
                }
            }
            if idempotent_hit {
                continue;
            }

            let started = std::time::Instant::now();
            let retry = node.retry.normalized();
            match self
                .execute_action_with_retry(&action_name, &rendered_args, &retry)
                .await
            {
                Ok((output, attempts)) => {
                    let output_json = Self::coerce_to_json(&output);
                    outputs.insert(node.id.clone(), output_json.clone());

                    if let (Some(storage), Some(idem)) =
                        (self.storage.as_ref(), node.idempotency.as_ref())
                    {
                        let mut idem_ctx = node_ctx.clone();
                        idem_ctx.insert("pipeline".to_string(), spec.name.clone());
                        idem_ctx.insert("node".to_string(), node.id.clone());
                        let idem_key = crate::core::orchestration::pipeline::render_template(
                            &idem.key_template,
                            &idem_ctx,
                        );
                        let storage_key = format!("pipeline:idem:{}", idem_key);
                        let ttl_secs = idem.ttl_secs.clamp(60, 30 * 24 * 60 * 60);
                        let now_utc = chrono::Utc::now();
                        let expires_at = now_utc + chrono::Duration::seconds(ttl_secs as i64);
                        let record = IdempotencyRecord {
                            pipeline: spec.name.clone(),
                            node: node.id.clone(),
                            status: "completed".to_string(),
                            stored_at: now_utc.to_rfc3339(),
                            expires_at: expires_at.to_rfc3339(),
                            output: output_json,
                        };
                        storage
                            .set(&storage_key, &serde_json::to_vec(&record)?)
                            .await?;
                    }

                    node_reports.push(serde_json::json!({
                        "node_id": node.id,
                        "action": action_name,
                        "status": "completed",
                        "attempts": attempts,
                        "duration_ms": started.elapsed().as_millis(),
                    }));
                }
                Err(e) => {
                    failed_nodes.insert(node.id.clone());
                    let msg = e.to_string();
                    node_reports.push(serde_json::json!({
                        "node_id": node.id,
                        "action": action_name,
                        "status": if node.on_error == crate::core::orchestration::pipeline::NodeErrorMode::Continue { "failed_continue" } else { "failed" },
                        "error": msg,
                        "duration_ms": started.elapsed().as_millis(),
                    }));
                    if node.on_error == crate::core::orchestration::pipeline::NodeErrorMode::Fail {
                        let failed = serde_json::json!({
                            "status": "failed",
                            "pipeline": spec.name.clone(),
                            "run_id": run_id.clone(),
                            "started_at": started_at.to_rfc3339(),
                            "finished_at": chrono::Utc::now().to_rfc3339(),
                            "node_reports": node_reports,
                            "outputs": outputs,
                        });
                        self.persist_pipeline_run(&spec.name, &run_id, &failed)
                            .await?;
                        return Err(anyhow::anyhow!(
                            "Pipeline '{}' failed at node '{}': {}",
                            spec.name,
                            node.id,
                            e
                        ));
                    }
                }
            }
        }

        let mut selected_outputs = serde_json::Map::new();
        if spec.outputs.is_empty() {
            for (k, v) in &outputs {
                selected_outputs.insert(k.clone(), v.clone());
            }
        } else {
            for key in &spec.outputs {
                if let Some(v) = outputs.get(key) {
                    selected_outputs.insert(key.clone(), v.clone());
                }
            }
        }

        let status = if failed_nodes.is_empty() {
            "completed"
        } else {
            "completed_with_errors"
        };
        let result = serde_json::json!({
            "status": status,
            "pipeline": spec.name.clone(),
            "run_id": run_id.clone(),
            "started_at": started_at.to_rfc3339(),
            "finished_at": chrono::Utc::now().to_rfc3339(),
            "node_reports": node_reports,
            "outputs": selected_outputs,
        });
        self.persist_pipeline_run(&spec.name, &run_id, &result)
            .await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    pub(in crate::runtime) async fn load_saved_pipeline_spec(
        &self,
        pipeline_name: &str,
    ) -> Result<crate::core::orchestration::pipeline::PipelineSpec> {
        let storage = self
            .storage
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Storage is not available for saved pipelines"))?;
        let key = format!("pipeline:spec:{}", Self::pipeline_key_slug(pipeline_name));
        let raw = storage
            .get(&key)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Saved pipeline '{}' not found", pipeline_name))?;
        let spec =
            serde_json::from_slice::<crate::core::orchestration::pipeline::PipelineSpec>(&raw)
                .map_err(|e| {
                    anyhow::anyhow!("Saved pipeline '{}' is invalid: {}", pipeline_name, e)
                })?;
        Ok(spec)
    }

    pub(in crate::runtime) async fn persist_pipeline_run(
        &self,
        pipeline_name: &str,
        run_id: &str,
        payload: &serde_json::Value,
    ) -> Result<()> {
        if let Some(storage) = self.storage.as_ref() {
            let key = format!(
                "pipeline:run:{}:{}",
                Self::pipeline_key_slug(pipeline_name),
                run_id
            );
            storage.set(&key, &serde_json::to_vec(payload)?).await?;
        }
        Ok(())
    }

    pub(in crate::runtime) async fn execute_action_with_retry(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        retry: &crate::core::orchestration::pipeline::RetryPolicy,
    ) -> Result<(String, u32)> {
        let policy = retry.normalized();
        let mut attempt = 1u32;
        let mut backoff_ms = policy.initial_backoff_ms;

        loop {
            match std::pin::Pin::from(Box::new(self.execute_action(action_name, arguments))).await {
                Ok(output) => return Ok((output, attempt)),
                Err(err) => {
                    let message = err.to_string();
                    if attempt >= policy.max_attempts
                        || !Self::is_retryable_error(&message, &policy)
                    {
                        return Err(anyhow::anyhow!("{}", message));
                    }
                    Self::sleep_with_backoff(backoff_ms, policy.jitter_ratio).await;
                    backoff_ms = (backoff_ms.saturating_mul(2)).min(policy.max_backoff_ms);
                    attempt += 1;
                }
            }
        }
    }
}
