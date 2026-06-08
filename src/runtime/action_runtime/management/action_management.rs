use super::super::*;

impl ActionRuntime {
    /// List available actions
    pub async fn list_actions(&self) -> Result<Vec<ActionDef>> {
        let actions = self.actions.read().await;
        Ok(actions.values().map(|s| s.info.clone()).collect())
    }

    pub async fn action_definition(&self, action_name: &str) -> Option<ActionDef> {
        let actions = self.actions.read().await;
        actions.get(action_name).map(|loaded| loaded.info.clone())
    }

    pub async fn list_action_scope_hints(&self) -> Result<HashMap<String, ActionScopeHint>> {
        let actions = self.actions.read().await;
        Ok(actions
            .iter()
            .map(|(name, loaded)| {
                (
                    name.clone(),
                    Self::action_scope_hint_for_loaded_action(name, loaded),
                )
            })
            .collect())
    }

    /// List actions the model may call this turn.
    /// System actions stay visible unless explicitly disabled; auth, bundle,
    /// integration, and policy preconditions are surfaced by the action at
    /// execution time as structured errors.
    pub async fn list_enabled_actions(&self) -> Result<Vec<ActionDef>> {
        let disabled = self.disabled_actions.read().await.clone();
        let actions = self
            .actions
            .read()
            .await
            .values()
            .map(|loaded| loaded.info.clone())
            .collect::<Vec<_>>();
        let mut enabled = Vec::new();
        for action in actions {
            if disabled.contains(action.name.as_str()) {
                continue;
            }
            if toolsets::default_hidden_action(action.name.as_str()) {
                continue;
            }
            if action
                .capabilities
                .iter()
                .any(|capability| capability.eq_ignore_ascii_case("custom_api"))
            {
                if let Some(review) = self.refresh_action_review_state(&action.name).await? {
                    if review.allow_execute && review.visible_in_catalog {
                        enabled.push(action);
                    }
                }
                continue;
            }
            if action.source == ActionSource::System {
                // Catalog returns all enabled system actions regardless of integration
                // readiness. Each tool surfaces its own preconditions at execution
                // time via structured ActionError envelopes the model relays. Load-time
                // filtering by integration state was the silent failure surface where
                // users saw "scope is empty" without any explanation of why.
                enabled.push(action);
                continue;
            }

            if let Some(review) = self.refresh_action_review_state(&action.name).await? {
                if !review.visible_in_catalog {
                    continue;
                }
            } else {
                continue;
            }
            enabled.push(action);
        }
        Ok(enabled)
    }

    /// Returns true if an action is enabled (not in the disabled set).
    pub async fn is_action_enabled(&self, name: &str) -> bool {
        let action = {
            let actions = self.actions.read().await;
            actions.get(name).map(|loaded| loaded.info.clone())
        };
        let Some(action) = action else {
            return false;
        };
        if action
            .capabilities
            .iter()
            .any(|capability| capability.eq_ignore_ascii_case("custom_api"))
        {
            return match self.refresh_action_review_state(name).await {
                Ok(Some(review)) => review.allow_execute && review.visible_in_catalog,
                Ok(None) => false,
                Err(_) => false,
            };
        }
        if action.source == ActionSource::System {
            // System actions are always enabled at the catalog level. Integration
            // readiness is checked at execution time, where the failure surfaces
            // as a structured ActionError::NotConnected the model can relay.
            return true;
        }

        let disabled = self.disabled_actions.read().await;
        if disabled.contains(name) {
            return false;
        }
        match self.refresh_action_review_state(name).await {
            Ok(Some(review)) => review.allow_execute,
            Ok(None) => false,
            Err(_) => false,
        }
    }

    /// Enable or disable an action without deleting it.
    /// - System actions cannot be disabled.
    pub async fn set_action_enabled(&self, name: &str, enabled: bool) -> Result<bool> {
        let source = {
            let actions = self.actions.read().await;
            match actions.get(name) {
                Some(action) => action.info.source.clone(),
                None => return Ok(false),
            }
        };

        if source == ActionSource::System {
            return Ok(false);
        }

        if enabled {
            match self.refresh_action_review_state(name).await? {
                Some(review) => {
                    if !review.allow_execute {
                        anyhow::bail!(
                            "{}",
                            review.blocked_reason.unwrap_or_else(|| {
                                format!("Action '{}' is not ready to enable.", name)
                            })
                        );
                    }
                }
                None => anyhow::bail!(
                    "Action '{}' has no persisted security review and cannot be enabled.",
                    name
                ),
            }
        }

        {
            let mut disabled = self.disabled_actions.write().await;
            if enabled {
                disabled.remove(name);
            } else {
                disabled.insert(name.to_string());
            }
        }

        self.save_disabled_actions().await?;
        Ok(true)
    }

    /// Get action count
    pub async fn action_count(&self) -> usize {
        self.actions.read().await.len()
    }

    /// Get action info and content for editing
    pub async fn get_action_content(&self, name: &str) -> Result<Option<(ActionDef, String)>> {
        let actions = self.actions.read().await;
        if let Some(action) = actions.get(name) {
            let info = action.info.clone();
            let file_path = action.info.file_path.clone();
            let workflow = action.workflow_content.clone();
            drop(actions); // Release lock before async file I/O

            if let Some(ref fp) = file_path {
                let content = tokio::fs::read_to_string(fp).await?;
                return Ok(Some((info, content)));
            } else if let Some(wf) = workflow {
                return Ok(Some((info, wf)));
            }
            return Ok(Some((info, String::new())));
        }
        Ok(None)
    }

    pub(in crate::runtime) fn preferred_skill_markdown_path(dir: &Path) -> std::path::PathBuf {
        dir.join("SKILL.md")
    }

    /// Update action content - for bundled actions, creates a custom copy first
    pub async fn update_action_content(&self, name: &str, content: &str) -> Result<bool> {
        let (source, file_path) = {
            let actions = self.actions.read().await;
            let Some(action) = actions.get(name) else {
                return Ok(false);
            };
            if action.info.source == ActionSource::System {
                return Ok(false);
            }
            (action.info.source.clone(), action.info.file_path.clone())
        };

        let (action_dir, action_file, action_source) = if source == ActionSource::Bundled {
            let custom_action_dir = self.actions_dir.join(name);
            tokio::fs::create_dir_all(&custom_action_dir).await?;
            (
                custom_action_dir.clone(),
                Self::preferred_skill_markdown_path(&custom_action_dir),
                ActionSource::Custom,
            )
        } else if let Some(file_path) = file_path {
            let action_file = PathBuf::from(file_path);
            let action_dir = action_file
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.actions_dir.join(name));
            (action_dir, action_file, ActionSource::Custom)
        } else {
            return Ok(false);
        };

        tokio::fs::write(&action_file, content).await?;
        if let Some(ref guard) = self.action_guard {
            if let Err(error) = guard.resign_action(&action_dir, name).await {
                tracing::warn!("Failed to re-sign action '{}': {}", name, error);
            }
        }

        let (new_info, new_content, frontmatter) = self
            .parse_action_md(&action_file, action_source.clone())
            .await?;
        let review = self
            .review_markdown_action(&action_dir, &new_info, &new_content, &frontmatter)
            .await?;

        {
            let mut actions = self.actions.write().await;
            if let Some(action) = actions.get_mut(name) {
                action.info = new_info;
                action.info.source = action_source;
                action.info.file_path = Some(action_file.to_string_lossy().to_string());
                action.workflow_content = Some(new_content);
            }
        }

        self.upsert_action_review(review.clone()).await?;
        if review.allow_execute {
            let mut disabled = self.disabled_actions.write().await;
            if disabled.remove(name) {
                drop(disabled);
                self.save_disabled_actions().await?;
            }
        } else {
            let mut disabled = self.disabled_actions.write().await;
            if disabled.insert(name.to_string()) {
                drop(disabled);
                self.save_disabled_actions().await?;
            }
        }

        tracing::info!(
            "Updated action '{}' and refreshed security review state",
            name
        );
        Ok(true)
    }

    /// Create a new custom action with security verification
    /// Returns the security verdict so the caller can present it to the user.
    /// `force` can keep non-blocking warnings visible, but semantic/security
    /// blocks are never overridden.
    pub async fn create_action(
        &self,
        name: &str,
        content: &str,
        _force: bool,
    ) -> Result<Option<crate::security::action_guard::ActionSecurityVerdict>> {
        let guard = self.action_guard.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Action security is unavailable, so importing new skills is disabled.")
        })?;
        let action_dir = self.actions_dir.join(name);
        tokio::fs::create_dir_all(&action_dir).await?;

        let action_file = Self::preferred_skill_markdown_path(&action_dir);
        tokio::fs::write(&action_file, content).await?;

        // Sign the new action manifest
        if let Err(e) = guard.resign_action(&action_dir, name).await {
            tracing::warn!("Failed to sign new action '{}': {}", name, e);
        }

        let (info, workflow_content, frontmatter) = self
            .parse_action_md(&action_file, ActionSource::Custom)
            .await
            .map_err(|error| anyhow::anyhow!("Failed to parse action: {}", error))?;
        let verdict = guard
            .evaluate_action(&action_dir, name, &workflow_content, &frontmatter)
            .await?;
        let required_env = Self::extract_required_envs_from_frontmatter(&frontmatter);
        let missing_env = self
            .compute_missing_required_envs(&info.name, &required_env)
            .await?;
        let fingerprint = crate::security::ActionGuard::compute_bundle_hash(&action_dir)
            .unwrap_or_else(|_| Self::fingerprint_text(&[&workflow_content, &frontmatter]));
        let review = Self::build_review_from_verdict(ActionReviewBuildInput {
            action_name: &info.name,
            source_kind: Self::action_source_label(&info.source),
            fingerprint,
            verdict: &verdict,
            required_env,
            missing_env,
            requires_auth: false,
            auth_configured: true,
            notes: Vec::new(),
        });
        let blocked = !verdict.allow_load;

        if blocked {
            tracing::warn!(
                "New action '{}' BLOCKED by security guard: {:?}",
                name,
                verdict.warnings
            );
            let _ = tokio::fs::remove_dir_all(&action_dir).await;
            self.remove_action_review(name).await?;
            return Ok(Some(verdict));
        }

        for warning in &verdict.warnings {
            tracing::warn!("Action '{}': {}", name, warning);
        }
        self.register_workflow_action(info, workflow_content).await;
        self.upsert_action_review(review).await?;
        tracing::info!(
            "Created and registered action '{}' at {:?}",
            name,
            action_file
        );
        Ok(Some(verdict))
    }

    /// Create a custom action after the import path has completed semantic
    /// capability review with the configured model. This signs and persists the
    /// skill, then stores the deterministic policy verdict without reclassifying
    /// the content through wording-based checks.
    pub async fn install_semantically_reviewed_action(
        &self,
        name: &str,
        content: &str,
        semantic_review: &crate::security::skill_review::SemanticSkillReview,
        _force: bool,
    ) -> Result<ActionReviewSnapshot> {
        if semantic_review.policy.blocked {
            anyhow::bail!("Skill '{}' blocked by semantic security policy", name);
        }

        let action_dir = self.actions_dir.join(name);
        tokio::fs::create_dir_all(&action_dir).await?;
        let action_file = Self::preferred_skill_markdown_path(&action_dir);
        tokio::fs::write(&action_file, content).await?;

        let (info, workflow_content, frontmatter) = self
            .parse_action_md(&action_file, ActionSource::Custom)
            .await
            .map_err(|error| anyhow::anyhow!("Failed to parse action: {}", error))?;
        let Some(ref guard) = self.action_guard else {
            anyhow::bail!("Action security is unavailable, so importing new skills is disabled.");
        };
        guard
            .resign_action(&action_dir, name)
            .await
            .with_context(|| format!("Failed to sign semantically reviewed skill '{}'", name))?;
        let integrity_ok = true;

        let required_env = Self::extract_required_envs_from_frontmatter(&frontmatter);
        let missing_env = self
            .compute_missing_required_envs(&info.name, &required_env)
            .await?;
        let fingerprint = crate::security::ActionGuard::compute_bundle_hash(&action_dir)
            .unwrap_or_else(|_| Self::fingerprint_text(&[&workflow_content, &frontmatter]));

        let mut notes = Vec::new();
        notes.push(format!(
            "Semantic capability review used configured model '{}'.",
            semantic_review.model
        ));
        if !semantic_review.summary.trim().is_empty() {
            notes.push(format!(
                "Semantic summary: {}",
                semantic_review.summary.trim()
            ));
        }
        let capabilities = semantic_review
            .capabilities
            .iter()
            .map(|capability| {
                if let Some(target) = capability.target.as_deref() {
                    format!("{}:{}", capability.normalized_kind(), target)
                } else {
                    capability.normalized_kind()
                }
            })
            .collect::<Vec<_>>();
        if !capabilities.is_empty() {
            notes.push(format!("Capabilities: {}.", capabilities.join(", ")));
        }
        for rule in &semantic_review.policy.matched_rules {
            notes.push(format!("Policy rule '{}': {}", rule.id, rule.message));
        }

        let warnings = semantic_review.policy.warnings.clone();
        let allow_load = !semantic_review.policy.blocked;
        let status = if !missing_env.is_empty() {
            ActionReviewStatus::NeedsSecrets
        } else if !warnings.is_empty()
            || semantic_review.policy.risk_band == "review"
            || semantic_review.policy.risk_band == "risky"
        {
            ActionReviewStatus::Warning
        } else {
            ActionReviewStatus::Ready
        };

        let allow_execute = matches!(
            status,
            ActionReviewStatus::Ready | ActionReviewStatus::Warning
        );
        let blocked_reason = if semantic_review.policy.blocked {
            semantic_review
                .policy
                .warnings
                .first()
                .cloned()
                .or_else(|| Some("Blocked by semantic skill security policy.".to_string()))
        } else if !missing_env.is_empty() {
            Some(format!(
                "Required secrets missing: {}",
                missing_env.join(", ")
            ))
        } else {
            None
        };

        let review = ActionReviewSnapshot {
            action_name: info.name.clone(),
            source_kind: Self::action_source_label(&info.source).to_string(),
            reviewed_at: chrono::Utc::now().to_rfc3339(),
            fingerprint,
            status,
            ready: allow_execute,
            allow_load,
            allow_execute,
            visible_in_catalog: allow_execute,
            integrity_ok,
            threat_level: format!("{:?}", semantic_review.policy.threat_level),
            total_severity: semantic_review.policy.total_severity,
            total_findings: semantic_review.policy.findings.len(),
            risk_score_10: semantic_review.policy.risk_score_10,
            risk_band: semantic_review.policy.risk_band.clone(),
            warnings,
            findings: semantic_review.policy.findings.clone(),
            required_env,
            missing_env,
            permissions_needed: capabilities,
            requires_auth: false,
            auth_configured: true,
            notes,
            blocked_reason,
        };

        self.register_workflow_action(info, workflow_content).await;
        self.upsert_action_review(review.clone()).await?;
        self.record_action_review_event(&review).await;
        if review.allow_execute {
            let mut disabled = self.disabled_actions.write().await;
            if disabled.remove(name) {
                drop(disabled);
                self.save_disabled_actions().await?;
            }
        } else {
            let mut disabled = self.disabled_actions.write().await;
            if disabled.insert(name.to_string()) {
                drop(disabled);
                self.save_disabled_actions().await?;
            }
        }
        Ok(review)
    }

    pub async fn update_semantically_reviewed_action(
        &self,
        name: &str,
        content: &str,
        semantic_review: &crate::security::skill_review::SemanticSkillReview,
        force: bool,
    ) -> Result<Option<ActionReviewSnapshot>> {
        let editable = {
            let actions = self.actions.read().await;
            let Some(action) = actions.get(name) else {
                return Ok(None);
            };
            action.info.source != ActionSource::System
        };
        if !editable {
            return Ok(None);
        }
        self.install_semantically_reviewed_action(name, content, semantic_review, force)
            .await
            .map(Some)
    }

    pub(in crate::runtime) fn capability_acquire_required_inputs(
        arguments: &serde_json::Value,
    ) -> Vec<String> {
        arguments
            .get("required_inputs")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    pub(in crate::runtime) fn capability_string_argument(
        arguments: &serde_json::Value,
        key: &str,
    ) -> Option<String> {
        arguments
            .get(key)
            .and_then(|value| value.as_str())
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    }

    pub(in crate::runtime) async fn enrich_capability_acquisition_arguments(
        arguments: &serde_json::Value,
    ) -> serde_json::Value {
        let mut enriched = arguments.clone();
        if !enriched.is_object() {
            enriched = serde_json::json!({});
        }
        let Some(root) = enriched.as_object_mut() else {
            return enriched;
        };

        let openapi_url = Self::capability_string_argument(arguments, "openapi_url");
        let docs_url = Self::capability_string_argument(arguments, "docs_url");
        let openapi_text = Self::capability_string_argument(arguments, "openapi_text");
        let docs_text = Self::capability_string_argument(arguments, "docs_text");
        let mut source_evidence = root
            .get("_capability_source_evidence")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        root.remove("_capability_source_evidence");
        if openapi_url.is_some()
            || openapi_text
                .as_ref()
                .is_some_and(|text| !text.trim().is_empty())
        {
            source_evidence.push(serde_json::Value::String("openapi".to_string()));
        }
        if docs_text
            .as_ref()
            .is_some_and(|text| !text.trim().is_empty())
        {
            source_evidence.push(serde_json::Value::String("docs".to_string()));
        }
        if !source_evidence.is_empty() {
            root.insert(
                "_capability_source_evidence".to_string(),
                serde_json::Value::Array(source_evidence),
            );
        }

        if root.get("docs_url").is_none() {
            if let Some(url) = docs_url {
                root.insert("docs_url".to_string(), serde_json::Value::String(url));
            }
        }
        if root.get("openapi_url").is_none() {
            if let Some(url) = openapi_url {
                root.insert("openapi_url".to_string(), serde_json::Value::String(url));
            }
        }

        enriched
    }

    pub(in crate::runtime) fn normalize_generated_action_name(raw: &str) -> String {
        let mut out = String::new();
        let mut prev_dash = false;
        for ch in raw.chars() {
            let mapped = if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            };
            if mapped == '-' {
                if !prev_dash && !out.is_empty() {
                    out.push('-');
                }
                prev_dash = true;
            } else {
                out.push(mapped);
                prev_dash = false;
            }
        }
        out.trim_matches('-').to_string()
    }

    /// Delete/disable an action.
    /// - Custom actions: deleted from disk and runtime.
    /// - Bundled actions: deleted from runtime-owned bundled directories for this install.
    /// - System actions: cannot be deleted/disabled.
    pub async fn delete_action(&self, name: &str) -> Result<bool> {
        let (source, file_path) = {
            let actions = self.actions.read().await;
            match actions.get(name) {
                Some(action) => (action.info.source.clone(), action.info.file_path.clone()),
                None => return Ok(false),
            }
        };
        tracing::info!(
            action = name,
            source = ?source,
            has_file_path = file_path.is_some(),
            "Runtime delete_action resolved action"
        );

        match source {
            ActionSource::System => {
                tracing::info!(action = name, "Runtime delete_action refused system action");
                Ok(false)
            }
            ActionSource::Bundled => {
                tracing::info!(
                    action = name,
                    "Runtime delete_action deleting bundled action"
                );
                self.delete_runtime_owned_bundled_skill_dir(name).await?;
                {
                    let mut removed = self.removed_bundled_actions.write().await;
                    removed.insert(name.to_string());
                }
                {
                    let mut disabled = self.disabled_actions.write().await;
                    disabled.remove(name);
                }
                self.save_removed_bundled_actions().await?;
                self.save_disabled_actions().await?;
                self.clear_action_secret_bindings(name).await?;
                self.remove_action_review(name).await?;
                let mut actions = self.actions.write().await;
                actions.remove(name);
                tracing::info!("Deleted bundled action '{}' for this install", name);
                Ok(true)
            }
            ActionSource::Custom => {
                tracing::info!(
                    action = name,
                    "Runtime delete_action deleting custom action"
                );
                if let Some(fp) = file_path {
                    let action_path = std::path::Path::new(&fp);
                    if let Some(action_dir) = action_path.parent() {
                        let dir_path = action_dir.to_path_buf();
                        if dir_path.exists() {
                            tracing::info!(
                                action = name,
                                path = %dir_path.display(),
                                "Runtime delete_action removing custom action directory"
                            );
                            tokio::fs::remove_dir_all(&dir_path).await?;
                        }
                    }
                }
                {
                    let mut disabled = self.disabled_actions.write().await;
                    disabled.remove(name);
                }
                self.save_disabled_actions().await?;
                self.clear_action_secret_bindings(name).await?;
                self.remove_action_review(name).await?;
                let mut actions = self.actions.write().await;
                actions.remove(name);
                tracing::info!("Deleted custom action '{}'", name);
                Ok(true)
            }
        }
    }

    /// Check if an action is a workflow action (LLM-driven) and get its workflow content
    /// Returns None if action doesn't exist or has no workflow content
    pub async fn get_workflow_content(&self, action_name: &str) -> Option<String> {
        self.actions
            .read()
            .await
            .get(action_name)
            .and_then(|s| s.workflow_content.clone())
    }
}
