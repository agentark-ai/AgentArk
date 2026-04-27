use super::*;

/// Conversation message for history tracking
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConversationMessage {
    pub role: String, // "user" or "assistant"
    pub content: String,
    pub _timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ConversationDigestSnapshot {
    #[serde(default)]
    user_intents: Vec<String>,
    #[serde(default)]
    assistant_outcomes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ConversationDigest {
    summary: String,
    total_messages: usize,
    updated_at: String,
    #[serde(default)]
    compacted_messages: usize,
    #[serde(default)]
    digest_version: u8,
    #[serde(default)]
    snapshot: ConversationDigestSnapshot,
}

#[derive(Debug, Default)]
pub(super) struct PackedConversationContext {
    pub(super) history: Vec<ConversationMessage>,
    pub(super) total_loaded: usize,
    pub(super) used_chars: usize,
    pub(super) used_digest: bool,
    pub(super) digest: Option<String>,
    pub(super) compacted_messages: usize,
    pub(super) digest_refreshed: bool,
}

impl Agent {
    pub(super) fn conversation_digest_key(conversation_id: &str) -> String {
        format!("conversation_digest:{}", conversation_id)
    }

    pub(super) fn parse_message_timestamp(ts: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(ts)
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now())
    }

    async fn load_conversation_digest(&self, conversation_id: &str) -> Option<ConversationDigest> {
        let key = Self::conversation_digest_key(conversation_id);
        self.storage
            .get(&key)
            .await
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_slice::<ConversationDigest>(&raw).ok())
            .filter(|d| !d.summary.trim().is_empty())
    }

    async fn save_conversation_digest(&self, conversation_id: &str, digest: &ConversationDigest) {
        if let Ok(raw) = serde_json::to_vec(digest) {
            let key = Self::conversation_digest_key(conversation_id);
            let _ = self.storage.set(&key, &raw).await;
        }
    }

    async fn load_conversation_message_count(&self, conversation_id: &str) -> usize {
        self.storage
            .get_conversation(conversation_id)
            .await
            .ok()
            .flatten()
            .map(|conversation| conversation.message_count.max(0) as usize)
            .unwrap_or(0)
    }

    fn normalize_conversation_digest_point_key(text: &str) -> String {
        text.split_whitespace()
            .map(|segment| {
                segment
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .collect::<String>()
            })
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
    }

    fn compact_message_for_digest(role: &str, text: &str) -> Option<String> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        if looks_like_raw_structured_tool_output(trimmed)
            || looks_like_raw_source_or_markup_dump(trimmed)
        {
            return Some(match role {
                "user" => "User shared code, markup, or structured payload for debugging."
                    .to_string(),
                "assistant" => {
                    "Assistant produced raw tool output, generated code, or markup during execution."
                        .to_string()
                }
                _ => "Structured technical content was exchanged earlier in the conversation."
                    .to_string(),
            });
        }
        Some(safe_truncate(trimmed, CONTEXT_DIGEST_POINT_MAX_CHARS))
    }

    fn push_recent_unique_digest_point(points: &mut Vec<String>, point: String) {
        let key = Self::normalize_conversation_digest_point_key(&point);
        if key.is_empty() {
            return;
        }
        if let Some(existing_idx) = points
            .iter()
            .position(|existing| Self::normalize_conversation_digest_point_key(existing) == key)
        {
            points.remove(existing_idx);
        }
        points.push(point);
        while points.len() > CONTEXT_DIGEST_MAX_POINTS_PER_ROLE {
            points.remove(0);
        }
    }

    fn extend_conversation_digest_snapshot(
        snapshot: &mut ConversationDigestSnapshot,
        messages: &[crate::storage::entities::message::Model],
    ) {
        for message in messages {
            let Some(point) = Self::compact_message_for_digest(&message.role, &message.content)
            else {
                continue;
            };
            match message.role.as_str() {
                "user" => Self::push_recent_unique_digest_point(&mut snapshot.user_intents, point),
                "assistant" => {
                    Self::push_recent_unique_digest_point(&mut snapshot.assistant_outcomes, point)
                }
                _ => {}
            }
        }
    }

    fn render_conversation_digest(
        snapshot: &ConversationDigestSnapshot,
        compacted_messages: usize,
    ) -> String {
        let mut out = String::from("Conversation recap from earlier compacted turns.\n");
        out.push_str(&format!(
            "Compacted earlier messages: {}.\n",
            compacted_messages
        ));
        if !snapshot.user_intents.is_empty() {
            out.push_str("User intents and requests:\n");
            for item in &snapshot.user_intents {
                out.push_str("- ");
                out.push_str(item);
                out.push('\n');
            }
        }
        if !snapshot.assistant_outcomes.is_empty() {
            out.push_str("Assistant commitments and outcomes:\n");
            for item in &snapshot.assistant_outcomes {
                out.push_str("- ");
                out.push_str(item);
                out.push('\n');
            }
        }

        safe_truncate(out.trim(), CONTEXT_DIGEST_MAX_CHARS)
    }

    fn build_conversation_digest(
        compacted_messages: &[crate::storage::entities::message::Model],
        total_messages: usize,
    ) -> ConversationDigest {
        let mut snapshot = ConversationDigestSnapshot::default();
        Self::extend_conversation_digest_snapshot(&mut snapshot, compacted_messages);
        let compacted_count = compacted_messages.len();
        ConversationDigest {
            summary: Self::render_conversation_digest(&snapshot, compacted_count),
            total_messages,
            updated_at: chrono::Utc::now().to_rfc3339(),
            compacted_messages: compacted_count,
            digest_version: CONTEXT_DIGEST_VERSION,
            snapshot,
        }
    }

    fn select_salient_older_messages(
        older: &[crate::storage::entities::message::Model],
        query_tokens: &HashSet<String>,
        limit: usize,
    ) -> Vec<crate::storage::entities::message::Model> {
        if older.is_empty() || query_tokens.is_empty() || limit == 0 {
            return Vec::new();
        }

        let mut scored: Vec<(usize, usize)> = older
            .iter()
            .enumerate()
            .map(|(idx, msg)| {
                let overlap = tokenize_lower(&msg.content)
                    .into_iter()
                    .filter(|t| query_tokens.contains(t))
                    .collect::<HashSet<_>>()
                    .len();
                let recency_bonus = (idx * 3) / older.len().max(1);
                (
                    idx,
                    overlap.saturating_mul(10).saturating_add(recency_bonus),
                )
            })
            .filter(|(_, score)| *score > 0)
            .collect();

        scored.sort_by(|a, b| b.1.cmp(&a.1));
        let mut selected_idx: Vec<usize> =
            scored.into_iter().take(limit).map(|(idx, _)| idx).collect();
        selected_idx.sort_unstable();

        selected_idx
            .into_iter()
            .filter_map(|idx| older.get(idx).cloned())
            .collect()
    }

    fn select_recent_older_messages(
        older: &[crate::storage::entities::message::Model],
        limit: usize,
    ) -> Vec<crate::storage::entities::message::Model> {
        if older.is_empty() || limit == 0 {
            return Vec::new();
        }

        let start = older.len().saturating_sub(limit);
        older[start..].to_vec()
    }

    pub(super) async fn build_packed_conversation_context(
        &self,
        conversation_id: &str,
        user_message: &str,
    ) -> PackedConversationContext {
        let mut packed = PackedConversationContext::default();

        let all_messages = match self
            .encrypted_storage
            .get_recent_messages_decrypted(conversation_id, CONTEXT_FETCH_LIMIT)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    "Failed to load conversation history for {}: {}",
                    conversation_id,
                    e
                );
                return packed;
            }
        };
        packed.total_loaded = self
            .load_conversation_message_count(conversation_id)
            .await
            .max(all_messages.len());

        if all_messages.is_empty() {
            return packed;
        }

        let split_at = all_messages.len().saturating_sub(CONTEXT_RECENT_TAIL);
        let (older, recent) = all_messages.split_at(split_at);
        let target_compacted_messages = packed.total_loaded.saturating_sub(CONTEXT_RECENT_TAIL);
        let mut digest_opt =
            self.load_conversation_digest(conversation_id)
                .await
                .filter(|digest| {
                    digest.digest_version == CONTEXT_DIGEST_VERSION
                        && !digest.summary.trim().is_empty()
                        && digest.compacted_messages <= target_compacted_messages
                });
        let refresh_needed = target_compacted_messages >= CONTEXT_MIN_MSGS_FOR_DIGEST
            && digest_opt
                .as_ref()
                .map(|digest| digest.compacted_messages < target_compacted_messages)
                .unwrap_or(true);
        if refresh_needed {
            let mut digest = digest_opt.take().unwrap_or_else(|| ConversationDigest {
                summary: String::new(),
                total_messages: packed.total_loaded,
                updated_at: chrono::Utc::now().to_rfc3339(),
                compacted_messages: 0,
                digest_version: CONTEXT_DIGEST_VERSION,
                snapshot: ConversationDigestSnapshot::default(),
            });
            let mut offset = digest.compacted_messages as u64;
            while offset < target_compacted_messages as u64 {
                let limit = CONTEXT_DIGEST_PAGE_SIZE
                    .min((target_compacted_messages as u64).saturating_sub(offset));
                let page = match self
                    .encrypted_storage
                    .get_messages_decrypted(conversation_id, limit, offset)
                    .await
                {
                    Ok(messages) => messages,
                    Err(error) => {
                        tracing::warn!(
                            "Failed to compact conversation history for {} at offset {}: {}",
                            conversation_id,
                            offset,
                            error
                        );
                        break;
                    }
                };
                if page.is_empty() {
                    break;
                }
                Self::extend_conversation_digest_snapshot(&mut digest.snapshot, &page);
                offset = offset.saturating_add(page.len() as u64);
            }
            digest.compacted_messages = offset as usize;
            digest.total_messages = packed.total_loaded;
            digest.updated_at = chrono::Utc::now().to_rfc3339();
            digest.digest_version = CONTEXT_DIGEST_VERSION;
            digest.summary =
                Self::render_conversation_digest(&digest.snapshot, digest.compacted_messages);
            if !digest.summary.trim().is_empty() {
                self.save_conversation_digest(conversation_id, &digest)
                    .await;
                packed.digest_refreshed = true;
                digest_opt = Some(digest);
            }
        } else if digest_opt.is_none() && older.len() >= CONTEXT_MIN_MSGS_FOR_DIGEST {
            let digest = Self::build_conversation_digest(older, packed.total_loaded);
            if !digest.summary.trim().is_empty() {
                self.save_conversation_digest(conversation_id, &digest)
                    .await;
                packed.digest_refreshed = true;
                digest_opt = Some(digest);
            }
        }

        let mut selected: Vec<ConversationMessage> = Vec::new();
        let mut seen_ids: HashSet<String> = HashSet::new();

        if let Some(digest) = digest_opt.as_ref().filter(|d| !d.summary.trim().is_empty()) {
            packed.used_digest = true;
            packed.digest = Some(safe_truncate(&digest.summary, CONTEXT_DIGEST_MAX_CHARS));
            packed.compacted_messages = digest.compacted_messages;
        }

        let query_tokens: HashSet<String> = tokenize_lower(user_message).into_iter().collect();
        let recent_older =
            Self::select_recent_older_messages(older, CONTEXT_SHORT_FOLLOWUP_OLDER_LIMIT);
        for msg in recent_older {
            if !seen_ids.insert(msg.id.clone()) {
                continue;
            }
            selected.push(ConversationMessage {
                role: msg.role,
                content: safe_truncate(&msg.content, CONTEXT_MAX_MESSAGE_CHARS),
                _timestamp: Self::parse_message_timestamp(&msg.timestamp),
            });
        }
        let salient =
            Self::select_salient_older_messages(older, &query_tokens, CONTEXT_SALIENT_OLDER_LIMIT);
        for msg in salient {
            if !seen_ids.insert(msg.id.clone()) {
                continue;
            }
            selected.push(ConversationMessage {
                role: msg.role,
                content: safe_truncate(&msg.content, CONTEXT_MAX_MESSAGE_CHARS),
                _timestamp: Self::parse_message_timestamp(&msg.timestamp),
            });
        }

        for msg in recent {
            if !seen_ids.insert(msg.id.clone()) {
                continue;
            }
            selected.push(ConversationMessage {
                role: msg.role.clone(),
                content: safe_truncate(&msg.content, CONTEXT_MAX_MESSAGE_CHARS),
                _timestamp: Self::parse_message_timestamp(&msg.timestamp),
            });
        }

        // Keep recent continuity first, then shrink from oldest non-summary lines.
        let mut total_chars: usize = selected.iter().map(|m| m.content.len()).sum();
        while total_chars > CONTEXT_MAX_CHARS && selected.len() > 4 {
            let removable = selected.len().saturating_sub(4);
            if removable == 0 {
                break;
            }
            total_chars = total_chars.saturating_sub(selected[0].content.len());
            selected.remove(0);
        }

        packed.used_chars = total_chars;
        packed.history = selected;
        packed
    }

    /// Resolve conversation for this request, creating one if needed for implicit chat turns.
    /// Returns `(conversation_id, is_new_conversation)`.
    pub(super) async fn resolve_conversation_id(
        &self,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        message_preview: &str,
    ) -> Result<(String, bool)> {
        let now = chrono::Utc::now().to_rfc3339();
        let scope = self.conversation_scope_mode().await;
        let conv_key = scope.conversation_key(channel, project_id);
        let create_fresh_conversation_without_explicit_id = matches!(channel, "http" | "web");

        let create_conversation = |id: String| crate::storage::entities::conversation::Model {
            id: id.clone(),
            title: safe_truncate(message_preview, 50),
            channel: channel.to_string(),
            project_id: project_id.map(|s| s.to_string()),
            created_at: now.clone(),
            updated_at: now.clone(),
            message_count: 0,
            archived: false,
            starred: false,
        };

        let stored_id = self
            .storage
            .get(&conv_key)
            .await
            .ok()
            .flatten()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .filter(|id| !id.is_empty());

        if let Some(cid) = conversation_id {
            let is_new = match self.storage.get_conversation(cid).await {
                Ok(Some(existing)) => existing.message_count == 0 || existing.title == "New Chat",
                Ok(None) => {
                    return Err(anyhow::anyhow!("Conversation not found"));
                }
                Err(error) => {
                    return Err(anyhow::anyhow!(
                        "Conversation lookup failed for '{}': {}",
                        cid,
                        error
                    ));
                }
            };
            let _ = self.storage.set(&conv_key, cid.as_bytes()).await;
            return Ok((cid.to_string(), is_new));
        }

        if !create_fresh_conversation_without_explicit_id {
            if let Some(id) = stored_id {
                match self.storage.get_conversation(&id).await {
                    Ok(Some(existing)) => {
                        let is_new = existing.message_count == 0 || existing.title == "New Chat";
                        return Ok((id, is_new));
                    }
                    Ok(None) | Err(_) => {
                        // Stale pointer (deleted/missing conversation) -> create new one.
                    }
                }
            }
        }

        let new_id = uuid::Uuid::new_v4().to_string();
        let conv = create_conversation(new_id.clone());
        let _ = self.storage.create_conversation(&conv).await;
        if !create_fresh_conversation_without_explicit_id {
            let _ = self.storage.set(&conv_key, new_id.as_bytes()).await;
        }
        Ok((new_id, true))
    }

    pub async fn ensure_conversation_id_for_request(
        &self,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        message_preview: &str,
    ) -> Result<String> {
        let preview = safe_truncate(message_preview, 80);
        let (conversation_id, _) = self
            .resolve_conversation_id(channel, conversation_id, project_id, &preview)
            .await?;
        Ok(conversation_id)
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

    pub(super) async fn persist_conversation_artifact_context(
        &self,
        conversation_id: &str,
        artifact: ConversationArtifactSpec<'_>,
    ) {
        let cid = conversation_id.trim();
        let artifact_type = artifact.artifact_type.trim();
        let artifact_id = artifact.artifact_id.trim();
        if cid.is_empty() || artifact_type.is_empty() || artifact_id.is_empty() {
            return;
        }
        let payload = ConversationArtifactContext {
            artifact_type: artifact_type.to_string(),
            artifact_id: artifact_id.to_string(),
            title: artifact.title.to_string(),
            summary: artifact.summary.to_string(),
            url: artifact.url.unwrap_or_default().to_string(),
            related_actions: artifact
                .related_actions
                .iter()
                .map(ToString::to_string)
                .collect(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        self.persist_conversation_artifact_context_payload(cid, payload)
            .await;
    }

    pub(super) async fn persist_conversation_artifact_context_payload(
        &self,
        conversation_id: &str,
        mut payload: ConversationArtifactContext,
    ) {
        let cid = conversation_id.trim();
        payload.artifact_type = safe_truncate(payload.artifact_type.trim(), 60);
        payload.artifact_id = safe_truncate(payload.artifact_id.trim(), 120);
        if cid.is_empty() || payload.artifact_type.is_empty() || payload.artifact_id.is_empty() {
            return;
        }
        payload.title = safe_truncate(payload.title.trim(), 120);
        payload.summary = safe_truncate(payload.summary.trim(), 240);
        payload.url = safe_truncate(payload.url.trim(), 300);
        payload.related_actions = payload
            .related_actions
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .fold(Vec::new(), |mut acc, value| {
                if !acc.iter().any(|existing| existing == &value) {
                    acc.push(value);
                }
                acc
            });
        payload.updated_at = chrono::Utc::now().to_rfc3339();
        let key = Self::conversation_recent_artifact_key(cid);
        let mut artifacts = self
            .load_recent_artifact_contexts_raw(cid)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|existing| {
                !existing
                    .artifact_type
                    .eq_ignore_ascii_case(payload.artifact_type.as_str())
                    || existing.artifact_id.trim() != payload.artifact_id.trim()
            })
            .collect::<Vec<_>>();
        artifacts.insert(0, payload);
        if artifacts.len() > CONVERSATION_RECENT_ARTIFACT_LIMIT {
            artifacts.truncate(CONVERSATION_RECENT_ARTIFACT_LIMIT);
        }
        if let Ok(serialized) = serde_json::to_vec(&artifacts) {
            let _ = self.storage.set(&key, &serialized).await;
        }
    }

    async fn load_recent_artifact_contexts_raw(
        &self,
        conversation_id: &str,
    ) -> Option<Vec<ConversationArtifactContext>> {
        let cid = conversation_id.trim();
        if cid.is_empty() {
            return None;
        }
        let key = Self::conversation_recent_artifact_key(cid);
        let raw = self.storage.get(&key).await.ok().flatten()?;
        serde_json::from_slice::<Vec<ConversationArtifactContext>>(&raw)
            .ok()
            .or_else(|| {
                serde_json::from_slice::<ConversationArtifactContext>(&raw)
                    .ok()
                    .map(|artifact| vec![artifact])
            })
    }

    pub(super) async fn load_recent_artifact_contexts(
        &self,
        conversation_id: &str,
    ) -> Vec<ConversationArtifactContext> {
        let cid = conversation_id.trim();
        if cid.is_empty() {
            return Vec::new();
        }
        let mut parsed = self
            .load_recent_artifact_contexts_raw(cid)
            .await
            .unwrap_or_default();
        if parsed.is_empty() {
            let legacy_key = Self::conversation_last_deployed_app_key(cid);
            parsed = self
                .storage
                .get(&legacy_key)
                .await
                .ok()
                .flatten()
                .and_then(|legacy_raw| {
                    serde_json::from_slice::<ConversationLastDeployedApp>(&legacy_raw)
                        .ok()
                        .map(|legacy| {
                            vec![ConversationArtifactContext {
                                artifact_type: "app".to_string(),
                                artifact_id: legacy.app_id,
                                title: legacy.title,
                                summary: "Recently deployed app in this conversation".to_string(),
                                url: legacy.url,
                                related_actions: vec![
                                    "app_inspect".to_string(),
                                    "file_read".to_string(),
                                    "file_write".to_string(),
                                    "app_restart".to_string(),
                                ],
                                updated_at: legacy.updated_at,
                            }]
                        })
                })
                .unwrap_or_default();
        }

        let mut fresh = Vec::new();
        let now = chrono::Utc::now();
        for artifact in parsed {
            let updated_at = chrono::DateTime::parse_from_rfc3339(artifact.updated_at.as_str())
                .ok()
                .map(|dt| dt.with_timezone(&chrono::Utc));
            let age_secs = updated_at
                .map(|dt| (now - dt).num_seconds())
                .unwrap_or(i64::MAX);
            if age_secs > APP_FOLLOWUP_CONTEXT_MAX_AGE_SECS {
                continue;
            }
            if self.recent_artifact_still_exists(&artifact).await {
                fresh.push(artifact);
            }
        }
        fresh.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        if fresh.len() > CONVERSATION_RECENT_ARTIFACT_LIMIT {
            fresh.truncate(CONVERSATION_RECENT_ARTIFACT_LIMIT);
        }

        let key = Self::conversation_recent_artifact_key(cid);
        if fresh.is_empty() {
            let _ = self.storage.delete(&key).await;
            let legacy_key = Self::conversation_last_deployed_app_key(cid);
            let _ = self.storage.delete(&legacy_key).await;
        } else if let Ok(serialized) = serde_json::to_vec(&fresh) {
            let _ = self.storage.set(&key, &serialized).await;
        }

        fresh
    }

    pub(crate) async fn persist_last_deployed_app_context(
        &self,
        conversation_id: &str,
        app_id: &str,
        title: &str,
        url: &str,
    ) {
        let cid = conversation_id.trim();
        let app_id = app_id.trim();
        if cid.is_empty() || app_id.is_empty() {
            return;
        }

        let payload = ConversationLastDeployedApp {
            app_id: app_id.to_string(),
            title: safe_truncate(title.trim(), 120),
            url: safe_truncate(url.trim(), 300),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Ok(serialized) = serde_json::to_vec(&payload) {
            let key = Self::conversation_last_deployed_app_key(cid);
            let _ = self.storage.set(&key, &serialized).await;
        }

        self.persist_conversation_artifact_context(
            cid,
            ConversationArtifactSpec {
                artifact_type: "app",
                artifact_id: app_id,
                title,
                summary: "Recently deployed app in this conversation",
                url: Some(url),
                related_actions: &["app_inspect", "file_read", "file_write", "app_restart"],
            },
        )
        .await;
    }

    async fn recent_artifact_still_exists(&self, artifact: &ConversationArtifactContext) -> bool {
        let artifact_id = artifact.artifact_id.trim();
        if artifact_id.is_empty() {
            return false;
        }

        match artifact.artifact_type.trim().to_ascii_lowercase().as_str() {
            "app" => self.app_registry.list().await.iter().any(|app| {
                app.get("id")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    == Some(artifact_id)
            }),
            "watcher" => match uuid::Uuid::parse_str(artifact_id) {
                Ok(watcher_id) => self.watcher_manager.get(watcher_id).await.is_some(),
                Err(_) => false,
            },
            "task" => {
                let tasks = self.tasks.read().await;
                tasks
                    .all()
                    .iter()
                    .any(|task| task.id.to_string() == artifact_id)
            }
            "background_session" => self.background_sessions.get(artifact_id).await.is_some(),
            "goal" => {
                let tasks = self.tasks.read().await;
                tasks.all().iter().any(|task| {
                    task.action == "goal"
                        && (task.id.to_string() == artifact_id
                            || task
                                .arguments
                                .get("goal_id")
                                .and_then(|value| value.as_str())
                                .map(str::trim)
                                == Some(artifact_id))
                })
            }
            _ => true,
        }
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
}
