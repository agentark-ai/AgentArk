use super::*;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct PendingSkillImport {
    pub(super) source_url: String,
    pub(super) skill_name: String,
    pub(super) requested_at: chrono::DateTime<chrono::Utc>,
}

impl Agent {
    pub(super) fn builtin_env_available_for_skill_import(
        cfg: &crate::core::config::AgentConfig,
        env: &str,
    ) -> bool {
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

    fn extract_skill_required_envs(content: &str) -> Vec<String> {
        let Some(stripped) = content.strip_prefix("---") else {
            return Vec::new();
        };
        let Some(end) = stripped.find("---") else {
            return Vec::new();
        };
        let frontmatter = &stripped[..end];

        let mut envs: Vec<String> = Vec::new();
        let unique_push = |out: &mut Vec<String>, value: String| {
            if !out.iter().any(|existing| existing == &value) {
                out.push(value);
            }
        };
        let is_env_key = |key: &str| {
            !key.is_empty()
                && key.len() <= 128
                && key
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        };

        if let (Ok(re_env_arr), Ok(re_quoted)) = (
            regex::Regex::new(r#"(?s)"env"\s*:\s*\[([^\]]*)\]"#),
            regex::Regex::new(r#""([A-Z0-9_]{2,})""#),
        ) {
            for cap in re_env_arr.captures_iter(frontmatter) {
                if let Some(inner) = cap.get(1).map(|value| value.as_str()) {
                    for quoted in re_quoted.captures_iter(inner) {
                        if let Some(name) = quoted.get(1).map(|value| value.as_str()) {
                            unique_push(&mut envs, name.to_string());
                        }
                    }
                }
            }
        }

        if let Ok(re_primary) = regex::Regex::new(r#""primaryEnv"\s*:\s*"([A-Z0-9_]{2,})""#) {
            for cap in re_primary.captures_iter(frontmatter) {
                if let Some(name) = cap.get(1).map(|value| value.as_str()) {
                    unique_push(&mut envs, name.to_string());
                }
            }
        }

        let mut in_list = false;
        for raw in frontmatter.lines() {
            let line = raw.trim_end();
            let trimmed = line.trim();
            if trimmed.starts_with("secrets:")
                || trimmed.starts_with("env:")
                || trimmed.starts_with("required_env:")
            {
                in_list = true;
                if let Some(start) = trimmed.find('[') {
                    if let Some(end) = trimmed.rfind(']') {
                        if end > start {
                            let inner = &trimmed[start + 1..end];
                            for part in inner.split(',') {
                                let name = part.trim().trim_matches('"').trim_matches('\'');
                                if is_env_key(name) {
                                    unique_push(&mut envs, name.to_string());
                                }
                            }
                        }
                    }
                } else if let Some((_key, rhs)) = trimmed.split_once(':') {
                    let name = rhs.trim().trim_matches('"').trim_matches('\'');
                    if is_env_key(name) {
                        unique_push(&mut envs, name.to_string());
                    }
                }
                continue;
            }

            if !raw.starts_with(' ') && !raw.starts_with('\t') && trimmed.contains(':') {
                in_list = false;
            }
            if in_list {
                if let Some(item) = trimmed.strip_prefix("- ") {
                    let name = item.trim().trim_matches('"').trim_matches('\'');
                    if is_env_key(name) {
                        unique_push(&mut envs, name.to_string());
                    }
                }
            }
        }

        envs
    }

    pub(super) async fn missing_skill_required_envs(
        &self,
        action_name: &str,
        content: &str,
    ) -> Result<Vec<String>> {
        let required_env = Self::extract_skill_required_envs(content);
        if required_env.is_empty() {
            return Ok(Vec::new());
        }

        let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
            &self.config_dir,
            Some(&self.data_dir),
        )?;
        let secrets = manager.load_secrets()?;
        let custom = &secrets.custom;

        let mut missing = Vec::new();
        for env in required_env {
            let binding_key = format!("action_envmap:{}:{}", action_name, env);
            let target = custom
                .get(&binding_key)
                .map(|value| value.as_str())
                .unwrap_or(env.as_str());
            let configured = if target == "builtin" {
                Self::builtin_env_available_for_skill_import(&self.config, &env)
            } else {
                crate::core::secrets::has_user_secret(custom, target)
                    || Self::builtin_env_available_for_skill_import(&self.config, &env)
            };
            if !configured {
                missing.push(env);
            }
        }

        Ok(missing)
    }

    fn pending_skill_import_key(conversation_id: &str) -> String {
        format!("pending_skill_import:{}", conversation_id.trim())
    }

    async fn load_pending_skill_import(&self, conversation_id: &str) -> Option<PendingSkillImport> {
        if conversation_id.trim().is_empty() {
            return None;
        }
        if let Some(item) = self
            .pending_skill_imports
            .read()
            .await
            .get(conversation_id)
            .cloned()
        {
            return Some(item);
        }
        let key = Self::pending_skill_import_key(conversation_id);
        let item = self.load_encrypted_json::<PendingSkillImport>(&key).await?;
        self.pending_skill_imports
            .write()
            .await
            .insert(conversation_id.to_string(), item.clone());
        Some(item)
    }

    pub(super) async fn peek_pending_skill_import(
        &self,
        conversation_id: &str,
    ) -> Option<PendingSkillImport> {
        if conversation_id.trim().is_empty() {
            return None;
        }
        let item = self.load_pending_skill_import(conversation_id).await?;
        if (chrono::Utc::now() - item.requested_at) > chrono::Duration::minutes(30) {
            self.clear_pending_skill_import(conversation_id).await;
            return None;
        }
        Some(item)
    }

    pub(super) async fn clear_pending_skill_import(&self, conversation_id: &str) {
        if conversation_id.trim().is_empty() {
            return;
        }
        self.pending_skill_imports
            .write()
            .await
            .remove(conversation_id);
        let key = Self::pending_skill_import_key(conversation_id);
        let _ = self.storage.delete(&key).await;
    }
}
