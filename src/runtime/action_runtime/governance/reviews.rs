use super::super::*;

impl ActionRuntime {
    pub(in crate::runtime) fn load_disabled_actions(
        path: &Path,
        settings: Option<&SecureConfigManager>,
    ) -> HashSet<String> {
        if let Some(manager) = settings.filter(|manager| manager.uses_storage_backend()) {
            match manager.load_encrypted_json::<Vec<String>>(
                crate::core::runtime::config::SETTINGS_DISABLED_ACTIONS_KEY,
            ) {
                Ok(Some(entries)) => {
                    return entries
                        .into_iter()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                Ok(None) => return HashSet::new(),
                Err(error) => {
                    tracing::warn!(
                        "Failed to load disabled actions from settings storage: {}",
                        error
                    )
                }
            }
        }

        let raw = match std::fs::read(path) {
            Ok(v) => v,
            Err(_) => return HashSet::new(),
        };
        serde_json::from_slice::<Vec<String>>(&raw)
            .map(|v| {
                v.into_iter()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(in crate::runtime) async fn save_disabled_actions(&self) -> Result<()> {
        let mut list: Vec<String> = self.disabled_actions.read().await.iter().cloned().collect();
        list.sort();
        let manager = self.settings_manager()?;
        if manager.uses_storage_backend() {
            manager.save_encrypted_json(
                crate::core::runtime::config::SETTINGS_DISABLED_ACTIONS_KEY,
                &list,
            )?;
        } else {
            let raw = serde_json::to_vec_pretty(&list)?;
            tokio::fs::write(&self.disabled_actions_file, raw).await?;
        }
        Ok(())
    }

    pub(in crate::runtime) fn load_action_reviews(
        path: &Path,
        settings: Option<&SecureConfigManager>,
    ) -> HashMap<String, ActionReviewRecord> {
        if let Some(manager) = settings.filter(|manager| manager.uses_storage_backend()) {
            match manager.load_encrypted_json::<HashMap<String, ActionReviewRecord>>(
                crate::core::runtime::config::SETTINGS_ACTION_REVIEWS_KEY,
            ) {
                Ok(Some(reviews)) => return reviews,
                Ok(None) => return HashMap::new(),
                Err(error) => {
                    tracing::warn!(
                        "Failed to load action reviews from settings storage: {}",
                        error
                    )
                }
            }
        }

        let raw = match std::fs::read(path) {
            Ok(v) => v,
            Err(_) => return HashMap::new(),
        };
        serde_json::from_slice::<HashMap<String, ActionReviewRecord>>(&raw).unwrap_or_default()
    }

    pub(in crate::runtime) async fn save_action_reviews(&self) -> Result<()> {
        let reviews = self.action_reviews.read().await.clone();
        let manager = self.settings_manager()?;
        if manager.uses_storage_backend() {
            manager.save_encrypted_json(
                crate::core::runtime::config::SETTINGS_ACTION_REVIEWS_KEY,
                &reviews,
            )?;
        } else {
            let raw = serde_json::to_vec_pretty(&reviews)?;
            tokio::fs::write(&self.action_reviews_file, raw).await?;
        }
        Ok(())
    }

    pub(in crate::runtime) async fn upsert_action_review(
        &self,
        review: ActionReviewSnapshot,
    ) -> Result<()> {
        let mut reviews = self.action_reviews.write().await;
        let entry = reviews
            .entry(review.action_name.clone())
            .or_insert_with(ActionReviewRecord::default);
        let changed = entry.current.action_name.is_empty()
            || entry.current.fingerprint != review.fingerprint
            || entry.current.status != review.status
            || entry.current.blocked_reason != review.blocked_reason
            || entry.current.missing_env != review.missing_env
            || entry.current.auth_configured != review.auth_configured
            || entry.current.allow_execute != review.allow_execute
            || entry.current.permissions_needed != review.permissions_needed
            || entry.current.warnings != review.warnings
            || (entry.current.risk_score_10 - review.risk_score_10).abs() > f32::EPSILON;
        if changed && !entry.current.action_name.is_empty() {
            entry.history.push(entry.current.clone());
            if entry.history.len() > ACTION_REVIEW_HISTORY_LIMIT {
                let drop_count = entry.history.len() - ACTION_REVIEW_HISTORY_LIMIT;
                entry.history.drain(0..drop_count);
            }
        }
        entry.current = review;
        drop(reviews);
        self.save_action_reviews().await?;
        if changed {
            self.record_cross_layer_capability_correlation().await;
        }
        Ok(())
    }

    pub(in crate::runtime) async fn remove_action_review(&self, name: &str) -> Result<()> {
        let mut reviews = self.action_reviews.write().await;
        reviews.remove(name);
        drop(reviews);
        self.save_action_reviews().await
    }

    pub(in crate::runtime) async fn clear_action_secret_bindings(
        &self,
        action_name: &str,
    ) -> Result<()> {
        let manager =
            SecureConfigManager::new_with_data_dir(&self.config_dir, Some(self.data_dir()))?;
        let prefix = format!("action_envmap:{}:", action_name);
        manager.update_custom_secrets(|custom| {
            custom.retain(|key, _| !key.starts_with(&prefix));
            Ok(())
        })?;
        Ok(())
    }

    pub async fn clear_action_secret_bindings_for_actions(
        &self,
        action_names: &[String],
    ) -> Result<()> {
        for action_name in action_names {
            self.clear_action_secret_bindings(action_name).await?;
        }
        Ok(())
    }

    pub(in crate::runtime) async fn remove_action_reviews<F>(
        &self,
        mut predicate: F,
    ) -> Result<usize>
    where
        F: FnMut(&str) -> bool,
    {
        let mut reviews = self.action_reviews.write().await;
        let before = reviews.len();
        reviews.retain(|name, _| !predicate(name));
        let removed = before.saturating_sub(reviews.len());
        drop(reviews);
        if removed > 0 {
            self.save_action_reviews().await?;
        }
        Ok(removed)
    }

    pub async fn get_action_review(&self, name: &str) -> Option<ActionReviewSnapshot> {
        self.action_reviews
            .read()
            .await
            .get(name)
            .map(|record| record.current.clone())
    }

    pub(in crate::runtime) fn load_removed_bundled_actions(
        path: &Path,
        settings: Option<&SecureConfigManager>,
    ) -> HashSet<String> {
        if let Some(manager) = settings.filter(|manager| manager.uses_storage_backend()) {
            match manager.load_encrypted_json::<Vec<String>>(
                crate::core::runtime::config::SETTINGS_REMOVED_BUNDLED_ACTIONS_KEY,
            ) {
                Ok(Some(entries)) => {
                    return entries
                        .into_iter()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                Ok(None) => return HashSet::new(),
                Err(error) => tracing::warn!(
                    "Failed to load removed bundled actions from settings storage: {}",
                    error
                ),
            }
        }

        let raw = match std::fs::read(path) {
            Ok(v) => v,
            Err(_) => return HashSet::new(),
        };
        serde_json::from_slice::<Vec<String>>(&raw)
            .map(|v| {
                v.into_iter()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(in crate::runtime) async fn save_removed_bundled_actions(&self) -> Result<()> {
        let mut list: Vec<String> = self
            .removed_bundled_actions
            .read()
            .await
            .iter()
            .cloned()
            .collect();
        list.sort();
        let manager = self.settings_manager()?;
        if manager.uses_storage_backend() {
            manager.save_encrypted_json(
                crate::core::runtime::config::SETTINGS_REMOVED_BUNDLED_ACTIONS_KEY,
                &list,
            )?;
        } else {
            let raw = serde_json::to_vec_pretty(&list)?;
            tokio::fs::write(&self.removed_bundled_actions_file, raw).await?;
        }
        Ok(())
    }

    /// Get the data directory (parent of actions_dir)
    pub(in crate::runtime) fn data_dir(&self) -> &Path {
        self.actions_dir.parent().unwrap_or(&self.actions_dir)
    }

    pub(in crate::runtime) fn default_workspace_root_for_data_dir(data_dir: &Path) -> PathBuf {
        data_dir.join("workspace")
    }

    pub(in crate::runtime) fn workspace_root_from_config(
        data_dir: &Path,
        configured: Option<&str>,
    ) -> PathBuf {
        let fallback = Self::default_workspace_root_for_data_dir(data_dir);
        let Some(configured) = configured.map(str::trim).filter(|value| !value.is_empty()) else {
            return fallback;
        };

        let candidate = {
            let path = PathBuf::from(configured);
            if path.is_absolute() {
                path
            } else {
                data_dir.join(path)
            }
        };

        if data_dir_looks_like_source_checkout(&candidate) {
            fallback
        } else {
            candidate
        }
    }

    pub(in crate::runtime) fn settings_manager(&self) -> Result<SecureConfigManager> {
        SecureConfigManager::new_with_data_dir(&self.config_dir, Some(self.data_dir()))
    }

    pub(in crate::runtime) fn workspace_root(&self) -> PathBuf {
        let configured = std::env::var("AGENTARK_FILE_WORKSPACE_ROOT")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        Self::workspace_root_from_config(self.data_dir(), configured.as_deref())
    }

    pub(in crate::runtime) fn action_source_label(source: &ActionSource) -> &'static str {
        match source {
            ActionSource::System => "system",
            ActionSource::Bundled => "bundled",
            ActionSource::Custom => "custom",
        }
    }

    pub(in crate::runtime) fn is_contextual_review_finding(
        finding: &crate::security::action_guard::AnalysisFinding,
    ) -> bool {
        let placeholder_like = finding.matched_text.contains('$')
            || finding.matched_text.contains("${")
            || finding.matched_text.contains("{{");
        match finding.category {
            crate::security::action_guard::FindingCategory::NetworkAccess
            | crate::security::action_guard::FindingCategory::EnvironmentAccess => true,
            crate::security::action_guard::FindingCategory::CredentialPattern => placeholder_like,
            _ => false,
        }
    }

    pub(in crate::runtime) fn compute_review_risk_summary(
        static_analysis: &crate::security::action_guard::StaticAnalysisResult,
        blocked: bool,
    ) -> (f32, String, usize, usize) {
        let total_findings = static_analysis.findings.len();
        let contextual_findings = static_analysis
            .findings
            .iter()
            .filter(|f| Self::is_contextual_review_finding(f))
            .count();
        let mut score = ((static_analysis.total_severity as f32) / 4.0).min(10.0);
        let contextual_ratio = if total_findings > 0 {
            (contextual_findings as f32) / (total_findings as f32)
        } else {
            0.0
        };
        if contextual_ratio >= 0.75 {
            score *= 0.65;
        } else if contextual_ratio >= 0.5 {
            score *= 0.8;
        }
        match static_analysis.threat_level {
            crate::security::action_guard::ThreatLevel::Malicious => {
                if contextual_ratio >= 0.8 {
                    score = score.max(4.0);
                } else {
                    score = score.max(8.5);
                }
            }
            crate::security::action_guard::ThreatLevel::Suspicious => {
                score = score.max(5.0);
            }
            crate::security::action_guard::ThreatLevel::Clean => {}
        }
        if blocked && contextual_ratio < 0.8 {
            score = score.max(8.5);
        } else if blocked {
            score = score.max(5.0);
        }
        let score_10 = ((score.clamp(0.0, 10.0)) * 10.0).round() / 10.0;
        let band = if score_10 < 5.0 {
            "secure"
        } else if score_10 < 8.0 {
            "review"
        } else {
            "risky"
        };
        (
            score_10,
            band.to_string(),
            total_findings,
            contextual_findings,
        )
    }

    pub(in crate::runtime) fn fingerprint_text(parts: &[impl AsRef<str>]) -> String {
        let mut hasher = Sha256::new();
        for part in parts {
            hasher.update(part.as_ref().as_bytes());
            hasher.update(b"\n---\n");
        }
        hex::encode(hasher.finalize())
    }

    pub(in crate::runtime) fn is_env_var_style_key(key: &str) -> bool {
        !key.is_empty()
            && key.len() <= 128
            && key
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    }

    pub(in crate::runtime) fn builtin_env_from_agent_config(cfg: &AgentConfig, env: &str) -> bool {
        let mut providers: Vec<&crate::core::LlmProvider> = vec![&cfg.llm];
        if let Some(fallback) = cfg.llm_fallback.as_ref() {
            providers.push(fallback);
        }
        for slot in &cfg.model_pool.slots {
            if slot.enabled {
                providers.push(&slot.provider);
            }
        }
        match env {
            "OPENAI_API_KEY" => providers.into_iter().any(|provider| {
                matches!(
                    provider,
                    crate::core::LlmProvider::OpenAI { api_key, .. } if !api_key.is_empty()
                )
            }),
            "OPENROUTER_API_KEY" => providers.into_iter().any(|provider| {
                matches!(
                    provider,
                    crate::core::LlmProvider::OpenAI {
                        api_key,
                        base_url,
                        ..
                    } if !api_key.is_empty()
                        && base_url
                            .as_deref()
                            .unwrap_or("")
                            .contains("openrouter")
                )
            }),
            "ANTHROPIC_API_KEY" => providers.into_iter().any(|provider| {
                matches!(
                    provider,
                    crate::core::LlmProvider::Anthropic { api_key, .. } if !api_key.is_empty()
                )
            }),
            _ => false,
        }
    }

    pub(in crate::runtime) fn extract_required_envs_from_frontmatter(
        frontmatter: &str,
    ) -> Vec<String> {
        let mut envs: Vec<String> = Vec::new();
        let unique_push = |out: &mut Vec<String>, value: String| {
            if !out.iter().any(|existing| existing == &value) {
                out.push(value);
            }
        };
        if let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(frontmatter) {
            Self::collect_required_envs_from_yaml(&value, &mut envs, &unique_push);
        }

        envs
    }

    pub(in crate::runtime) fn collect_required_envs_from_yaml<F>(
        value: &serde_yaml::Value,
        envs: &mut Vec<String>,
        unique_push: &F,
    ) where
        F: Fn(&mut Vec<String>, String),
    {
        match value {
            serde_yaml::Value::Mapping(map) => {
                for value in map.values() {
                    Self::collect_required_envs_from_yaml(value, envs, unique_push);
                }
            }
            serde_yaml::Value::Sequence(items) => {
                for item in items {
                    Self::collect_required_envs_from_yaml(item, envs, unique_push);
                }
            }
            serde_yaml::Value::String(text) => {
                for item in Self::split_env_candidate_text(text) {
                    if item.contains('_') && Self::is_env_var_style_key(&item) {
                        unique_push(envs, item);
                    }
                }
            }
            _ => {}
        }
    }

    pub(in crate::runtime) fn split_env_candidate_text(text: &str) -> Vec<String> {
        text.split(|ch: char| ch == ',' || ch.is_whitespace() || ch == '[' || ch == ']')
            .map(|item| item.trim().trim_matches('"').trim_matches('\''))
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect()
    }

    pub(in crate::runtime) fn split_frontmatter_block(content: &str) -> Option<(&str, &str)> {
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

    pub(in crate::runtime) fn parse_frontmatter_yaml(
        frontmatter: &str,
    ) -> Option<serde_yaml::Value> {
        let trimmed = frontmatter.trim();
        if trimmed.is_empty() {
            return None;
        }
        serde_yaml::from_str::<serde_yaml::Value>(trimmed).ok()
    }

    pub(in crate::runtime) fn extract_auth_profile_id_from_frontmatter(
        frontmatter: &str,
    ) -> Option<String> {
        let yaml = Self::parse_frontmatter_yaml(frontmatter)?;
        let root = yaml.as_mapping()?;
        let direct_keys = ["auth_profile", "auth_profile_id"];
        for key in direct_keys {
            if let Some(value) = root
                .get(serde_yaml::Value::String(key.to_string()))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Some(value.to_string());
            }
        }
        let auth = root.get(serde_yaml::Value::String("auth".to_string()))?;
        let auth_map = auth.as_mapping()?;
        for key in ["profile", "profile_id", "id"] {
            if let Some(value) = auth_map
                .get(serde_yaml::Value::String(key.to_string()))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Some(value.to_string());
            }
        }
        None
    }

    pub(in crate::runtime) fn extract_auth_env_exports_from_frontmatter(
        frontmatter: &str,
    ) -> BTreeMap<String, String> {
        let mut exports = BTreeMap::new();
        let Some(yaml) = Self::parse_frontmatter_yaml(frontmatter) else {
            return exports;
        };
        let Some(root) = yaml.as_mapping() else {
            return exports;
        };
        let Some(auth) = root.get(serde_yaml::Value::String("auth".to_string())) else {
            return exports;
        };
        let Some(auth_map) = auth.as_mapping() else {
            return exports;
        };
        let mapping = auth_map
            .get(serde_yaml::Value::String("env_exports".to_string()))
            .or_else(|| auth_map.get(serde_yaml::Value::String("exports".to_string())));
        let Some(mapping) = mapping.and_then(|value| value.as_mapping()) else {
            return exports;
        };
        for (key, value) in mapping {
            let Some(env_name) = key.as_str().map(str::trim).filter(|item| !item.is_empty()) else {
                continue;
            };
            let Some(source) = value
                .as_str()
                .map(str::trim)
                .filter(|item| !item.is_empty())
            else {
                continue;
            };
            exports.insert(env_name.to_string(), source.to_string());
        }
        exports
    }

    pub(in crate::runtime) fn env_is_configured_for_action(
        cfg: &AgentConfig,
        custom: &std::collections::HashMap<String, String>,
        action_name: &str,
        env: &str,
    ) -> bool {
        let binding_key = format!("action_envmap:{}:{}", action_name, env);
        let target = custom.get(&binding_key).map(|s| s.as_str()).unwrap_or(env);
        if target == "builtin" {
            return Self::builtin_env_from_agent_config(cfg, env);
        }
        crate::core::runtime::secrets::has_user_secret(custom, target)
            || Self::builtin_env_from_agent_config(cfg, env)
    }

    pub(in crate::runtime) fn plugin_secret_key(plugin_id: &str) -> String {
        format!("plugin_sdk_secret:{}", plugin_id.trim())
    }

    pub(in crate::runtime) fn build_blocked_review(
        action_name: &str,
        source_kind: &str,
        fingerprint: String,
        reason: impl Into<String>,
    ) -> ActionReviewSnapshot {
        ActionReviewSnapshot {
            action_name: action_name.to_string(),
            source_kind: source_kind.to_string(),
            reviewed_at: chrono::Utc::now().to_rfc3339(),
            fingerprint,
            status: ActionReviewStatus::Blocked,
            ready: false,
            allow_load: false,
            allow_execute: false,
            visible_in_catalog: false,
            integrity_ok: false,
            threat_level: "Unknown".to_string(),
            risk_band: "risky".to_string(),
            warnings: Vec::new(),
            findings: Vec::new(),
            required_env: Vec::new(),
            missing_env: Vec::new(),
            permissions_needed: Vec::new(),
            requires_auth: false,
            auth_configured: false,
            notes: Vec::new(),
            blocked_reason: Some(reason.into()),
            ..ActionReviewSnapshot::default()
        }
    }

    pub(in crate::runtime) fn build_review_from_verdict(
        input: ActionReviewBuildInput<'_>,
    ) -> ActionReviewSnapshot {
        let ActionReviewBuildInput {
            action_name,
            source_kind,
            fingerprint,
            verdict,
            required_env,
            missing_env,
            requires_auth,
            auth_configured,
            notes,
        } = input;
        let blocked = !verdict.allow_load;
        let (risk_score_10, risk_band, total_findings, _contextual_findings) =
            Self::compute_review_risk_summary(&verdict.static_analysis, blocked);
        let mut warnings = verdict.warnings.clone();
        warnings.extend(notes.iter().cloned());
        let permissions_needed = verdict
            .permissions_needed
            .iter()
            .map(|perm| perm.to_string())
            .collect::<Vec<_>>();
        let blocked_reason = if blocked {
            verdict
                .warnings
                .first()
                .cloned()
                .or_else(|| Some("Blocked by security review".to_string()))
        } else if !auth_configured && requires_auth {
            Some("Required authentication is not configured.".to_string())
        } else if !missing_env.is_empty() {
            Some(format!(
                "Required secrets missing: {}",
                missing_env.join(", ")
            ))
        } else {
            None
        };
        let status = if blocked {
            ActionReviewStatus::Blocked
        } else if !auth_configured && requires_auth || !missing_env.is_empty() {
            ActionReviewStatus::NeedsSecrets
        } else if !warnings.is_empty() || !permissions_needed.is_empty() || risk_band == "review" {
            ActionReviewStatus::Warning
        } else {
            ActionReviewStatus::Ready
        };
        let allow_execute = matches!(
            status,
            ActionReviewStatus::Ready | ActionReviewStatus::Warning
        );
        ActionReviewSnapshot {
            action_name: action_name.to_string(),
            source_kind: source_kind.to_string(),
            reviewed_at: chrono::Utc::now().to_rfc3339(),
            fingerprint,
            status,
            ready: allow_execute,
            allow_load: verdict.allow_load,
            allow_execute,
            visible_in_catalog: allow_execute,
            integrity_ok: verdict.integrity_ok,
            threat_level: format!("{:?}", verdict.static_analysis.threat_level),
            total_severity: verdict.static_analysis.total_severity,
            total_findings,
            risk_score_10,
            risk_band,
            warnings,
            findings: verdict.static_analysis.findings.clone(),
            required_env,
            missing_env,
            permissions_needed,
            requires_auth,
            auth_configured,
            notes,
            blocked_reason,
        }
    }

    pub(in crate::runtime) fn build_deterministic_runtime_action_review(
        action_name: &str,
        source_kind: &str,
        fingerprint: String,
        capabilities: &[String],
        required_env: Vec<String>,
        missing_env: Vec<String>,
        requires_auth: bool,
        auth_configured: bool,
        notes: Vec<String>,
    ) -> ActionReviewSnapshot {
        let warnings = notes.clone();
        let blocked_reason = if !auth_configured && requires_auth {
            Some("Required authentication is not configured.".to_string())
        } else if !missing_env.is_empty() {
            Some(format!(
                "Required secrets missing: {}",
                missing_env.join(", ")
            ))
        } else {
            None
        };
        let status = if !auth_configured && requires_auth || !missing_env.is_empty() {
            ActionReviewStatus::NeedsSecrets
        } else if !warnings.is_empty() {
            ActionReviewStatus::Warning
        } else {
            ActionReviewStatus::Ready
        };
        let allow_execute = matches!(
            status,
            ActionReviewStatus::Ready | ActionReviewStatus::Warning
        );
        let mut review = ActionReviewSnapshot {
            action_name: action_name.to_string(),
            source_kind: source_kind.to_string(),
            reviewed_at: chrono::Utc::now().to_rfc3339(),
            fingerprint,
            status,
            ready: allow_execute,
            allow_load: true,
            allow_execute,
            visible_in_catalog: allow_execute,
            integrity_ok: true,
            threat_level: "Clean".to_string(),
            total_severity: 0,
            total_findings: 0,
            risk_score_10: 0.0,
            risk_band: "secure".to_string(),
            warnings,
            findings: Vec::new(),
            required_env,
            missing_env,
            permissions_needed: Vec::new(),
            requires_auth,
            auth_configured,
            notes,
            blocked_reason,
        };
        let capability_report = crate::security::capabilities::evaluate_declared_capabilities(
            source_kind,
            action_name,
            capabilities,
        );
        Self::apply_capability_report_to_review(&mut review, capability_report);
        Self::reconcile_dynamic_review_state(&mut review);
        review
    }

    pub(in crate::runtime) fn apply_capability_report_to_review(
        review: &mut ActionReviewSnapshot,
        report: crate::security::capabilities::CapabilityLayerReport,
    ) {
        for observation in &report.observations {
            let selector = observation.selector();
            if !review
                .permissions_needed
                .iter()
                .any(|existing| existing == &selector)
            {
                review.permissions_needed.push(selector);
            }
        }
        for warning in &report.warnings {
            if !review.warnings.iter().any(|existing| existing == warning) {
                review.warnings.push(warning.clone());
            }
            if !review.notes.iter().any(|existing| existing == warning) {
                review.notes.push(warning.clone());
            }
        }
        for rule in &report.matched_rules {
            let note = format!("Capability policy rule '{}': {}", rule.id, rule.message);
            if !review.notes.iter().any(|existing| existing == &note) {
                review.notes.push(note);
            }
        }
        review.findings.extend(report.findings);
        review.total_findings = review.findings.len();
        review.total_severity = review.total_severity.saturating_add(report.total_severity);
        if report.risk_score_10 > review.risk_score_10 {
            review.risk_score_10 = report.risk_score_10;
            review.risk_band = report.risk_band.clone();
        }
        if matches!(
            report.threat_level,
            crate::security::action_guard::ThreatLevel::Malicious
        ) {
            review.threat_level = "Malicious".to_string();
        } else if matches!(
            report.threat_level,
            crate::security::action_guard::ThreatLevel::Suspicious
        ) && review.threat_level != "Malicious"
        {
            review.threat_level = "Suspicious".to_string();
        }

        if report.blocked {
            review.status = ActionReviewStatus::Blocked;
            review.ready = false;
            review.allow_load = false;
            review.allow_execute = false;
            review.visible_in_catalog = false;
            review.blocked_reason = report
                .warnings
                .first()
                .cloned()
                .or_else(|| Some("Blocked by capability security policy.".to_string()));
            return;
        }

        if matches!(review.status, ActionReviewStatus::Ready)
            && (!report.warnings.is_empty()
                || report.risk_band == "review"
                || report.risk_band == "risky")
        {
            review.status = ActionReviewStatus::Warning;
            review.ready = true;
            review.allow_execute = true;
            review.visible_in_catalog = true;
        }
    }

    pub(in crate::runtime) async fn record_security_event(
        &self,
        event_type: &str,
        severity: &str,
        message: String,
        source: Option<String>,
    ) {
        let Some(storage) = self.storage.as_ref() else {
            tracing::info!(
                event_type = event_type,
                severity = severity,
                source = source.as_deref().unwrap_or("runtime"),
                "{}",
                message
            );
            return;
        };
        let log = crate::storage::entities::security_log::Model {
            id: uuid::Uuid::new_v4().to_string(),
            event_type: event_type.to_string(),
            severity: severity.to_string(),
            message: crate::security::redact_pii(&message),
            source,
            count: 1,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Err(error) = storage.insert_security_log(&log).await {
            tracing::debug!("Failed to persist action security event: {}", error);
        }
    }

    pub(in crate::runtime) async fn record_custom_messaging_channel_upsert_event(
        &self,
        channel: &crate::custom_messaging_channels::CustomMessagingChannelView,
        operation: &'static str,
    ) {
        self.record_security_event(
            if operation == "update" {
                "custom_messaging_channel_update"
            } else {
                "custom_messaging_channel_create"
            },
            "medium",
            format!(
                "Custom messaging channel {} by runtime action. channel_id={}",
                operation, channel.id
            ),
            Some(format!(
                "actor=runtime_action;source_kind=custom_channel;channel_id={}",
                channel.id
            )),
        )
        .await;

        let mut capabilities = vec![
            "calls-network".to_string(),
            "sends-message".to_string(),
            "sends-external".to_string(),
        ];
        if channel.requires_auth {
            capabilities.push("requests-secrets".to_string());
            capabilities.push("uses-auth-profile".to_string());
        }
        let report = crate::security::capabilities::evaluate_declared_capabilities(
            "custom_channel",
            &channel.id,
            &capabilities,
        );
        let severity = if report.blocked || report.risk_score_10 >= 8.0 {
            "high"
        } else if report.risk_score_10 >= 5.0 || !report.warnings.is_empty() {
            "medium"
        } else {
            "low"
        };
        let rules = report
            .matched_rules
            .iter()
            .map(|rule| rule.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        self.record_security_event(
            "capability_review",
            severity,
            format!(
                "Custom messaging channel capability review: channel_id={}, risk_score={}, capabilities=[{}], rules=[{}]",
                channel.id,
                report.risk_score_10,
                capabilities.join(", "),
                rules
            ),
            Some(format!(
                "actor=runtime_action;source_kind=custom_channel;channel_id={}",
                channel.id
            )),
        )
        .await;
    }

    pub(in crate::runtime) fn review_event_severity(review: &ActionReviewSnapshot) -> &'static str {
        if matches!(review.status, ActionReviewStatus::Blocked) || review.risk_score_10 >= 8.0 {
            "high"
        } else if review.risk_score_10 >= 5.0 || !review.warnings.is_empty() {
            "medium"
        } else {
            "low"
        }
    }

    pub(in crate::runtime) fn has_semantic_skill_review_marker(
        review: &ActionReviewSnapshot,
    ) -> bool {
        review.notes.iter().any(|note| {
            note.starts_with("Semantic capability review used configured model ")
                || note.starts_with("Semantic capability review used configured model '")
        })
    }

    pub(in crate::runtime) async fn record_action_review_event(
        &self,
        review: &ActionReviewSnapshot,
    ) {
        if review.permissions_needed.is_empty()
            && review.warnings.is_empty()
            && !matches!(review.status, ActionReviewStatus::Blocked)
        {
            return;
        }
        let message = format!(
            "Action capability review: action='{}', source='{}', status='{:?}', risk_score={}, capabilities=[{}], warnings={}",
            review.action_name,
            review.source_kind,
            review.status,
            review.risk_score_10,
            review.permissions_needed.join(", "),
            review.warnings.len()
        );
        self.record_security_event(
            "capability_review",
            Self::review_event_severity(review),
            message,
            Some(format!(
                "source_kind={};action={}",
                review.source_kind, review.action_name
            )),
        )
        .await;
    }

    pub(in crate::runtime) async fn record_cross_layer_capability_correlation(&self) {
        let observations = {
            let reviews = self.action_reviews.read().await;
            let mut observations = Vec::new();
            for record in reviews.values() {
                let review = &record.current;
                if matches!(review.status, ActionReviewStatus::Blocked)
                    || !review.allow_load
                    || review.permissions_needed.is_empty()
                {
                    continue;
                }
                observations.extend(
                    crate::security::capabilities::observations_from_declared_capabilities(
                        &review.source_kind,
                        &review.action_name,
                        &review.permissions_needed,
                    ),
                );
            }
            observations
        };
        let Some(report) =
            crate::security::capabilities::evaluate_cross_layer_capabilities(observations)
        else {
            return;
        };
        let rules = report
            .matched_rules
            .iter()
            .map(|rule| rule.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let subjects = report
            .observations
            .iter()
            .map(|observation| format!("{}:{}", observation.layer, observation.entity_id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(", ");
        self.record_security_event(
            "capability_correlation",
            "high",
            format!(
                "Cross-layer capability policy match: rules=[{}], subjects=[{}]",
                rules, subjects
            ),
            Some("scope=runtime".to_string()),
        )
        .await;
    }

    pub(in crate::runtime) fn prune_cli_auth_exported_envs(
        review: &mut ActionReviewSnapshot,
        binding: &CliToolBinding,
    ) {
        if binding.auth_env_exports.is_empty() {
            return;
        }
        review
            .missing_env
            .retain(|env| !binding.auth_env_exports.contains_key(env));
    }

    pub(in crate::runtime) fn reconcile_dynamic_review_state(review: &mut ActionReviewSnapshot) {
        if matches!(review.status, ActionReviewStatus::Blocked) {
            review.ready = false;
            review.allow_execute = false;
            review.visible_in_catalog = false;
            return;
        }

        if !review.missing_env.is_empty() || (review.requires_auth && !review.auth_configured) {
            review.status = ActionReviewStatus::NeedsSecrets;
            review.ready = false;
            review.allow_execute = false;
            review.visible_in_catalog = false;
            if review.blocked_reason.is_none() {
                review.blocked_reason = if !review.missing_env.is_empty() {
                    Some(format!(
                        "Required secrets missing: {}",
                        review.missing_env.join(", ")
                    ))
                } else {
                    Some("Required authentication is not configured.".to_string())
                };
            }
        } else {
            review.ready = review.allow_load;
            review.allow_execute = review.allow_load;
            review.visible_in_catalog = review.allow_load;
            review.blocked_reason = None;
            if matches!(
                review.status,
                ActionReviewStatus::NeedsSecrets | ActionReviewStatus::Unreviewed
            ) {
                review.status = if review.warnings.is_empty() {
                    ActionReviewStatus::Ready
                } else {
                    ActionReviewStatus::Warning
                };
            }
        }
    }

    pub(in crate::runtime) async fn compute_missing_required_envs(
        &self,
        action_name: &str,
        required_env: &[String],
    ) -> Result<Vec<String>> {
        if required_env.is_empty() {
            return Ok(Vec::new());
        }
        let manager =
            SecureConfigManager::new_with_data_dir(&self.config_dir, Some(self.data_dir()))?;
        let config = manager.load()?;
        let secrets = manager.load_secrets()?;
        let custom = &secrets.custom;
        let mut missing = Vec::new();
        for env in required_env {
            if !Self::env_is_configured_for_action(&config, custom, action_name, env) {
                missing.push(env.clone());
            }
        }
        Ok(missing)
    }

    pub(in crate::runtime) async fn auth_profile_status(
        &self,
        auth_profile_id: &str,
    ) -> Result<(bool, Vec<String>)> {
        let storage = self
            .storage()
            .ok_or_else(|| anyhow::anyhow!("Storage is unavailable for auth profile lookups"))?;
        let view = crate::core::connectivity::auth_profiles::AuthProfileControlPlane::get(
            &storage,
            auth_profile_id,
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("Auth profile '{}' was not found", auth_profile_id))?;
        let mut notes = Vec::new();
        if let Some(reason) = view.blocked_reason {
            notes.push(reason);
        }
        Ok((view.ready, notes))
    }

    pub(in crate::runtime) async fn resolve_auth_profile_http(
        &self,
        auth_profile_id: &str,
    ) -> Result<crate::core::connectivity::auth_profiles::AuthProfileResolution> {
        let storage = self
            .storage()
            .ok_or_else(|| anyhow::anyhow!("Storage is unavailable for auth profile lookups"))?;
        crate::core::connectivity::auth_profiles::AuthProfileControlPlane::resolve_http(
            &storage,
            auth_profile_id,
        )
        .await
    }

    pub(in crate::runtime) async fn review_markdown_action(
        &self,
        action_dir: &Path,
        info: &ActionDef,
        workflow_content: &str,
        frontmatter: &str,
    ) -> Result<ActionReviewSnapshot> {
        let Some(guard) = self.action_guard.as_ref() else {
            let fingerprint = crate::security::ActionGuard::compute_bundle_hash(action_dir)
                .unwrap_or_else(|_| Self::fingerprint_text(&[workflow_content]));
            return Ok(Self::build_blocked_review(
                &info.name,
                Self::action_source_label(&info.source),
                fingerprint,
                "Action security is unavailable, so user-added skills are not loadable.",
            ));
        };
        let verdict = guard
            .evaluate_action(action_dir, &info.name, workflow_content, frontmatter)
            .await?;
        let required_env = Self::extract_required_envs_from_frontmatter(frontmatter);
        let missing_env = self
            .compute_missing_required_envs(&info.name, &required_env)
            .await?;
        let auth_profile_id = Self::extract_auth_profile_id_from_frontmatter(frontmatter);
        let (requires_auth, auth_configured, mut notes) =
            if let Some(auth_profile_id) = auth_profile_id.as_deref() {
                let (ready, notes) = self.auth_profile_status(auth_profile_id).await?;
                (true, ready, notes)
            } else {
                (false, true, Vec::new())
            };
        if let Some(auth_profile_id) = auth_profile_id.as_deref() {
            notes.push(format!("Uses auth profile '{}'.", auth_profile_id));
        }
        let fingerprint = crate::security::ActionGuard::compute_bundle_hash(action_dir)
            .unwrap_or_else(|_| Self::fingerprint_text(&[workflow_content, frontmatter]));
        Ok(Self::build_review_from_verdict(ActionReviewBuildInput {
            action_name: &info.name,
            source_kind: Self::action_source_label(&info.source),
            fingerprint,
            verdict: &verdict,
            required_env,
            missing_env,
            requires_auth,
            auth_configured,
            notes,
        }))
    }

    pub(in crate::runtime) async fn review_cli_action(
        &self,
        action_dir: &Path,
        info: &ActionDef,
        skill_markdown: &str,
        frontmatter: &str,
        binding: &CliToolBinding,
    ) -> Result<ActionReviewSnapshot> {
        let mut review = self
            .review_markdown_action(action_dir, info, skill_markdown, frontmatter)
            .await?;
        if binding.auth_profile_id.is_some() {
            Self::prune_cli_auth_exported_envs(&mut review, binding);
            if binding.auth_env_exports.is_empty() {
                review.auth_configured = false;
                review.blocked_reason = Some(
                    "CLI auth profiles require `auth.env_exports` so credentials can be injected into the subprocess.".to_string(),
                );
                let note = "CLI auth profiles require `auth.env_exports` so credentials can be injected into the subprocess.".to_string();
                if !review.notes.iter().any(|existing| existing == &note) {
                    review.notes.push(note);
                }
            } else {
                let mut exported_envs =
                    binding.auth_env_exports.keys().cloned().collect::<Vec<_>>();
                exported_envs.sort();
                let note = format!("CLI auth exports: {}.", exported_envs.join(", "));
                if !review.notes.iter().any(|existing| existing == &note) {
                    review.notes.push(note);
                }
            }
            Self::reconcile_dynamic_review_state(&mut review);
        }
        let executable_ok = std::path::Path::new(&binding.executable_path).is_file();
        if executable_ok {
            return Ok(review);
        }
        review.status = ActionReviewStatus::NeedsSecrets;
        review.ready = false;
        review.allow_execute = false;
        review.visible_in_catalog = false;
        review.blocked_reason = Some(format!(
            "CLI executable '{}' is not present on this machine.",
            binding.executable_path
        ));
        let note =
            "CLI skills are machine-specific and must be revalidated after reload.".to_string();
        if !review.notes.iter().any(|existing| existing == &note) {
            review.notes.push(note);
        }
        Ok(review)
    }

    pub(in crate::runtime) async fn review_cli_action_for_startup(
        &self,
        action_dir: &Path,
        info: &ActionDef,
        skill_markdown: &str,
        frontmatter: &str,
        binding: &CliToolBinding,
    ) -> Result<ActionReviewSnapshot> {
        let required_env = Self::extract_required_envs_from_frontmatter(frontmatter);
        let missing_env = self
            .compute_missing_required_envs(&info.name, &required_env)
            .await?;
        let mut notes = Vec::new();
        let (requires_auth, auth_configured) =
            if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
                notes.push(format!("Uses auth profile '{}'.", auth_profile_id));
                let (ready, auth_notes) = self.auth_profile_status(auth_profile_id).await?;
                notes.extend(auth_notes);
                (true, ready)
            } else {
                (false, true)
            };
        let fingerprint = crate::security::ActionGuard::compute_bundle_hash(action_dir)
            .unwrap_or_else(|_| Self::fingerprint_text(&[skill_markdown, frontmatter]));
        let mut review = Self::build_deterministic_runtime_action_review(
            &info.name,
            Self::action_source_label(&info.source),
            fingerprint,
            &info.capabilities,
            required_env,
            missing_env,
            requires_auth,
            auth_configured,
            notes,
        );
        if binding.auth_profile_id.is_some() {
            Self::prune_cli_auth_exported_envs(&mut review, binding);
            if binding.auth_env_exports.is_empty() {
                review.auth_configured = false;
                review.blocked_reason = Some(
                    "CLI auth profiles require `auth.env_exports` so credentials can be injected into the subprocess.".to_string(),
                );
                let note = "CLI auth profiles require `auth.env_exports` so credentials can be injected into the subprocess.".to_string();
                if !review.notes.iter().any(|existing| existing == &note) {
                    review.notes.push(note);
                }
            } else {
                let mut exported_envs =
                    binding.auth_env_exports.keys().cloned().collect::<Vec<_>>();
                exported_envs.sort();
                let note = format!("CLI auth exports: {}.", exported_envs.join(", "));
                if !review.notes.iter().any(|existing| existing == &note) {
                    review.notes.push(note);
                }
            }
            Self::reconcile_dynamic_review_state(&mut review);
        }
        let executable_ok = std::path::Path::new(&binding.executable_path).is_file();
        if executable_ok {
            return Ok(review);
        }
        review.status = ActionReviewStatus::NeedsSecrets;
        review.ready = false;
        review.allow_execute = false;
        review.visible_in_catalog = false;
        review.blocked_reason = Some(format!(
            "CLI executable '{}' is not present on this machine.",
            binding.executable_path
        ));
        let note =
            "CLI skills are machine-specific and must be revalidated after reload.".to_string();
        if !review.notes.iter().any(|existing| existing == &note) {
            review.notes.push(note);
        }
        Ok(review)
    }

    pub(in crate::runtime) fn url_review_notes(url_str: &str) -> Vec<String> {
        let mut notes = Vec::new();
        if let Ok(url) = reqwest::Url::parse(url_str) {
            if url.scheme() != "https" {
                notes.push(format!("Remote endpoint '{}' does not use HTTPS.", url_str));
            }
            if let Some(host) = url.host_str() {
                let is_private = if host.eq_ignore_ascii_case("localhost") {
                    true
                } else if let Ok(ip) = host.parse::<std::net::IpAddr>() {
                    match ip {
                        std::net::IpAddr::V4(v4) => {
                            v4.is_private() || v4.is_loopback() || v4.is_link_local()
                        }
                        std::net::IpAddr::V6(v6) => {
                            v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local()
                        }
                    }
                } else {
                    false
                };
                if is_private {
                    notes.push(format!(
                        "Remote endpoint '{}' resolves to a private or loopback host.",
                        url_str
                    ));
                }
            }
        }
        notes
    }

    pub(in crate::runtime) async fn review_plugin_action(
        &self,
        info: &ActionDef,
        binding: &PluginBinding,
    ) -> Result<ActionReviewSnapshot> {
        let fingerprint = Self::fingerprint_text(&[
            info.name.as_str(),
            info.description.as_str(),
            &binding.base_url,
            &serde_json::to_string(&info.input_schema).unwrap_or_default(),
            &info.capabilities.join(","),
        ]);
        let mut notes = Self::url_review_notes(&binding.base_url);
        let auth_configured = if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
            notes.push(format!("Uses auth profile '{}'.", auth_profile_id));
            let (ready, auth_notes) = self.auth_profile_status(auth_profile_id).await?;
            notes.extend(auth_notes);
            ready
        } else {
            binding.auth_configured
        };
        let review = Self::build_deterministic_runtime_action_review(
            &info.name,
            "plugin",
            fingerprint,
            &info.capabilities,
            Vec::new(),
            Vec::new(),
            binding.auth_required,
            auth_configured,
            notes,
        );
        self.record_action_review_event(&review).await;
        Ok(review)
    }

    pub(in crate::runtime) async fn review_custom_api_action(
        &self,
        info: &ActionDef,
        binding: &CustomApiBinding,
    ) -> Result<ActionReviewSnapshot> {
        let fingerprint = Self::fingerprint_text(&[
            info.name.as_str(),
            info.description.as_str(),
            &binding.base_url,
            &binding.path,
            &binding.method,
            &serde_json::to_string(&info.input_schema).unwrap_or_default(),
            &info.capabilities.join(","),
        ]);
        if matches!(
            binding.auth_mode,
            crate::custom_apis::CustomApiAuthMode::OAuth2
        ) && binding.auth_profile_id.is_none()
        {
            return Ok(Self::build_blocked_review(
                &info.name,
                "custom_api",
                fingerprint,
                "OAuth2 custom API actions require a bound auth profile.",
            ));
        }
        let mut notes = Self::url_review_notes(&binding.base_url);
        if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
            notes.push(format!("Uses auth profile '{}'.", auth_profile_id));
        }
        let requires_auth = binding.auth_profile_id.is_some()
            || !matches!(
                binding.auth_mode,
                crate::custom_apis::CustomApiAuthMode::None
            );
        let auth_configured = if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
            let (ready, auth_notes) = self.auth_profile_status(auth_profile_id).await?;
            notes.extend(auth_notes);
            ready
        } else if requires_auth {
            let manager =
                SecureConfigManager::new_with_data_dir(&self.config_dir, Some(self.data_dir()))?;
            manager
                .get_custom_secret(&binding.secret_key)?
                .is_some_and(|value| !value.trim().is_empty())
        } else {
            true
        };
        let review = Self::build_deterministic_runtime_action_review(
            &info.name,
            "custom_api",
            fingerprint,
            &info.capabilities,
            Vec::new(),
            Vec::new(),
            requires_auth,
            auth_configured,
            notes,
        );
        self.record_action_review_event(&review).await;
        Ok(review)
    }

    pub(in crate::runtime) async fn review_extension_pack_action(
        &self,
        info: &ActionDef,
        binding: &ExtensionPackActionBinding,
    ) -> Result<ActionReviewSnapshot> {
        let fingerprint = Self::fingerprint_text(&[
            info.name.as_str(),
            info.description.as_str(),
            &binding.pack_id,
            &binding.feature_id,
            &binding.action_name,
            &binding.binding_kind,
            &serde_json::to_string(&info.input_schema).unwrap_or_default(),
            &info.capabilities.join(","),
        ]);
        let Some(registry) = self.extension_pack_registry.as_ref() else {
            return Ok(Self::build_blocked_review(
                &info.name,
                "extension_pack",
                fingerprint,
                "Extension-pack registry is unavailable in this runtime.",
            ));
        };

        let pack = {
            let guard = registry.read().await;
            guard.get_pack(&binding.pack_id).await?
        };
        let Some(pack) = pack else {
            return Ok(Self::build_blocked_review(
                &info.name,
                "extension_pack",
                fingerprint,
                format!("Extension pack '{}' was not found.", binding.pack_id),
            ));
        };

        let mut notes = Vec::new();
        notes.push(format!(
            "Uses extension pack '{}' ({}).",
            pack.manifest.name, pack.manifest.id
        ));
        notes.push(format!("Feature '{}'.", binding.feature_id));
        notes.push(format!("Binding kind: {}.", binding.binding_kind));
        if let Some(connection_id) = binding.connection_id.as_deref() {
            notes.push(format!("Connection '{}'.", connection_id));
        }
        if matches!(
            pack.trust_level,
            crate::extension_packs::ExtensionPackTrustLevel::Unverified
        ) {
            notes.push("Pack is installed as unverified.".to_string());
        }

        let mut review = Self::build_deterministic_runtime_action_review(
            &info.name,
            "extension_pack",
            fingerprint,
            &info.capabilities,
            Vec::new(),
            Vec::new(),
            false,
            true,
            notes,
        );
        if matches!(
            pack.trust_level,
            crate::extension_packs::ExtensionPackTrustLevel::Unverified
        ) && (!binding.read_only || binding.binding_kind.eq_ignore_ascii_case("local_cli"))
        {
            review.status = ActionReviewStatus::Blocked;
            review.ready = false;
            review.allow_load = false;
            review.allow_execute = false;
            review.visible_in_catalog = false;
            review.blocked_reason = Some(
                "Unverified extension packs may not expose host CLI or write-capable actions."
                    .to_string(),
            );
        }
        self.record_action_review_event(&review).await;
        Ok(review)
    }

    pub(in crate::runtime) async fn review_mcp_action(
        &self,
        info: &ActionDef,
        binding: &McpBinding,
    ) -> Result<ActionReviewSnapshot> {
        let fingerprint = Self::fingerprint_text(&[
            info.name.as_str(),
            info.description.as_str(),
            &binding.server_id,
            &binding.server_name,
            &info.capabilities.join(","),
        ]);
        let mut warnings = binding.warnings.clone();
        let auth_configured = if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
            warnings.push(format!("Uses auth profile '{}'.", auth_profile_id));
            let (ready, notes) = self.auth_profile_status(auth_profile_id).await?;
            warnings.extend(notes);
            ready
        } else {
            binding.auth_configured
        };
        let review = Self::build_deterministic_runtime_action_review(
            &info.name,
            "mcp",
            fingerprint,
            &info.capabilities,
            Vec::new(),
            Vec::new(),
            binding.auth_required,
            auth_configured,
            warnings,
        );
        self.record_action_review_event(&review).await;
        Ok(review)
    }

    pub async fn refresh_action_review_state(
        &self,
        action_name: &str,
    ) -> Result<Option<ActionReviewSnapshot>> {
        let loaded = {
            let actions = self.actions.read().await;
            actions.get(action_name).map(|action| {
                (
                    action.info.clone(),
                    action.cli_binding.clone(),
                    action.plugin_binding.clone(),
                    action.custom_api_binding.clone(),
                    action.mcp_binding.clone(),
                    action.extension_pack_binding.clone(),
                )
            })
        };
        let Some((
            info,
            cli_binding,
            plugin_binding,
            custom_api_binding,
            mcp_binding,
            extension_pack_binding,
        )) = loaded
        else {
            return Ok(None);
        };
        let mut review = match self.get_action_review(action_name).await {
            Some(review) => review,
            None => return Ok(None),
        };

        if info.source != ActionSource::System {
            if let Some(action_dir) = info
                .file_path
                .as_deref()
                .and_then(|file_path| Path::new(file_path).parent().map(Path::to_path_buf))
            {
                match crate::security::ActionGuard::compute_bundle_hash(&action_dir) {
                    Ok(current_fingerprint)
                        if !review.fingerprint.is_empty()
                            && current_fingerprint != review.fingerprint =>
                    {
                        let note = "Skill files changed on disk outside the reviewed API path; re-import or update the skill to run semantic review again.".to_string();
                        review.status = ActionReviewStatus::Blocked;
                        review.ready = false;
                        review.allow_load = false;
                        review.allow_execute = false;
                        review.visible_in_catalog = false;
                        review.integrity_ok = false;
                        review.threat_level = "Malicious".to_string();
                        review.risk_band = "risky".to_string();
                        review.risk_score_10 = review.risk_score_10.max(8.5);
                        review.total_severity = review.total_severity.saturating_add(10);
                        review.blocked_reason = Some(note.clone());
                        if !review.warnings.iter().any(|existing| existing == &note) {
                            review.warnings.push(note.clone());
                        }
                        if !review.notes.iter().any(|existing| existing == &note) {
                            review.notes.push(note.clone());
                        }
                        review
                            .findings
                            .push(crate::security::action_guard::AnalysisFinding {
                                category:
                                    crate::security::action_guard::FindingCategory::BundleShape,
                                description:
                                    "Reviewed skill fingerprint no longer matches disk content."
                                        .to_string(),
                                matched_text: "disk-content-changed-after-review".to_string(),
                                line_number: 1,
                                severity: 10,
                                file_path: info.file_path.clone(),
                            });
                        review.total_findings = review.findings.len();
                        self.upsert_action_review(review.clone()).await?;
                        {
                            let mut disabled = self.disabled_actions.write().await;
                            if disabled.insert(action_name.to_string()) {
                                drop(disabled);
                                self.save_disabled_actions().await?;
                            }
                        }
                        self.record_action_review_event(&review).await;
                        return Ok(Some(review));
                    }
                    Err(error) => {
                        let note = format!(
                            "Unable to re-check reviewed skill bundle fingerprint: {}",
                            error
                        );
                        review.status = ActionReviewStatus::Blocked;
                        review.ready = false;
                        review.allow_load = false;
                        review.allow_execute = false;
                        review.visible_in_catalog = false;
                        review.integrity_ok = false;
                        review.threat_level = "Malicious".to_string();
                        review.risk_band = "risky".to_string();
                        review.risk_score_10 = review.risk_score_10.max(8.5);
                        review.total_severity = review.total_severity.saturating_add(10);
                        review.blocked_reason = Some(note.clone());
                        if !review.warnings.iter().any(|existing| existing == &note) {
                            review.warnings.push(note.clone());
                        }
                        if !review.notes.iter().any(|existing| existing == &note) {
                            review.notes.push(note.clone());
                        }
                        self.upsert_action_review(review.clone()).await?;
                        {
                            let mut disabled = self.disabled_actions.write().await;
                            if disabled.insert(action_name.to_string()) {
                                drop(disabled);
                                self.save_disabled_actions().await?;
                            }
                        }
                        self.record_action_review_event(&review).await;
                        return Ok(Some(review));
                    }
                    _ => {}
                }
            }
        }

        if matches!(review.status, ActionReviewStatus::Blocked) {
            return Ok(Some(review));
        }

        if !review.required_env.is_empty() {
            review.missing_env = self
                .compute_missing_required_envs(action_name, &review.required_env)
                .await?;
        }

        let mut cli_executable_missing = None::<String>;
        if let Some(binding) = cli_binding {
            if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
                review.requires_auth = true;
                Self::prune_cli_auth_exported_envs(&mut review, &binding);
                if binding.auth_env_exports.is_empty() {
                    review.auth_configured = false;
                    review.blocked_reason = Some(
                        "CLI auth profiles require `auth.env_exports` so credentials can be injected into the subprocess.".to_string(),
                    );
                    let note = "CLI auth profiles require `auth.env_exports` so credentials can be injected into the subprocess.".to_string();
                    if !review.notes.iter().any(|existing| existing == &note) {
                        review.notes.push(note);
                    }
                } else {
                    let (ready, notes) = self.auth_profile_status(auth_profile_id).await?;
                    review.auth_configured = ready;
                    for note in notes {
                        if !review.notes.iter().any(|existing| existing == &note) {
                            review.notes.push(note);
                        }
                    }
                    let mut exported_envs =
                        binding.auth_env_exports.keys().cloned().collect::<Vec<_>>();
                    exported_envs.sort();
                    let note = format!("CLI auth exports: {}.", exported_envs.join(", "));
                    if !review.notes.iter().any(|existing| existing == &note) {
                        review.notes.push(note);
                    }
                }
            }
            if !std::path::Path::new(&binding.executable_path).is_file() {
                cli_executable_missing = Some(binding.executable_path.clone());
            }
        }

        if let Some(binding) = plugin_binding {
            review.requires_auth = binding.auth_required;
            if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
                let (ready, notes) = self.auth_profile_status(auth_profile_id).await?;
                review.auth_configured = ready;
                for note in notes {
                    if !review.notes.iter().any(|existing| existing == &note) {
                        review.notes.push(note);
                    }
                }
            } else if binding.auth_required {
                let manager = SecureConfigManager::new_with_data_dir(
                    &self.config_dir,
                    Some(self.data_dir()),
                )?;
                review.auth_configured = manager
                    .get_custom_secret(&Self::plugin_secret_key(&binding.plugin_id))?
                    .is_some_and(|value| !value.trim().is_empty());
            }
        }

        if let Some(binding) = custom_api_binding {
            let requires_auth = binding.auth_profile_id.is_some()
                || !matches!(
                    binding.auth_mode,
                    crate::custom_apis::CustomApiAuthMode::None
                );
            review.requires_auth = requires_auth;
            if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
                let (ready, notes) = self.auth_profile_status(auth_profile_id).await?;
                review.auth_configured = ready;
                for note in notes {
                    if !review.notes.iter().any(|existing| existing == &note) {
                        review.notes.push(note);
                    }
                }
            } else if requires_auth {
                let manager = SecureConfigManager::new_with_data_dir(
                    &self.config_dir,
                    Some(self.data_dir()),
                )?;
                review.auth_configured = manager
                    .get_custom_secret(&binding.secret_key)?
                    .is_some_and(|value| !value.trim().is_empty());
            }
        }

        if let Some(binding) = mcp_binding {
            review.requires_auth = binding.auth_required;
            review.auth_configured =
                if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
                    let (ready, notes) = self.auth_profile_status(auth_profile_id).await?;
                    for note in notes {
                        if !review.notes.iter().any(|existing| existing == &note) {
                            review.notes.push(note);
                        }
                    }
                    ready
                } else {
                    binding.auth_configured
                };
        }

        if let Some(binding) = extension_pack_binding {
            let note = format!(
                "Uses extension pack '{}' feature '{}'.",
                binding.pack_id, binding.feature_id
            );
            if !review.notes.iter().any(|existing| existing == &note) {
                review.notes.push(note);
            }
        }

        Self::reconcile_dynamic_review_state(&mut review);

        if let Some(executable_path) = cli_executable_missing {
            review.status = ActionReviewStatus::NeedsSecrets;
            review.ready = false;
            review.allow_execute = false;
            review.visible_in_catalog = false;
            review.blocked_reason = Some(format!(
                "CLI executable '{}' is not present on this machine.",
                executable_path
            ));
            let note =
                "CLI skills are machine-specific and must be revalidated after reload.".to_string();
            if !review.notes.iter().any(|existing| existing == &note) {
                review.notes.push(note);
            }
        }

        review.source_kind = if info.source == ActionSource::System {
            review.source_kind
        } else {
            Self::action_source_label(&info.source).to_string()
        };
        self.upsert_action_review(review.clone()).await?;
        Ok(Some(review))
    }
}
