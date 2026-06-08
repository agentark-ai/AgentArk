use super::super::*;

impl ActionRuntime {
    pub(in crate::runtime) fn runtime_storage(&self) -> Result<crate::storage::Storage> {
        self.storage
            .clone()
            .ok_or_else(|| anyhow::anyhow!("AgentArk storage is not available in this runtime"))
    }

    pub(in crate::runtime) fn compact_text(value: &str, max_chars: usize) -> String {
        if value.chars().count() <= max_chars {
            return value.to_string();
        }
        value.chars().take(max_chars).collect::<String>()
    }

    pub(in crate::runtime) async fn load_storage_json_value(
        storage: &crate::storage::Storage,
        key: &str,
    ) -> Option<serde_json::Value> {
        storage
            .get(key)
            .await
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok())
    }

    pub(in crate::runtime) fn preview_json_array(
        value: Option<serde_json::Value>,
        limit: usize,
    ) -> Option<serde_json::Value> {
        match value {
            Some(serde_json::Value::Array(items)) => Some(serde_json::Value::Array(
                items
                    .into_iter()
                    .rev()
                    .take(limit)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect(),
            )),
            other => other,
        }
    }

    pub(in crate::runtime) fn pulse_event_from_storage_row(
        row: crate::storage::arkpulse_event::Model,
    ) -> Option<crate::sentinel::PulseEvent> {
        Some(crate::sentinel::PulseEvent {
            timestamp: row.timestamp,
            status: row.status,
            message: row.message,
            summary: row.summary,
            flags: serde_json::from_str(&row.flags_json).ok()?,
            overdue_tasks: row.overdue_tasks.max(0) as usize,
            failed_tasks: row.failed_tasks.max(0) as usize,
            details: serde_json::from_str(&row.details_json).ok()?,
        })
    }

    pub(in crate::runtime) fn summarize_pulse_event(
        row: &crate::storage::arkpulse_event::Model,
    ) -> serde_json::Value {
        let flags = serde_json::from_str::<serde_json::Value>(&row.flags_json)
            .unwrap_or_else(|_| serde_json::json!([]));
        let details = serde_json::from_str::<serde_json::Value>(&row.details_json)
            .unwrap_or_else(|_| serde_json::json!({}));
        let doctor_findings = details
            .get("doctor_findings")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let health_checks = details
            .get("health_checks")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        serde_json::json!({
            "timestamp": row.timestamp,
            "status": row.status,
            "message": Self::compact_text(&row.message, 180),
            "summary": Self::compact_text(&row.summary, 220),
            "flags": flags,
            "overdue_tasks": row.overdue_tasks.max(0),
            "failed_tasks": row.failed_tasks.max(0),
            "doctor_finding_count": doctor_findings.len(),
            "health_check_count": health_checks.len(),
            "details": details,
        })
    }

    pub(in crate::runtime) async fn inspect_arkpulse_json(
        &self,
        storage: &crate::storage::Storage,
        limit: u64,
    ) -> Result<serde_json::Value> {
        let rows = storage.list_arkpulse_events(limit.max(12)).await?;
        let stored_count = storage
            .count_arkpulse_events()
            .await
            .unwrap_or(rows.len() as u64);
        let latest = rows.first();
        let latest_details = latest
            .and_then(|row| serde_json::from_str::<serde_json::Value>(&row.details_json).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        let latest_flags = latest
            .and_then(|row| serde_json::from_str::<serde_json::Value>(&row.flags_json).ok())
            .unwrap_or_else(|| serde_json::json!([]));
        let mut anomalies = Vec::new();
        if let Some(row) = latest {
            if !row.status.eq_ignore_ascii_case("ok") {
                anomalies.push(serde_json::json!({
                    "severity": row.status,
                    "message": Self::compact_text(&row.summary, 220),
                }));
            }
            if row.failed_tasks > 0 {
                anomalies.push(serde_json::json!({
                    "severity": "warn",
                    "message": format!("{} failed task(s) were observed in the latest Pulse run.", row.failed_tasks),
                }));
            }
            if row.overdue_tasks > 0 {
                anomalies.push(serde_json::json!({
                    "severity": "warn",
                    "message": format!("{} overdue task(s) were observed in the latest Pulse run.", row.overdue_tasks),
                }));
            }
        }
        if let Some(findings) = latest_details
            .get("doctor_findings")
            .and_then(|value| value.as_array())
        {
            for finding in findings.iter().take(limit as usize) {
                let severity = finding
                    .get("severity")
                    .and_then(|value| value.as_str())
                    .unwrap_or("info");
                if severity.eq_ignore_ascii_case("info") {
                    continue;
                }
                anomalies.push(serde_json::json!({
                    "severity": severity,
                    "title": finding.get("title").and_then(|value| value.as_str()),
                    "message": finding.get("message").and_then(|value| value.as_str()),
                    "target": finding.get("target").and_then(|value| value.as_str()),
                }));
            }
        }
        Ok(serde_json::json!({
            "surface": "arkpulse",
            "running": crate::sentinel::is_pulse_running(),
            "stored_event_count": stored_count,
            "latest_status": latest.map(|row| row.status.clone()),
            "latest_timestamp": latest.map(|row| row.timestamp.clone()),
            "latest_flags": latest_flags,
            "anomalies": anomalies,
            "recent_events": rows.iter().take(limit as usize).map(Self::summarize_pulse_event).collect::<Vec<_>>(),
        }))
    }

    pub(in crate::runtime) async fn inspect_gateway_ops_json(
        &self,
        storage: &crate::storage::Storage,
        limit: u64,
    ) -> Result<serde_json::Value> {
        let config = self.settings_manager()?.load()?;
        let pulse_rows = storage.list_arkpulse_events(limit.max(12)).await?;
        let pulse_events = pulse_rows
            .into_iter()
            .filter_map(Self::pulse_event_from_storage_row)
            .collect::<Vec<_>>();
        let overview = crate::core::GatewayOpsControlPlane::overview_from_parts(
            storage,
            &config,
            Some(pulse_events.as_slice()),
        )
        .await?;
        Ok(serde_json::json!({
            "surface": "gateway_ops",
            "overview": overview,
        }))
    }

    pub(in crate::runtime) async fn inspect_sentinel_json(
        &self,
        storage: &crate::storage::Storage,
        limit: u64,
    ) -> Result<serde_json::Value> {
        let autonomy_settings = storage
            .get(crate::core::AUTONOMY_SETTINGS_STORAGE_KEY)
            .await?
            .and_then(|raw| serde_json::from_slice::<crate::core::AutonomySettings>(&raw).ok())
            .unwrap_or_default();
        let scan_state = Self::load_storage_json_value(storage, "sentinel_scan_state_v1").await;
        let observations = Self::preview_json_array(
            Self::load_storage_json_value(storage, "sentinel_observations_v1").await,
            limit as usize,
        );
        let proposals = Self::preview_json_array(
            Self::load_storage_json_value(storage, "sentinel_proposals_v1").await,
            limit as usize,
        );
        let background_learning =
            crate::channels::http::load_background_learning_feed(storage, &autonomy_settings).await;
        Ok(serde_json::json!({
            "surface": "sentinel",
            "autonomy_mode": autonomy_settings.autonomy_mode,
            "agent_paused": autonomy_settings.agent_paused,
            "settings": autonomy_settings.sentinel,
            "scan_state": scan_state,
            "observation_count": observations.as_ref().and_then(|value| value.as_array()).map(|items| items.len()).unwrap_or(0),
            "proposal_count": proposals.as_ref().and_then(|value| value.as_array()).map(|items| items.len()).unwrap_or(0),
            "observations": observations,
            "proposals": proposals,
            "background_learning": background_learning,
        }))
    }

    pub(in crate::runtime) async fn inspect_evolution_json(
        &self,
        storage: &crate::storage::Storage,
        limit: u64,
    ) -> Result<serde_json::Value> {
        let learning_enabled =
            crate::core::knowledge::learning::load_learning_enabled(storage).await;
        let learning_model_slot =
            crate::core::knowledge::learning::load_learning_model_slot(storage).await;
        let learning_queue_cap =
            crate::core::knowledge::learning::load_learning_queue_cap(storage).await;
        let queue_counts = storage.learning_queue_counts().await?;
        let candidates = storage
            .list_learning_candidates_with_options(None, false, limit)
            .await?;
        let patterns = storage
            .list_procedural_patterns(None, None, &["active", "draft"], limit)
            .await?;
        let items = storage
            .list_active_experience_items(
                &["constraint", "personal_fact", "lesson", "procedure"],
                None,
                None,
                limit,
            )
            .await?;
        let recent_runs = storage.list_recent_experience_runs_any_scope(limit).await?;
        Ok(serde_json::json!({
            "surface": "evolution",
            "learning_enabled": learning_enabled,
            "learning_model_slot": learning_model_slot,
            "learning_queue_cap": learning_queue_cap,
            "queue_counts": queue_counts,
            "review_queue_size": candidates.iter().filter(|candidate| candidate.approval_status == "draft").count(),
            "recent_candidates": candidates,
            "recent_patterns": patterns,
            "recent_items": items,
            "recent_runs": recent_runs,
        }))
    }

    pub(in crate::runtime) async fn inspect_trace_json(
        &self,
        storage: &crate::storage::Storage,
        trace_id: Option<&str>,
        limit: u64,
    ) -> Result<serde_json::Value> {
        if let Some(trace_id) = trace_id.map(str::trim).filter(|value| !value.is_empty()) {
            let trace = storage.get_execution_trace(trace_id).await?;
            let logs = storage
                .list_operational_logs_for_trace_ids(&[trace_id.to_string()], limit.max(12))
                .await?;
            return Ok(serde_json::json!({
                "surface": "trace",
                "trace_id": trace_id,
                "trace": trace,
                "operational_logs": logs,
            }));
        }

        let traces = storage
            .list_execution_trace_summaries(None, limit, 0)
            .await?;
        let trace_ids = traces
            .iter()
            .map(|trace| trace.id.clone())
            .collect::<Vec<_>>();
        let logs = storage
            .list_operational_logs_for_trace_ids(&trace_ids, limit.max(12))
            .await?;
        Ok(serde_json::json!({
            "surface": "trace",
            "recent_traces": traces,
            "recent_operational_logs": logs,
        }))
    }

    pub(in crate::runtime) async fn inspect_moltbook_json(
        &self,
        storage: &crate::storage::Storage,
        limit: u64,
    ) -> Result<serde_json::Value> {
        let activity = Self::preview_json_array(
            Self::load_storage_json_value(storage, "moltbook_activity_log_v1").await,
            limit as usize,
        )
        .unwrap_or_else(|| serde_json::json!([]));
        let recent_errors = activity
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter(|item| {
                        item.get("level")
                            .and_then(|value| value.as_str())
                            .map(|level| level.eq_ignore_ascii_case("error"))
                            .unwrap_or(false)
                    })
                    .take(limit as usize)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(serde_json::json!({
            "surface": "moltbook",
            "configured": crate::integrations::moltbook::MoltbookConnector::new_with_config_dir(
                self.config_dir.clone()
            ).has_configured_api_key(),
            "recent_activity": activity,
            "recent_errors": recent_errors,
        }))
    }

    pub(in crate::runtime) fn agentark_knowledge_query_terms(query: &str) -> Vec<String> {
        query
            .split(|ch: char| !ch.is_alphanumeric())
            .map(|part| part.trim().to_ascii_lowercase())
            .filter(|part| part.chars().count() >= 2)
            .collect()
    }

    pub(in crate::runtime) fn agentark_knowledge_match_score(
        text: &str,
        terms: &[String],
    ) -> usize {
        if terms.is_empty() {
            return 1;
        }
        let haystack = text.to_ascii_lowercase();
        terms
            .iter()
            .filter(|term| haystack.contains(term.as_str()))
            .count()
    }

    pub(in crate::runtime) fn agentark_knowledge_chunk_field(
        content: &str,
        field: &str,
    ) -> Option<String> {
        let prefix = format!("{field}:");
        content.lines().find_map(|line| {
            line.strip_prefix(&prefix)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
    }

    pub(in crate::runtime) fn agentark_knowledge_hit_json(
        hit: crate::core::knowledge::document_search::DocumentSearchHit,
    ) -> serde_json::Value {
        let title = Self::agentark_knowledge_chunk_field(&hit.content, "title")
            .unwrap_or_else(|| hit.filename.clone());
        let source = Self::agentark_knowledge_chunk_field(&hit.content, "source")
            .unwrap_or_else(|| "agentark_knowledge".to_string());
        let url = Self::agentark_knowledge_chunk_field(&hit.content, "url");
        let tags = Self::agentark_knowledge_chunk_field(&hit.content, "tags");
        serde_json::json!({
            "source": source,
            "result_type": "agentark_knowledge_document",
            "authority": "supplemental_manual",
            "availability_authority": "does_not_override_live_registry",
            "title": title,
            "document_id": hit.document_id,
            "chunk_index": hit.chunk_index,
            "content": Self::compact_text(&hit.content, 1800),
            "score": hit.score,
            "lexical_score": hit.lexical_score,
            "dense_score": hit.dense_score,
            "match_reason": hit.match_reason,
            "url": url,
            "tags": tags,
        })
    }

    pub(in crate::runtime) fn document_lookup_hit_json(
        hit: crate::core::knowledge::document_search::DocumentSearchHit,
    ) -> serde_json::Value {
        serde_json::json!({
            "filename": hit.filename,
            "document_id": hit.document_id,
            "chunk_index": hit.chunk_index,
            "content": Self::compact_text(&hit.content, 1800),
            "score": hit.score,
            "lexical_score": hit.lexical_score,
            "dense_score": hit.dense_score,
            "match_reason": hit.match_reason,
        })
    }

    pub(in crate::runtime) fn document_lookup_doc_ids_from_arguments(
        arguments: &serde_json::Value,
    ) -> Result<std::collections::HashSet<String>> {
        let mut doc_ids = std::collections::HashSet::new();
        let Some(items) = arguments.get("doc_ids") else {
            return Ok(doc_ids);
        };
        let Some(items) = items.as_array() else {
            anyhow::bail!("document_lookup doc_ids must be an array when supplied");
        };
        for item in items {
            let Some(raw) = item
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            if !raw
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
            {
                anyhow::bail!("document_lookup doc_ids contain unsupported characters");
            }
            doc_ids.insert(raw.to_string());
            if doc_ids.len() >= 16 {
                break;
            }
        }
        Ok(doc_ids)
    }

    pub(in crate::runtime) async fn execute_document_lookup(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let query = arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("document_lookup requires a non-empty query"))?;
        let limit = arguments
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(6)
            .clamp(1, 12) as usize;
        let project_id = arguments
            .get("project_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let requested_doc_ids = Self::document_lookup_doc_ids_from_arguments(arguments)?;
        let storage = self.runtime_storage()?;
        let mut docs = storage.list_documents_for_search(project_id).await?;
        if !requested_doc_ids.is_empty() {
            docs.retain(|doc| requested_doc_ids.contains(&doc.id));
        }
        let matches = crate::core::knowledge::document_search::search_document_models(
            &storage,
            self.embedding_client.as_deref(),
            query,
            limit,
            docs,
        )
        .await?
        .into_iter()
        .map(Self::document_lookup_hit_json)
        .collect::<Vec<_>>();
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "query": query,
            "retrieval": {
                "mode": if self.embedding_client.is_some() {
                    "document_chunks_with_dense_similarity"
                } else {
                    "document_chunks_lexical"
                },
                "embedding_available": self.embedding_client.is_some(),
                "max_results": limit,
                "scoped_doc_ids": requested_doc_ids.iter().cloned().collect::<Vec<_>>(),
            },
            "results": matches,
        }))?)
    }

    pub(in crate::runtime) fn agentark_capability_overview_result(
        actions: &[ActionDef],
    ) -> serde_json::Value {
        let mut capability_counts = std::collections::BTreeMap::<String, usize>::new();
        let mut integration_counts = std::collections::BTreeMap::<String, usize>::new();
        let mut source_counts = std::collections::BTreeMap::<String, usize>::new();
        for action in actions {
            if action.capabilities.is_empty() {
                *capability_counts
                    .entry("uncategorized".to_string())
                    .or_default() += 1;
            } else {
                for capability in &action.capabilities {
                    let capability = capability.trim();
                    if !capability.is_empty() {
                        *capability_counts.entry(capability.to_string()).or_default() += 1;
                    }
                }
            }
            let metadata = action.action_metadata();
            *integration_counts
                .entry(format!("{:?}", metadata.integration_class))
                .or_default() += 1;
            *source_counts
                .entry(format!("{:?}", action.source))
                .or_default() += 1;
        }
        let top_capabilities = capability_counts
            .iter()
            .rev()
            .take(48)
            .map(|(name, count)| format!("{name} ({count})"))
            .collect::<Vec<_>>();
        serde_json::json!({
            "source": crate::core::knowledge::agentark_knowledge::RUNTIME_SOURCE,
            "result_type": "live_capability_registry_overview",
            "title": "Live AgentArk capability registry",
            "content": Self::compact_text(
                &format!(
                    "Current enabled action count: {}. Capability groups: {}. Integration classes: {:?}. Action sources: {:?}. This live registry is authoritative for current availability. Every live_action result returned by this lookup is already enabled for this runtime; if it has auth metadata, that credential/config requirement is satisfied for the enabled action. AgentArk manual documents are supplemental context.",
                    actions.len(),
                    if top_capabilities.is_empty() { "none".to_string() } else { top_capabilities.join(", ") },
                    integration_counts,
                    source_counts,
                ),
                1800,
            ),
            "action_count": actions.len(),
            "capability_counts": capability_counts,
            "integration_class_counts": integration_counts,
            "action_source_counts": source_counts,
            "score": 1.0,
            "match_reason": "live_registry_overview",
        })
    }

    pub(in crate::runtime) fn credential_state_for_enabled_action(
        action: &ActionDef,
    ) -> &'static str {
        if action.authorization.requires_auth || action.action_metadata().requires_auth {
            "auth_config_satisfied"
        } else {
            "not_required"
        }
    }

    pub(in crate::runtime) fn agentark_capability_live_availability_result(
        scored: &[(&ActionDef, usize, f64)],
        limit: usize,
    ) -> Option<serde_json::Value> {
        let ready_actions = scored
            .iter()
            .filter(|(_, raw_score, _)| *raw_score > 0)
            .take(limit.max(1))
            .map(|(action, _, score)| {
                let metadata = action.action_metadata();
                serde_json::json!({
                    "action_name": action.name,
                    "description": action.description,
                    "capabilities": action.capabilities,
                    "role": metadata.role,
                    "integration_class": metadata.integration_class,
                    "side_effect_level": metadata.side_effect_level,
                    "credential_state": Self::credential_state_for_enabled_action(action),
                    "ready_for_agent": true,
                    "score": score,
                })
            })
            .collect::<Vec<_>>();
        if ready_actions.is_empty() {
            return None;
        }
        let summary = ready_actions
            .iter()
            .filter_map(|item| item.get("action_name").and_then(|value| value.as_str()))
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ");
        Some(serde_json::json!({
            "source": crate::core::knowledge::agentark_knowledge::RUNTIME_SOURCE,
            "result_type": "live_availability_summary",
            "title": "Live matching actions ready now",
            "content": format!(
                "Ready live action matches for this query: {}. These actions are present in the enabled runtime action catalog and are available to the agent now. For any listed action with auth metadata, credential_state=auth_config_satisfied means the connection/config requirement is already satisfied for this runtime. Supplemental manual setup docs do not override this live availability result.",
                summary
            ),
            "ready_for_agent": true,
            "credential_state_scope": "enabled_runtime_actions",
            "matched_actions": ready_actions,
            "score": 1.0,
            "match_reason": "live_enabled_action_availability",
        }))
    }

    pub(in crate::runtime) fn agentark_capability_action_result(
        action: &ActionDef,
        score: f64,
        match_reason: &str,
    ) -> serde_json::Value {
        let metadata = action.action_metadata();
        let caps = if action.capabilities.is_empty() {
            "none".to_string()
        } else {
            action.capabilities.join(", ")
        };
        let content = format!(
            "`{}` | availability: ready_for_agent_now | credential_state: {} | capabilities: {} | source: {:?} | role: {:?} | integration: {:?} | delivery: {:?} | side_effect: {:?} | auth_metadata_present: {} | {}",
            action.name,
            Self::credential_state_for_enabled_action(action),
            caps,
            action.source,
            metadata.role,
            metadata.integration_class,
            metadata.delivery_mode,
            metadata.side_effect_level,
            metadata.requires_auth || action.authorization.requires_auth,
            action.description
        );
        serde_json::json!({
            "source": crate::core::knowledge::agentark_knowledge::RUNTIME_SOURCE,
            "result_type": "live_action",
            "title": action.name.clone(),
            "action_name": action.name.clone(),
            "description": action.description.clone(),
            "version": action.version.clone(),
            "capabilities": action.capabilities.clone(),
            "action_source": action.source.clone(),
            "action_metadata": metadata.clone(),
            "availability": {
                "ready_for_agent": true,
                "source": "enabled_runtime_action_catalog",
                "credential_state": Self::credential_state_for_enabled_action(action),
            },
            "authorization": {
                "requires_auth": action.authorization.requires_auth,
                "risk_level": action.authorization.risk_level.clone(),
                "human_approval": action.authorization.human_approval.clone(),
                "access": action.authorization.access.clone(),
            },
            "content": Self::compact_text(&content, 1800),
            "score": score,
            "match_reason": match_reason,
        })
    }

    pub(in crate::runtime) fn agentark_capability_registry_results(
        actions: &[ActionDef],
        query: &str,
        limit: usize,
    ) -> Vec<serde_json::Value> {
        if limit == 0 {
            return Vec::new();
        }
        let terms = Self::agentark_knowledge_query_terms(query);
        let mut scored = actions
            .iter()
            .map(|action| {
                let metadata = action.action_metadata();
                let searchable = format!(
                    "{}\n{}\n{}\n{:?}\n{:?}\n{:?}\n{:?}",
                    action.name,
                    action.description,
                    action.capabilities.join("\n"),
                    action.source,
                    metadata.role,
                    metadata.integration_class,
                    metadata.delivery_mode
                );
                let raw_score = Self::agentark_knowledge_match_score(&searchable, &terms);
                let score = if terms.is_empty() {
                    1.0
                } else {
                    raw_score as f64 / terms.len().max(1) as f64
                };
                (action, raw_score, score)
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right
                .2
                .partial_cmp(&left.2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.0.name.cmp(&right.0.name))
        });

        let mut results = vec![Self::agentark_capability_overview_result(actions)];
        if let Some(availability) =
            Self::agentark_capability_live_availability_result(&scored, limit)
        {
            results.push(availability);
        }
        results.extend(
            scored
                .into_iter()
                .filter(|(_, raw_score, _)| terms.is_empty() || *raw_score > 0)
                .take(limit.saturating_sub(1))
                .map(|(action, raw_score, score)| {
                    let reason = if terms.is_empty() {
                        "live_registry_default"
                    } else if raw_score > 0 {
                        "live_registry_lexical_match"
                    } else {
                        "live_registry_context"
                    };
                    Self::agentark_capability_action_result(action, score, reason)
                }),
        );
        results
    }

    pub(in crate::runtime) fn agentark_knowledge_fallback_results(
        actions: &[ActionDef],
        query: &str,
        limit: usize,
        doc_ids: &std::collections::HashSet<String>,
        source_filter: Option<&str>,
    ) -> Vec<serde_json::Value> {
        let terms = Self::agentark_knowledge_query_terms(query);
        let mut scored = crate::core::knowledge::agentark_knowledge::build_seed_agentark_knowledge_documents(actions)
            .into_iter()
            .filter(|doc| doc_ids.is_empty() || doc_ids.contains(&doc.id))
            .filter(|doc| source_filter.map(|source| doc.source == source).unwrap_or(true))
            .flat_map(|doc| {
                let title = doc.title.clone();
                let doc_id = doc.id.clone();
                let url = doc.url.clone();
                let tags = doc.tags.clone();
                let terms_for_doc = terms.clone();
                doc.chunks
                    .into_iter()
                    .enumerate()
                    .map(move |(chunk_index, content)| {
                        let raw_score =
                            Self::agentark_knowledge_match_score(&content, &terms_for_doc);
                        let score = if terms_for_doc.is_empty() {
                            1.0
                        } else {
                            raw_score as f64 / terms_for_doc.len().max(1) as f64
                        };
                        (
                            score,
                            serde_json::json!({
                                "source": Self::agentark_knowledge_chunk_field(&content, "source").unwrap_or_else(|| "agentark_knowledge".to_string()),
                                "result_type": "agentark_knowledge_document",
                                "authority": "supplemental_manual",
                                "availability_authority": "does_not_override_live_registry",
                                "title": title.clone(),
                                "document_id": doc_id.clone(),
                                "chunk_index": chunk_index,
                                "content": Self::compact_text(&content, 1800),
                                "score": score,
                                "lexical_score": score,
                                "dense_score": serde_json::Value::Null,
                                "match_reason": "lexical_fallback",
                                "url": url.clone(),
                                "tags": tags.clone(),
                            }),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|(score, _)| *score > 0.0 || terms.is_empty())
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right
                .0
                .partial_cmp(&left.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored
            .into_iter()
            .take(limit)
            .map(|(_, value)| value)
            .collect()
    }

    pub(in crate::runtime) fn agentark_knowledge_doc_ids_from_arguments(
        arguments: &serde_json::Value,
    ) -> Result<std::collections::HashSet<String>> {
        let mut doc_ids = std::collections::HashSet::new();
        let Some(items) = arguments.get("doc_ids") else {
            return Ok(doc_ids);
        };
        let Some(items) = items.as_array() else {
            anyhow::bail!("agentark_capability_lookup doc_ids must be an array when supplied");
        };
        for item in items {
            let Some(raw) = item
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            if !crate::core::knowledge::agentark_knowledge::is_agentark_knowledge_document_id(raw)
                || !raw
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '-' | '_'))
            {
                anyhow::bail!(
                    "agentark_capability_lookup doc_ids may only contain AgentArk knowledge document IDs"
                );
            }
            doc_ids.insert(raw.to_string());
            if doc_ids.len() >= 16 {
                break;
            }
        }
        Ok(doc_ids)
    }

    pub(in crate::runtime) async fn execute_agentark_capability_lookup(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let query = arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("agentark_capability_lookup requires a non-empty query")
            })?;
        let limit = arguments
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(4)
            .clamp(1, 8) as usize;
        let requested_doc_ids = Self::agentark_knowledge_doc_ids_from_arguments(arguments)?;
        let actions = self.list_enabled_actions().await?;
        let registry_results = Self::agentark_capability_registry_results(&actions, query, limit);
        let embedding_available = self.embedding_client.is_some();
        let (mode, supplemental_results) = match self.runtime_storage() {
            Ok(storage) => {
                let mut agentark_docs = match storage
                    .list_documents_by_id_prefix(
                        crate::core::knowledge::agentark_knowledge::DOCUMENT_ID_PREFIX,
                        512,
                    )
                    .await
                {
                    Ok(docs) => docs,
                    Err(error) => {
                        tracing::warn!(
                            error = %error,
                            "AgentArk manual document lookup failed; returning live registry only"
                        );
                        Vec::new()
                    }
                };
                if !requested_doc_ids.is_empty() {
                    agentark_docs.retain(|doc| requested_doc_ids.contains(&doc.id));
                }
                let semantic_results = if agentark_docs.is_empty() {
                    Vec::new()
                } else {
                    match crate::core::knowledge::document_search::search_document_models(
                        &storage,
                        self.embedding_client.as_deref(),
                        query,
                        limit.saturating_mul(4).max(limit),
                        agentark_docs,
                    )
                    .await
                    {
                        Ok(hits) => {
                            hits.into_iter()
                                .map(Self::agentark_knowledge_hit_json)
                                .filter(|hit| {
                                    hit.get("source").and_then(|value| value.as_str())
                                == Some(crate::core::knowledge::agentark_knowledge::CURATED_SOURCE)
                                })
                                .take(limit)
                                .collect::<Vec<_>>()
                        }
                        Err(error) => {
                            tracing::warn!(
                                error = %error,
                                "AgentArk manual search failed; falling back to generated manual text"
                            );
                            Vec::new()
                        }
                    }
                };
                if semantic_results.is_empty() {
                    (
                        "live_registry_with_bounded_lexical_manual_fallback",
                        Self::agentark_knowledge_fallback_results(
                            &actions,
                            query,
                            limit,
                            &requested_doc_ids,
                            Some(crate::core::knowledge::agentark_knowledge::CURATED_SOURCE),
                        ),
                    )
                } else {
                    (
                        if requested_doc_ids.is_empty() {
                            "live_registry_with_pgvector_manual_chunks"
                        } else {
                            "live_registry_with_pgvector_scoped_manual_chunks"
                        },
                        semantic_results,
                    )
                }
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "AgentArk manual storage unavailable; returning live registry only"
                );
                ("live_registry_only", Vec::new())
            }
        };
        let mut matches = registry_results;
        matches.extend(supplemental_results);
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "query": query,
            "retrieval": {
                "mode": mode,
                "embedding_available": embedding_available,
                "result_scope": "agentark_capability_registry_and_manual",
                "max_results_per_source": limit,
                "authoritative_source": crate::core::knowledge::agentark_knowledge::RUNTIME_SOURCE,
                "supplemental_source": crate::core::knowledge::agentark_knowledge::CURATED_SOURCE,
                "capability_registry_action_count": actions.len(),
                "scoped_doc_ids": requested_doc_ids.iter().cloned().collect::<Vec<_>>(),
            },
            "results": matches,
        }))?)
    }

    pub(in crate::runtime) fn memory_lookup_terms(query: &str) -> Vec<String> {
        let mut seen = std::collections::BTreeSet::new();
        query
            .split(|ch: char| !ch.is_alphanumeric())
            .filter_map(|term| {
                let term = term.trim().to_ascii_lowercase();
                if term.chars().count() < 2 || !seen.insert(term.clone()) {
                    None
                } else {
                    Some(term)
                }
            })
            .collect()
    }

    pub(in crate::runtime) fn memory_lookup_score<'a>(
        terms: &[String],
        weighted_fields: impl IntoIterator<Item = (&'a str, f32)>,
    ) -> f32 {
        if terms.is_empty() {
            return 0.0;
        }
        let fields = weighted_fields
            .into_iter()
            .map(|(value, weight)| (value.to_ascii_lowercase(), weight))
            .collect::<Vec<_>>();
        let mut score = 0.0f32;
        for term in terms {
            for (field, weight) in &fields {
                if field.contains(term) {
                    score += *weight;
                }
            }
        }
        score
    }

    pub(in crate::runtime) fn memory_lookup_include_sensitive_experience_item(
        item: &crate::storage::experience_item::Model,
    ) -> bool {
        let sensitivity = item
            .metadata
            .get("sensitivity")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .replace(['-', ' '], "_");
        !matches!(sensitivity.as_str(), "sensitive" | "crisis_sensitive")
    }

    pub(in crate::runtime) fn memory_lookup_experience_json(
        item: crate::storage::experience_item::Model,
    ) -> serde_json::Value {
        let key = item
            .metadata
            .get("key")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let memory_kind = item
            .metadata
            .get("memory_kind")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        serde_json::json!({
            "id": item.id,
            "kind": item.kind,
            "scope": item.scope,
            "project_id": item.project_id,
            "conversation_id": item.conversation_id,
            "title": Self::compact_text(&item.title, 180),
            "content": Self::compact_text(&item.content, 420),
            "key": key,
            "memory_kind": memory_kind,
            "confidence": item.confidence,
            "support_count": item.support_count,
            "updated_at": item.updated_at,
        })
    }

    pub(in crate::runtime) async fn execute_memory_lookup(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let query = arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("memory_lookup requires a non-empty query"))?;
        let limit = arguments
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(5)
            .clamp(1, 12) as usize;
        let include_semantic = arguments
            .get("include_semantic")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let include_structured = arguments
            .get("include_structured")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let include_procedures = arguments
            .get("include_procedures")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let include_lessons = arguments
            .get("include_lessons")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let project_id = arguments
            .get("project_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let conversation_id = arguments
            .get("conversation_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let storage = self.runtime_storage()?;
        let terms = Self::memory_lookup_terms(query);
        let semantic_kinds = if include_lessons {
            vec![
                "identity",
                "preference",
                "location",
                "workflow",
                "constraint",
                "personal_fact",
                "other",
                "lesson",
                "procedure",
            ]
        } else {
            vec![
                "identity",
                "preference",
                "location",
                "workflow",
                "constraint",
                "personal_fact",
                "other",
            ]
        };

        let semantic_facts = if include_semantic {
            let mut scored = storage
                .list_active_experience_items(&semantic_kinds, project_id, conversation_id, 96)
                .await?
                .into_iter()
                .filter(Self::memory_lookup_include_sensitive_experience_item)
                .map(|item| {
                    let key = item
                        .metadata
                        .get("key")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let memory_kind = item
                        .metadata
                        .get("memory_kind")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let score = Self::memory_lookup_score(
                        &terms,
                        [
                            (item.kind.as_str(), 2.0),
                            (key.as_str(), 2.0),
                            (memory_kind.as_str(), 2.0),
                            (item.title.as_str(), 1.5),
                            (item.content.as_str(), 1.0),
                            (item.normalized_key.as_str(), 1.0),
                        ],
                    );
                    (score, item)
                })
                .collect::<Vec<_>>();
            scored.sort_by(|left, right| {
                right
                    .0
                    .partial_cmp(&left.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| right.1.updated_at.cmp(&left.1.updated_at))
            });
            scored
                .into_iter()
                .filter(|(score, _)| *score > 0.0 || terms.is_empty())
                .take(limit)
                .map(|(_, item)| Self::memory_lookup_experience_json(item))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let preferences = if include_structured {
            let mut scored = storage
                .list_user_preferences(48, 0, project_id)
                .await?
                .into_iter()
                .map(|item| {
                    let score = Self::memory_lookup_score(
                        &terms,
                        [(item.key.as_str(), 2.0), (item.value.as_str(), 1.0)],
                    );
                    (score, item)
                })
                .collect::<Vec<_>>();
            scored.sort_by(|left, right| {
                right
                    .0
                    .partial_cmp(&left.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| right.1.updated_at.cmp(&left.1.updated_at))
            });
            scored
                .into_iter()
                .filter(|(score, _)| *score > 0.0 || terms.is_empty())
                .take(limit)
                .map(|(_, item)| {
                    serde_json::json!({
                        "id": item.id,
                        "key": item.key,
                        "value": Self::compact_text(&item.value, 320),
                        "sensitivity": item.sensitivity,
                        "confidence": item.confidence,
                        "project_id": item.project_id,
                        "updated_at": item.updated_at,
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let user_data = if include_structured {
            let mut scored = storage
                .list_user_data_items(48, 0, project_id, None)
                .await?
                .into_iter()
                .map(|item| {
                    let url = item.url.clone().unwrap_or_default();
                    let score = Self::memory_lookup_score(
                        &terms,
                        [
                            (item.kind.as_str(), 2.0),
                            (item.title.as_str(), 1.5),
                            (item.content.as_str(), 1.0),
                            (url.as_str(), 0.5),
                        ],
                    );
                    (score, item)
                })
                .collect::<Vec<_>>();
            scored.sort_by(|left, right| {
                right
                    .0
                    .partial_cmp(&left.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| right.1.updated_at.cmp(&left.1.updated_at))
            });
            scored
                .into_iter()
                .filter(|(score, _)| *score > 0.0 || terms.is_empty())
                .take(limit)
                .map(|(_, item)| {
                    serde_json::json!({
                        "id": item.id,
                        "kind": item.kind,
                        "title": Self::compact_text(&item.title, 180),
                        "content": Self::compact_text(&item.content, 320),
                        "url": item.url,
                        "pinned": item.pinned,
                        "project_id": item.project_id,
                        "conversation_id": item.conversation_id,
                        "updated_at": item.updated_at,
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let knowledge = if include_structured {
            let mut scored = storage
                .list_visible_knowledge_items(48, 0, project_id)
                .await?
                .into_iter()
                .map(|item| {
                    let tags = item.tags.clone().unwrap_or_default();
                    let source = item.source.clone().unwrap_or_default();
                    let score = Self::memory_lookup_score(
                        &terms,
                        [
                            (item.title.as_str(), 1.5),
                            (item.content.as_str(), 1.0),
                            (tags.as_str(), 1.0),
                            (source.as_str(), 0.5),
                        ],
                    );
                    (score, item)
                })
                .collect::<Vec<_>>();
            scored.sort_by(|left, right| {
                right
                    .0
                    .partial_cmp(&left.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| right.1.updated_at.cmp(&left.1.updated_at))
            });
            scored
                .into_iter()
                .filter(|(score, _)| *score > 0.0 || terms.is_empty())
                .take(limit)
                .map(|(_, item)| {
                    serde_json::json!({
                        "id": item.id,
                        "title": Self::compact_text(&item.title, 180),
                        "content": Self::compact_text(&item.content, 420),
                        "source": item.source,
                        "url": item.url,
                        "tags": item.tags,
                        "project_id": item.project_id,
                        "updated_at": item.updated_at,
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let procedures = if include_procedures {
            storage
                .search_procedural_patterns(query, project_id, conversation_id, limit as u64)
                .await?
                .into_iter()
                .map(|hit| {
                    serde_json::json!({
                        "id": hit.pattern.id,
                        "status": hit.pattern.status,
                        "title": Self::compact_text(&hit.pattern.title, 180),
                        "trigger_summary": Self::compact_text(&hit.pattern.trigger_summary, 260),
                        "summary": Self::compact_text(&hit.pattern.summary, 420),
                        "score": hit.score,
                        "sample_count": hit.pattern.sample_count,
                        "success_rate": hit.pattern.success_rate,
                        "updated_at": hit.pattern.updated_at,
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "query": query,
            "retrieval": {
                "mode": "bounded_local_memory",
                "max_results_per_bucket": limit,
                "include_semantic": include_semantic,
                "include_structured": include_structured,
                "include_procedures": include_procedures,
                "include_lessons": include_lessons,
                "scope": {
                    "project_id": project_id,
                    "conversation_id": conversation_id,
                },
            },
            "results": {
                "semantic_facts": semantic_facts,
                "preferences": preferences,
                "user_data": user_data,
                "knowledge": knowledge,
                "procedures": procedures,
            },
        }))?)
    }

    pub(in crate::runtime) async fn execute_ark_inspect(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        ark_inspect::execute(self, arguments).await
    }

    pub(in crate::runtime) async fn execute_postgres_schema_inspect(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let storage = self.runtime_storage()?;
        let payload = storage
            .inspect_postgres_schema_json(
                arguments
                    .get("table_filter")
                    .and_then(|value| value.as_str()),
                arguments
                    .get("limit")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(25),
            )
            .await?;
        Ok(serde_json::to_string_pretty(&payload)?)
    }

    pub(in crate::runtime) async fn execute_postgres_query_readonly(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let storage = self.runtime_storage()?;
        let request: crate::storage::ReadonlyTableQuery = serde_json::from_value(arguments.clone())
            .map_err(|error| {
                anyhow::anyhow!("Invalid structured database query arguments: {}", error)
            })?;
        let payload = storage.query_table_json(&request).await.map_err(|error| {
            anyhow::anyhow!(
                "{}. Inspect the live schema with postgres_schema_inspect and retry with corrected table or column names.",
                error
            )
        })?;
        Ok(serde_json::to_string_pretty(&payload)?)
    }
}
