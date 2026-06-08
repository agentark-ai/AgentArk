use super::*;

/// Conversation message for history tracking.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConversationMessage {
    pub role: String,
    pub content: String,
    pub _timestamp: chrono::DateTime<chrono::Utc>,
}

impl super::Agent {
    pub(super) fn conversation_digest_key(conversation_id: &str) -> String {
        format!("conversation_digest:{}", conversation_id.trim())
    }

    pub(crate) fn conversation_recent_artifact_key(conversation_id: &str) -> String {
        format!(
            "{}{}",
            CONVERSATION_RECENT_ARTIFACT_KEY_PREFIX,
            conversation_id.trim()
        )
    }

    pub(crate) fn conversation_last_deployed_app_key(conversation_id: &str) -> String {
        format!(
            "{}{}",
            CONVERSATION_LAST_DEPLOYED_APP_KEY_PREFIX,
            conversation_id.trim()
        )
    }

    pub(super) fn parse_message_timestamp(ts: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(ts)
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now())
    }

    pub(super) fn conversation_history_budget(
        &self,
        _scope: &str,
    ) -> crate::core::model::context_budget::HistoryTokenBudget {
        let history_tokens = crate::core::model::context_budget::read_usize_env(
            "AGENTARK_CHAT_HISTORY_TOKEN_BUDGET",
        )
        .unwrap_or_else(|| {
            DEFAULT_CHAT_HISTORY_CONTEXT_WINDOW_TOKENS
                .saturating_mul(DEFAULT_CHAT_HISTORY_BUDGET_RATIO_PERCENT)
                / 100
        })
        .saturating_sub(DEFAULT_DIRECT_CHAT_FIXED_PROMPT_TOKENS)
        .max(MIN_CHAT_HISTORY_TOKEN_BUDGET);
        crate::core::model::context_budget::HistoryTokenBudget {
            history_tokens,
            summary_tokens: history_tokens
                .saturating_mul(35)
                .saturating_div(100)
                .min(MAX_CHAT_HISTORY_SUMMARY_TOKENS),
        }
    }

    pub(super) fn chat_message_token_budget(
        budget: crate::core::model::context_budget::HistoryTokenBudget,
    ) -> usize {
        (budget.history_tokens / 8)
            .clamp(MIN_CHAT_MESSAGE_TOKEN_BUDGET, MAX_CHAT_MESSAGE_TOKEN_BUDGET)
    }

    pub(super) fn conversation_message_token_estimate(
        message: &ConversationMessage,
        max_tokens: usize,
    ) -> usize {
        crate::core::model::context_budget::estimate_role_message_tokens(
            &message.role,
            &crate::core::model::context_budget::truncate_to_token_budget(
                &message.content,
                max_tokens,
            ),
        )
    }

    async fn create_conversation_row_if_absent(
        &self,
        conversation_id: &str,
        channel: &str,
        project_id: Option<&str>,
        message_preview: &str,
    ) -> Result<()> {
        let conversation_id = conversation_id.trim();
        if conversation_id.is_empty() {
            return Ok(());
        }
        let now = chrono::Utc::now().to_rfc3339();
        let title = safe_truncate(message_preview.trim(), 50);
        let conv = crate::storage::entities::conversation::Model {
            id: conversation_id.to_string(),
            title: if title.is_empty() {
                "New conversation".to_string()
            } else {
                title
            },
            channel: channel.to_string(),
            project_id: project_id.map(str::to_string),
            created_at: now.clone(),
            updated_at: now,
            message_count: 0,
            archived: false,
            starred: false,
        };
        self.storage.create_conversation_if_absent(&conv).await?;
        Ok(())
    }

    fn routed_channel_conversation_key(
        channel: &str,
        route_conversation_id: &str,
        project_id: Option<&str>,
    ) -> Option<String> {
        let channel = channel.trim();
        let route_conversation_id = route_conversation_id.trim();
        if route_conversation_id.is_empty() || matches!(channel, "http" | "web") {
            return None;
        }

        let encoded_route = urlencoding::encode(route_conversation_id);
        let key = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(project_id) => format!(
                "active_conversation_route_{}_{}_{}",
                channel,
                urlencoding::encode(project_id),
                encoded_route
            ),
            None => format!("active_conversation_route_{}_{}", channel, encoded_route),
        };
        Some(key)
    }

    async fn load_existing_active_conversation(&self, key: &str) -> Result<Option<String>> {
        let Some(active_id) = self
            .storage
            .get(key)
            .await?
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };

        if self.storage.get_conversation(&active_id).await?.is_some() {
            Ok(Some(active_id))
        } else {
            Ok(None)
        }
    }

    pub(crate) async fn ensure_conversation_id_for_request(
        &self,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        message_preview: &str,
    ) -> Result<String> {
        if let Some(id) = conversation_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if self.storage.get_conversation(id).await?.is_none() {
                anyhow::bail!("Conversation not found");
            }
            return Ok(id.to_string());
        }

        let scope = self.conversation_scope_mode().await;
        let conv_key = scope.conversation_key(channel, project_id);
        if let Some(active_id) = self
            .storage
            .get(&conv_key)
            .await?
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            if self.storage.get_conversation(&active_id).await?.is_some() {
                return Ok(active_id);
            }
        }

        let new_id = uuid::Uuid::new_v4().to_string();
        self.create_conversation_row_if_absent(&new_id, channel, project_id, message_preview)
            .await?;
        self.storage.set(&conv_key, new_id.as_bytes()).await?;
        Ok(new_id)
    }

    pub(super) async fn resolve_conversation_id(
        &self,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        message_preview: &str,
    ) -> Result<(String, bool)> {
        let scope = self.conversation_scope_mode().await;
        let conv_key = scope.conversation_key(channel, project_id);

        if let Some(id) = conversation_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if let Some(route_key) = Self::routed_channel_conversation_key(channel, id, project_id)
            {
                if let Some(active_id) = self.load_existing_active_conversation(&route_key).await? {
                    self.storage.set(&conv_key, active_id.as_bytes()).await?;
                    return Ok((active_id, false));
                }

                if self.storage.get_conversation(id).await?.is_none() {
                    self.create_conversation_row_if_absent(
                        id,
                        channel,
                        project_id,
                        message_preview,
                    )
                    .await?;
                    self.storage.set(&route_key, id.as_bytes()).await?;
                    self.storage.set(&conv_key, id.as_bytes()).await?;
                    return Ok((id.to_string(), true));
                }

                self.storage.set(&route_key, id.as_bytes()).await?;
                self.storage.set(&conv_key, id.as_bytes()).await?;
                return Ok((id.to_string(), false));
            }

            if self.storage.get_conversation(id).await?.is_none() {
                anyhow::bail!("Conversation not found");
            }
            self.storage.set(&conv_key, id.as_bytes()).await?;
            return Ok((id.to_string(), false));
        }

        if let Some(active_id) = self
            .storage
            .get(&conv_key)
            .await?
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            if self.storage.get_conversation(&active_id).await?.is_some() {
                return Ok((active_id, false));
            }
        }

        let new_id = uuid::Uuid::new_v4().to_string();
        self.create_conversation_row_if_absent(&new_id, channel, project_id, message_preview)
            .await?;
        self.storage.set(&conv_key, new_id.as_bytes()).await?;
        Ok((new_id, true))
    }

    pub(crate) async fn start_new_channel_conversation(
        &self,
        channel: &str,
        current_conversation_id: &str,
        project_id: Option<&str>,
        message_preview: &str,
    ) -> Result<String> {
        let scope = self.conversation_scope_mode().await;
        let conv_key = scope.conversation_key(channel, project_id);
        let new_id = uuid::Uuid::new_v4().to_string();
        self.create_conversation_row_if_absent(&new_id, channel, project_id, message_preview)
            .await?;
        self.storage.set(&conv_key, new_id.as_bytes()).await?;
        if let Some(route_key) =
            Self::routed_channel_conversation_key(channel, current_conversation_id, project_id)
        {
            self.storage.set(&route_key, new_id.as_bytes()).await?;
        }
        Ok(new_id)
    }

    pub(crate) async fn clear_current_channel_conversation(
        &self,
        channel: &str,
        conversation_id: &str,
        project_id: Option<&str>,
    ) -> Result<String> {
        let current_id = conversation_id.trim();
        if !current_id.is_empty() {
            let clear_id = if let Some(route_key) =
                Self::routed_channel_conversation_key(channel, current_id, project_id)
            {
                self.load_existing_active_conversation(&route_key)
                    .await?
                    .unwrap_or_else(|| current_id.to_string())
            } else {
                current_id.to_string()
            };
            self.clear_conversation_by_id(channel, &clear_id, project_id)
                .await;
        }
        self.start_new_channel_conversation(channel, current_id, project_id, "New Chat")
            .await
    }

    pub(super) async fn persist_conversation_artifact_context(
        &self,
        conversation_id: &str,
        spec: ConversationArtifactSpec<'_>,
    ) {
        let context = ConversationArtifactContext {
            artifact_type: spec.artifact_type.to_string(),
            artifact_id: spec.artifact_id.to_string(),
            title: spec.title.to_string(),
            summary: spec.summary.to_string(),
            url: spec.url.unwrap_or_default().to_string(),
            related_actions: spec
                .related_actions
                .iter()
                .map(|value| value.to_string())
                .collect(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        self.persist_conversation_artifact_context_payload(conversation_id, context)
            .await;
    }

    pub(super) async fn persist_conversation_artifact_context_payload(
        &self,
        conversation_id: &str,
        context: ConversationArtifactContext,
    ) {
        let conversation_id = conversation_id.trim();
        if conversation_id.is_empty() {
            return;
        }
        let mut contexts = self.load_recent_artifact_contexts(conversation_id).await;
        contexts.retain(|item| {
            item.artifact_type != context.artifact_type || item.artifact_id != context.artifact_id
        });
        contexts.insert(0, context);
        contexts.truncate(CONVERSATION_RECENT_ARTIFACT_LIMIT);
        if let Ok(bytes) = serde_json::to_vec(&contexts) {
            let _ = self
                .storage
                .set(
                    &Self::conversation_recent_artifact_key(conversation_id),
                    &bytes,
                )
                .await;
        }
    }

    pub(super) async fn load_recent_artifact_contexts(
        &self,
        conversation_id: &str,
    ) -> Vec<ConversationArtifactContext> {
        let Some(raw) = self
            .storage
            .get(&Self::conversation_recent_artifact_key(conversation_id))
            .await
            .ok()
            .flatten()
        else {
            return Vec::new();
        };
        if let Ok(items) = serde_json::from_slice::<Vec<ConversationArtifactContext>>(&raw) {
            return items;
        }
        serde_json::from_slice::<ConversationArtifactContext>(&raw)
            .ok()
            .into_iter()
            .collect()
    }

    pub(super) async fn load_recent_artifact_context(
        &self,
        conversation_id: &str,
    ) -> Option<ConversationArtifactContext> {
        self.load_recent_artifact_contexts(conversation_id)
            .await
            .into_iter()
            .next()
    }

    pub(super) async fn persist_last_deployed_app_context(
        &self,
        conversation_id: &str,
        app_id: &str,
        title: &str,
        url: &str,
    ) {
        let app = ConversationLastDeployedApp {
            app_id: app_id.to_string(),
            title: title.to_string(),
            url: url.to_string(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Ok(bytes) = serde_json::to_vec(&app) {
            let _ = self
                .storage
                .set(
                    &Self::conversation_last_deployed_app_key(conversation_id),
                    &bytes,
                )
                .await;
        }
        self.persist_conversation_artifact_context(
            conversation_id,
            ConversationArtifactSpec {
                artifact_type: "app",
                artifact_id: app_id,
                title,
                summary: "Recently deployed app in this conversation",
                url: Some(url),
                related_actions: &["service_manage"],
            },
        )
        .await;
    }

    pub(super) fn conversation_artifacts_for_prompt(
        contexts: &[ConversationArtifactContext],
        limit: usize,
    ) -> Vec<serde_json::Value> {
        contexts
            .iter()
            .take(limit)
            .map(|item| {
                serde_json::json!({
                    "artifact_type": item.artifact_type,
                    "artifact_id": item.artifact_id,
                    "title": safe_truncate(&item.title, 120),
                    "summary": safe_truncate(&item.summary, 220),
                    "url": item.url,
                    "related_actions": item.related_actions,
                    "updated_at": item.updated_at,
                })
            })
            .collect()
    }

    pub(super) async fn build_saved_user_facts_context(
        &self,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
        current_message: &str,
    ) -> Option<String> {
        build_saved_user_facts_context_from_storage(
            &self.storage,
            self.embedding_client.as_deref(),
            project_id,
            conversation_id,
            current_message,
        )
        .await
    }
}
