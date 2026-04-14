use super::*;

static MOLTBOOK_RUN_ACTIVE: AtomicBool = AtomicBool::new(false);

struct MoltbookRunGuard;

impl Drop for MoltbookRunGuard {
    fn drop(&mut self) {
        MOLTBOOK_RUN_ACTIVE.store(false, Ordering::Release);
    }
}

fn try_start_moltbook_run() -> Option<MoltbookRunGuard> {
    if MOLTBOOK_RUN_ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        Some(MoltbookRunGuard)
    } else {
        None
    }
}

pub(super) fn is_moltbook_running() -> bool {
    MOLTBOOK_RUN_ACTIVE.load(Ordering::Relaxed)
}

pub(super) const MOLTBOOK_SETTINGS_KEY: &str = "moltbook_settings_v1";
pub(super) const MOLTBOOK_ACTIVITY_LOG_KEY: &str = "moltbook_activity_log_v1";
pub(super) const MOLTBOOK_LAST_RUN_KEY: &str = "moltbook_last_run_v1";
pub(super) const MOLTBOOK_NEXT_RUN_KEY: &str = "moltbook_next_run_v1";
pub(super) const MOLTBOOK_DEFER_COUNT_KEY: &str = "moltbook_defer_count_v1";
pub(super) const MOLTBOOK_LAST_STATUS_KEY: &str = "moltbook_last_status_v1";
pub(super) const MOLTBOOK_LAST_POST_KEY: &str = "moltbook_last_post_v1";
pub(super) const MOLTBOOK_LAST_COMMENT_KEY: &str = "moltbook_last_comment_v1";
pub(super) const MOLTBOOK_LAST_UPVOTE_KEY: &str = "moltbook_last_upvote_v1";
pub(super) const MOLTBOOK_LAST_ENGAGEMENT_KEY: &str = "moltbook_last_engagement_v1";
pub(super) const MOLTBOOK_LAST_RUN_STATS_KEY: &str = "moltbook_last_run_stats_v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct MoltbookSettings {
    pub(super) enabled: bool,
    pub(super) mode: String,           // off | read_only | assist | autopost
    pub(super) sync_frequency: String, // every_minute | every_5_minutes | every_10_minutes | every_30_minutes | hourly | every_3_hours | every_6_hours | every_12_hours | daily | weekly
    pub(super) write_enabled: bool,
    pub(super) defer_when_busy: bool,
    pub(super) model_slot_id: Option<String>,
}

impl Default for MoltbookSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: "autopost".to_string(),
            sync_frequency: "every_12_hours".to_string(),
            write_enabled: true,
            defer_when_busy: true,
            model_slot_id: None,
        }
    }
}

pub(super) fn has_moltbook_api_key(
    config_dir: &std::path::Path,
    data_dir: Option<&std::path::Path>,
) -> bool {
    if std::env::var("MOLTBOOK_API_KEY")
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return true;
    }
    crate::core::config::SecureConfigManager::new_with_data_dir(config_dir, data_dir)
        .ok()
        .and_then(|manager| manager.get_custom_secret("moltbook_api_key").ok().flatten())
        .is_some_and(|value| !value.trim().is_empty())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MoltbookActivityEvent {
    id: String,
    run_id: String,
    timestamp: String,
    level: String,
    action: String,
    details: serde_json::Value,
}

pub(super) fn normalize_moltbook_mode(mode: &str) -> String {
    match mode.trim().to_lowercase().as_str() {
        "off" => "off".to_string(),
        "read_only" => "read_only".to_string(),
        "assist" => "assist".to_string(),
        "engage" => "autopost".to_string(),
        "autopost" => "autopost".to_string(),
        _ => "autopost".to_string(),
    }
}

pub(super) fn normalize_moltbook_frequency(freq: &str) -> String {
    let trimmed = freq.trim();
    match trimmed.to_lowercase().as_str() {
        "every_minute" | "every_1_minute" | "every_1m" | "1m" | "1min" | "minute" => {
            "every_minute".to_string()
        }
        "every_5_minutes" | "every_5_minute" | "every_5m" | "5m" | "5min" => {
            "every_5_minutes".to_string()
        }
        "every_10_minutes" | "every_10_minute" | "every_10m" | "10m" | "10min" => {
            "every_10_minutes".to_string()
        }
        "every_30_minutes" | "every_30_minute" | "every_30m" | "30m" | "30min" => {
            "every_30_minutes".to_string()
        }
        "hourly" | "every_hour" | "every_1_hour" | "every_1h" => "hourly".to_string(),
        "every_3_hours" | "every_3h" | "3h" => "every_3_hours".to_string(),
        "every_6_hours" | "every_6h" | "6h" => "every_6_hours".to_string(),
        "every_12_hours" | "every_12h" | "12h" | "twice_daily" => "every_12_hours".to_string(),
        "daily" => "daily".to_string(),
        "weekly" | "once_a_week" => "weekly".to_string(),
        _ if trimmed.parse::<cron::Schedule>().is_ok() => trimmed.to_string(),
        _ => "every_12_hours".to_string(),
    }
}

pub(super) fn normalize_moltbook_model_slot_id(slot_id: Option<&str>) -> Option<String> {
    slot_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

pub(super) async fn load_moltbook_settings(storage: &crate::storage::Storage) -> MoltbookSettings {
    let raw = match storage.get(MOLTBOOK_SETTINGS_KEY).await {
        Ok(Some(v)) => v,
        _ => return MoltbookSettings::default(),
    };
    match serde_json::from_slice::<MoltbookSettings>(&raw) {
        Ok(mut settings) => {
            settings.mode = normalize_moltbook_mode(&settings.mode);
            settings.sync_frequency = normalize_moltbook_frequency(&settings.sync_frequency);
            settings.model_slot_id =
                normalize_moltbook_model_slot_id(settings.model_slot_id.as_deref());
            settings
        }
        Err(_) => MoltbookSettings::default(),
    }
}

pub(super) async fn save_moltbook_settings(
    storage: &crate::storage::Storage,
    settings: &MoltbookSettings,
) -> Result<(), String> {
    let mut normalized = settings.clone();
    normalized.mode = normalize_moltbook_mode(&normalized.mode);
    normalized.sync_frequency = normalize_moltbook_frequency(&normalized.sync_frequency);
    normalized.model_slot_id =
        normalize_moltbook_model_slot_id(normalized.model_slot_id.as_deref());
    let bytes = serde_json::to_vec(&normalized).map_err(|e| e.to_string())?;
    storage
        .set(MOLTBOOK_SETTINGS_KEY, &bytes)
        .await
        .map_err(|e| e.to_string())
}

async fn load_moltbook_activity(storage: &crate::storage::Storage) -> Vec<MoltbookActivityEvent> {
    let raw = match storage.get(MOLTBOOK_ACTIVITY_LOG_KEY).await {
        Ok(Some(v)) => v,
        _ => return vec![],
    };
    serde_json::from_slice::<Vec<MoltbookActivityEvent>>(&raw).unwrap_or_default()
}

pub(super) async fn append_moltbook_activity(
    storage: &crate::storage::Storage,
    run_id: &str,
    level: &str,
    action: &str,
    details: serde_json::Value,
) {
    let mut events = load_moltbook_activity(storage).await;
    events.push(MoltbookActivityEvent {
        id: uuid::Uuid::new_v4().to_string(),
        run_id: run_id.to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        level: level.to_string(),
        action: action.to_string(),
        details,
    });
    if events.len() > 500 {
        let drop = events.len() - 500;
        events.drain(0..drop);
    }
    if let Ok(bytes) = serde_json::to_vec(&events) {
        let _ = storage.set(MOLTBOOK_ACTIVITY_LOG_KEY, &bytes).await;
    }
}

fn push_moltbook_run_link(
    out: &mut Vec<serde_json::Value>,
    seen: &mut HashSet<String>,
    label: impl Into<String>,
    url_like: Option<String>,
) {
    let Some(url) = url_like
        .map(|value| value.trim().to_string())
        .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
    else {
        return;
    };
    let label = label.into();
    let key = format!("{}|{}", label, url);
    if !seen.insert(key) {
        return;
    }
    out.push(serde_json::json!({
        "label": label,
        "url": url,
    }));
}

fn moltbook_human_post_url(post_id: &str) -> String {
    format!("https://www.moltbook.com/post/{}", post_id)
}

fn moltbook_post_api_url(post_id: &str) -> String {
    format!("https://www.moltbook.com/api/v1/posts/{}", post_id)
}

fn moltbook_next_run_at(
    schedule: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> chrono::DateTime<chrono::Utc> {
    let normalized = normalize_moltbook_frequency(schedule);
    let fallback_secs = match normalized.as_str() {
        "every_minute" => 60,
        "every_5_minutes" => 5 * 60,
        "every_10_minutes" => 10 * 60,
        "every_30_minutes" => 30 * 60,
        "hourly" => 60 * 60,
        "every_3_hours" => 3 * 60 * 60,
        "every_6_hours" => 6 * 60 * 60,
        "every_12_hours" => 12 * 60 * 60,
        "daily" => 24 * 60 * 60,
        "weekly" => 7 * 24 * 60 * 60,
        _ => 12 * 60 * 60,
    };
    if let Ok(parsed) = normalized.parse::<cron::Schedule>() {
        if let Some(next) = parsed.upcoming(chrono::Utc).next() {
            return next;
        }
    }
    now + chrono::Duration::seconds(fallback_secs)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct MoltbookEngagementPlan {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    actions: Vec<MoltbookEngagementAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct MoltbookEngagementAction {
    #[serde(default)]
    action: String,
    #[serde(default)]
    post_id: Option<String>,
    #[serde(default)]
    parent_id: Option<String>,
    #[serde(default)]
    submolt: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct MoltbookMemoryInsights {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    insights: Vec<String>,
}

fn moltbook_write_mode_enabled(settings: &MoltbookSettings, trigger: &str) -> bool {
    if !settings.write_enabled {
        return false;
    }

    match settings.mode.as_str() {
        "autopost" => true,
        "assist" => trigger == "manual",
        _ => false,
    }
}

fn moltbook_text_preview(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let compact = compact.trim();
    if compact.chars().count() <= max_chars {
        compact.to_string()
    } else {
        compact
            .chars()
            .take(max_chars.saturating_sub(3))
            .collect::<String>()
            + "..."
    }
}

fn moltbook_post_preview(post: &serde_json::Value) -> String {
    const CANDIDATE_KEYS: &[&str] = &["content", "body", "text", "self_text", "excerpt", "summary"];
    for key in CANDIDATE_KEYS {
        if let Some(value) = post.get(*key).and_then(|v| v.as_str()) {
            let preview = moltbook_text_preview(value, 220);
            if !preview.is_empty() {
                return preview;
            }
        }
    }
    String::new()
}

fn moltbook_memory_feed_items(
    feed_posts_raw: &[serde_json::Value],
    engaged_post_ids: &[String],
) -> Vec<serde_json::Value> {
    let engaged_ids = engaged_post_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect::<HashSet<_>>();

    feed_posts_raw
        .iter()
        .take(6)
        .filter_map(|post| {
            let post_id = post
                .get("id")
                .and_then(|value| value.as_str())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            let title = post
                .get("title")
                .and_then(|value| value.as_str())
                .map(|value| value.trim().to_string())
                .unwrap_or_default();
            let submolt = post
                .get("submolt")
                .and_then(|value| {
                    value
                        .get("name")
                        .and_then(|name| name.as_str())
                        .or_else(|| value.as_str())
                })
                .map(|value| value.trim().to_string())
                .unwrap_or_else(|| "general".to_string());
            let preview = moltbook_post_preview(post);
            if title.is_empty() && preview.is_empty() {
                return None;
            }
            let engaged = post_id
                .as_ref()
                .map(|id| engaged_ids.contains(id))
                .unwrap_or(false);
            Some(serde_json::json!({
                "id": post_id,
                "engaged": engaged,
                "title": title,
                "submolt": submolt,
                "preview": preview
            }))
        })
        .collect()
}

fn format_moltbook_memory_text(insights: &MoltbookMemoryInsights) -> String {
    let mut lines = Vec::new();
    let summary = insights.summary.trim();
    if summary.is_empty() {
        lines.push("Moltbook community learnings:".to_string());
    } else {
        lines.push(format!("Moltbook community learnings: {}", summary));
    }
    for insight in insights.insights.iter().take(4) {
        lines.push(format!("- {}", insight));
    }
    lines.join("\n")
}

fn format_moltbook_external_knowledge_title(insights: &MoltbookMemoryInsights) -> String {
    let seed = if insights.summary.trim().is_empty() {
        insights
            .insights
            .first()
            .map(|value| value.as_str())
            .unwrap_or("Community learning")
    } else {
        insights.summary.trim()
    };
    format!("Moltbook: {}", moltbook_text_preview(seed, 96))
}

fn sanitize_moltbook_public_text(text: &str) -> crate::security::OutboundPrivacyTextResult {
    crate::security::check_outbound_text(text, &crate::security::OutboundPrivacyPolicy::default())
}

fn moltbook_privacy_details(
    result: &crate::security::OutboundPrivacyTextResult,
) -> serde_json::Value {
    serde_json::json!({
        "decision": result.decision,
        "reasons": result.reasons,
        "redactions": result.redactions
    })
}

fn sanitize_moltbook_memory_insights(
    insights: &MoltbookMemoryInsights,
) -> (Option<MoltbookMemoryInsights>, serde_json::Value) {
    let mut combined_reasons = Vec::new();
    let mut combined_redactions = Vec::new();
    let mut dropped_insight_count = 0usize;
    let mut summary_blocked = false;

    let push_unique = |items: &mut Vec<String>, value: &str| {
        let trimmed = value.trim();
        if trimmed.is_empty() || items.iter().any(|existing| existing == trimmed) {
            return;
        }
        items.push(trimmed.to_string());
    };

    let summary_result = sanitize_moltbook_public_text(&insights.summary);
    for reason in &summary_result.reasons {
        push_unique(&mut combined_reasons, reason);
    }
    for redaction in &summary_result.redactions {
        push_unique(&mut combined_redactions, redaction);
    }

    let sanitized_summary = if matches!(
        summary_result.decision,
        crate::security::OutboundPrivacyDecision::Block
    ) {
        summary_blocked = true;
        String::new()
    } else {
        summary_result.sanitized_text.trim().to_string()
    };

    let mut sanitized_insights = Vec::new();
    for insight in &insights.insights {
        let result = sanitize_moltbook_public_text(insight);
        for reason in &result.reasons {
            push_unique(&mut combined_reasons, reason);
        }
        for redaction in &result.redactions {
            push_unique(&mut combined_redactions, redaction);
        }
        if matches!(
            result.decision,
            crate::security::OutboundPrivacyDecision::Block
        ) {
            dropped_insight_count += 1;
            continue;
        }
        let sanitized = result.sanitized_text.trim().to_string();
        if !sanitized.is_empty() {
            sanitized_insights.push(sanitized);
        }
    }

    let privacy = serde_json::json!({
        "fenced_external_source": true,
        "summary_blocked": summary_blocked,
        "dropped_insight_count": dropped_insight_count,
        "reasons": combined_reasons,
        "redactions": combined_redactions
    });

    if sanitized_summary.is_empty() && sanitized_insights.is_empty() {
        (None, privacy)
    } else {
        (
            Some(MoltbookMemoryInsights {
                summary: sanitized_summary,
                insights: sanitized_insights,
            }),
            privacy,
        )
    }
}

async fn persist_moltbook_external_knowledge(
    storage: &crate::storage::Storage,
    insights: &MoltbookMemoryInsights,
) -> std::result::Result<crate::storage::entities::knowledge_item::Model, String> {
    let title = format_moltbook_external_knowledge_title(insights);
    let content = format_moltbook_memory_text(insights);
    storage
        .create_knowledge_item(
            &title,
            &content,
            Some("moltbook"),
            None,
            Some("external-source,moltbook,community-learning,learning-fenced"),
            None,
        )
        .await
        .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Copy)]
struct MoltbookMemoryDistillationInput<'a> {
    trigger: &'a str,
    settings: &'a MoltbookSettings,
    feed_posts_raw: &'a [serde_json::Value],
    decision_summary: &'a str,
    engaged_post_ids: &'a [String],
    comment_count: usize,
    upvote_count: usize,
    post_count: usize,
    engagement_failures: &'a [String],
    posted_url: Option<&'a str>,
}

async fn distill_moltbook_memory_insights(
    state: &AppState,
    input: MoltbookMemoryDistillationInput<'_>,
) -> std::result::Result<Option<MoltbookMemoryInsights>, String> {
    let MoltbookMemoryDistillationInput {
        trigger,
        settings,
        feed_posts_raw,
        decision_summary,
        engaged_post_ids,
        comment_count,
        upvote_count,
        post_count,
        engagement_failures,
        posted_url,
    } = input;
    if feed_posts_raw.is_empty() && comment_count + upvote_count + post_count == 0 {
        return Ok(None);
    }

    let feed_items = moltbook_memory_feed_items(feed_posts_raw, engaged_post_ids);
    if feed_items.is_empty()
        && decision_summary.trim().is_empty()
        && comment_count + upvote_count + post_count == 0
    {
        return Ok(None);
    }

    let (llm, agent_name) = {
        let agent = state.agent.read().await;
        (
            agent
                .llm_for_explicit_slot_or_primary(settings.model_slot_id.as_deref())
                .clone(),
            agent.config.name.clone(),
        )
    };

    let payload = serde_json::json!({
        "trigger": trigger,
        "mode": settings.mode.clone(),
        "agent_name": agent_name,
        "decision_summary": moltbook_text_preview(decision_summary, 240),
        "engagement": {
            "comment_count": comment_count,
            "upvote_count": upvote_count,
            "post_count": post_count,
            "engaged_post_ids": engaged_post_ids,
            "failures": engagement_failures.iter().take(3).cloned().collect::<Vec<_>>(),
            "created_post_url": posted_url
        },
        "feed": feed_items
    });

    let prompt = crate::branding::render_template(
        r#"You are extracting durable semantic memory from __PRODUCT_NAME__'s Moltbook run.
Return strict JSON only with this shape:
{"summary":"short string","insights":["insight 1","insight 2"]}

Rules:
- Capture only reusable learnings from the community feed or the agent's engagement choices.
- Prefer stable patterns, recurring tensions, practical lessons, or high-signal open questions.
- Ignore routine operational details, one-off status updates, IDs, timestamps, and pure process notes.
- Do not include usernames, post IDs, URLs, or private data.
- Keep 0-4 insights, each one sentence and under 180 characters.
- If nothing is worth saving for future conversations, return an empty insights array."#,
    );

    let response = llm
        .chat(&prompt, &payload.to_string(), &[], &[])
        .await
        .map_err(|e| e.to_string())?;
    {
        let agent = state.agent.read().await;
        agent
            .record_llm_usage("moltbook", "memory_distiller", &response)
            .await;
    }

    let parsed = extract_json(&response.content)
        .and_then(|value| serde_json::from_value::<MoltbookMemoryInsights>(value).ok())
        .ok_or_else(|| "Could not parse Moltbook memory distiller response.".to_string())?;

    let mut seen = HashSet::new();
    let insights = parsed
        .insights
        .into_iter()
        .map(|value| moltbook_text_preview(value.trim(), 180))
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.to_ascii_lowercase()))
        .take(4)
        .collect::<Vec<_>>();

    if insights.is_empty() {
        return Ok(None);
    }

    Ok(Some(MoltbookMemoryInsights {
        summary: moltbook_text_preview(parsed.summary.trim(), 180),
        insights,
    }))
}

async fn build_moltbook_recent_activity_context(
    storage: &crate::storage::Storage,
) -> serde_json::Value {
    let events = load_moltbook_activity(storage).await;
    let mut recent_runs = Vec::new();
    let mut recent_posts = Vec::new();
    let mut recent_comment_count = 0usize;
    let mut recent_upvote_count = 0usize;
    let mut recent_post_count = 0usize;
    let mut runs_since_last_post = 0usize;
    let mut saw_recent_post = false;

    for event in events.iter().rev() {
        let action = event.action.trim().to_lowercase();
        let details = event.details.as_object().cloned().unwrap_or_default();

        if action == "run_completed" && recent_runs.len() < 12 {
            let comment_count = details
                .get("comment_count")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as usize;
            let upvote_count = details
                .get("upvote_count")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as usize;
            let post_count = details
                .get("post_count")
                .and_then(|value| value.as_u64())
                .unwrap_or_else(|| {
                    if details
                        .get("posted")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
                    {
                        1
                    } else {
                        0
                    }
                }) as usize;

            recent_comment_count += comment_count;
            recent_upvote_count += upvote_count;
            recent_post_count += post_count;

            if !saw_recent_post {
                if post_count > 0 {
                    saw_recent_post = true;
                } else {
                    runs_since_last_post += 1;
                }
            }

            recent_runs.push(serde_json::json!({
                "run_id": event.run_id,
                "timestamp": event.timestamp,
                "trigger": details.get("trigger").cloned().unwrap_or(serde_json::Value::Null),
                "read_count": details.get("read_count").cloned().unwrap_or(serde_json::json!(0)),
                "comment_count": comment_count,
                "upvote_count": upvote_count,
                "post_count": post_count,
                "decision_summary": details.get("decision_summary").cloned().unwrap_or(serde_json::Value::Null),
            }));
        } else if action == "post_created" && recent_posts.len() < 5 {
            let request = details
                .get("request")
                .and_then(|value| value.as_object())
                .cloned()
                .unwrap_or_default();
            recent_posts.push(serde_json::json!({
                "timestamp": event.timestamp,
                "title": request.get("title").cloned().unwrap_or(serde_json::Value::Null),
                "submolt": request.get("submolt").cloned().unwrap_or(serde_json::Value::Null),
                "post_url": details.get("post_url").cloned().unwrap_or(serde_json::Value::Null),
                "reason": details.get("reason").cloned().unwrap_or(serde_json::Value::Null),
            }));
        }
    }

    serde_json::json!({
        "recent_run_count": recent_runs.len(),
        "recent_comment_count": recent_comment_count,
        "recent_upvote_count": recent_upvote_count,
        "recent_post_count": recent_post_count,
        "runs_since_last_post": if recent_runs.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::json!(runs_since_last_post)
        },
        "recent_runs": recent_runs,
        "recent_posts": recent_posts,
    })
}

fn build_moltbook_submolt_context(feed_posts_raw: &[serde_json::Value]) -> serde_json::Value {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut examples: HashMap<String, Vec<String>> = HashMap::new();

    for post in feed_posts_raw.iter().take(20) {
        let submolt = post
            .get("submolt")
            .and_then(|value| {
                value
                    .get("name")
                    .and_then(|name| name.as_str())
                    .or_else(|| value.as_str())
            })
            .unwrap_or("general")
            .trim()
            .to_string();
        *counts.entry(submolt.clone()).or_insert(0) += 1;
        if let Some(title) = post
            .get("title")
            .and_then(|value| value.as_str())
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            let bucket = examples.entry(submolt).or_default();
            if bucket.len() < 3 {
                bucket.push(title.to_string());
            }
        }
    }

    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    serde_json::json!({
        "visible_submolts": ranked
            .iter()
            .map(|(name, count)| serde_json::json!({
                "name": name,
                "count": count,
                "sample_titles": examples.get(name).cloned().unwrap_or_default()
            }))
            .collect::<Vec<_>>()
    })
}

pub(super) async fn run_moltbook_cycle(state: &AppState, trigger: &str) -> serde_json::Value {
    let Some(run_guard) = try_start_moltbook_run() else {
        return serde_json::json!({
            "status": "running",
            "message": "Moltbook run already in progress"
        });
    };
    run_moltbook_cycle_with_guard(state, trigger, run_guard).await
}

async fn run_moltbook_cycle_with_guard(
    state: &AppState,
    trigger: &str,
    _run_guard: MoltbookRunGuard,
) -> serde_json::Value {
    let (storage, config_dir) = {
        let agent = state.agent.read().await;
        (agent.storage.clone(), agent.config_dir.clone())
    };

    let settings = load_moltbook_settings(&storage).await;
    let now = chrono::Utc::now();
    let run_id = uuid::Uuid::new_v4().to_string();

    if !settings.enabled {
        let _ = storage.delete(MOLTBOOK_NEXT_RUN_KEY).await;
        let _ = storage.delete(MOLTBOOK_DEFER_COUNT_KEY).await;
        let _ = storage.set(MOLTBOOK_LAST_STATUS_KEY, b"disabled").await;
        append_moltbook_activity(
            &storage,
            &run_id,
            "info",
            "skipped_disabled",
            serde_json::json!({
                "trigger": trigger,
                "reason": "Moltbook is disabled on the Moltbook page.",
            }),
        )
        .await;
        return serde_json::json!({
            "status": "disabled",
            "run_id": run_id
        });
    }
    if settings.mode == "off" {
        let _ = storage.delete(MOLTBOOK_NEXT_RUN_KEY).await;
        let _ = storage.delete(MOLTBOOK_DEFER_COUNT_KEY).await;
        let _ = storage.set(MOLTBOOK_LAST_STATUS_KEY, b"off_mode").await;
        append_moltbook_activity(
            &storage,
            &run_id,
            "info",
            "skipped_off_mode",
            serde_json::json!({
                "trigger": trigger,
                "reason": "Moltbook mode is set to off.",
            }),
        )
        .await;
        return serde_json::json!({
            "status": "off_mode",
            "run_id": run_id
        });
    }

    if trigger != "manual" {
        let next_run = storage
            .get(MOLTBOOK_NEXT_RUN_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|v| String::from_utf8(v).ok())
            .and_then(|s| parse_utc_rfc3339(&s));
        if let Some(next) = next_run {
            if now < next {
                return serde_json::json!({
                    "status": "not_due",
                    "run_id": run_id,
                    "next_run_at": next.to_rfc3339(),
                });
            }
        }
    }

    let busy_reasons = if trigger != "manual" && settings.defer_when_busy {
        server_load_reasons(state).await
    } else {
        Vec::new()
    };
    if trigger != "manual" && settings.defer_when_busy && !busy_reasons.is_empty() {
        let defers = storage
            .get(MOLTBOOK_DEFER_COUNT_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|v| String::from_utf8(v).ok())
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);

        if defers >= 3 {
            let retry_at = std::cmp::max(
                now + chrono::Duration::minutes(30),
                moltbook_next_run_at(&settings.sync_frequency, now),
            );
            let _ = storage
                .set(MOLTBOOK_NEXT_RUN_KEY, retry_at.to_rfc3339().as_bytes())
                .await;
            let _ = storage.delete(MOLTBOOK_DEFER_COUNT_KEY).await;
            let _ = storage.set(MOLTBOOK_LAST_STATUS_KEY, b"skipped_busy").await;
            append_moltbook_activity(
                &storage,
                &run_id,
                "warning",
                "skipped_busy_max_defers",
                serde_json::json!({
                    "trigger": trigger,
                    "max_defers": 3,
                    "busy_reasons": busy_reasons,
                    "retry_at": retry_at.to_rfc3339()
                }),
            )
            .await;
            return serde_json::json!({
                "status": "skipped_busy",
                "run_id": run_id,
                "busy_reasons": busy_reasons,
                "retry_at": retry_at.to_rfc3339(),
            });
        }

        let new_defers = defers + 1;
        let deferred_to = now + chrono::Duration::minutes(10);
        let _ = storage
            .set(MOLTBOOK_NEXT_RUN_KEY, deferred_to.to_rfc3339().as_bytes())
            .await;
        let _ = storage
            .set(MOLTBOOK_DEFER_COUNT_KEY, new_defers.to_string().as_bytes())
            .await;
        let _ = storage
            .set(MOLTBOOK_LAST_STATUS_KEY, b"deferred_busy")
            .await;
        append_moltbook_activity(
            &storage,
            &run_id,
            "warning",
            "deferred_busy",
            serde_json::json!({
                "trigger": trigger,
                "deferred_to": deferred_to.to_rfc3339(),
                "attempt": new_defers,
                "max_defers": 3,
                "busy_reasons": busy_reasons
            }),
        )
        .await;
        return serde_json::json!({
            "status": "deferred_busy",
            "run_id": run_id,
            "deferred_to": deferred_to.to_rfc3339(),
            "attempt": new_defers,
            "max_defers": 3,
            "busy_reasons": busy_reasons
        });
    }

    // Reset busy deferral count on any non-busy run attempt (manual or scheduled).
    let _ = storage.delete(MOLTBOOK_DEFER_COUNT_KEY).await;

    append_moltbook_activity(
        &storage,
        &run_id,
        "info",
        "run_started",
        serde_json::json!({ "trigger": trigger }),
    )
    .await;

    let connector =
        crate::integrations::moltbook::MoltbookConnector::new_with_config_dir(config_dir);
    let mut read_count = 0usize;
    let mut posted = false;
    let mut post_count = 0usize;
    let mut comment_count = 0usize;
    let mut upvote_count = 0usize;
    let mut collaboration_count = 0usize;
    let mut posted_id: Option<String> = None;
    let mut posted_api_url: Option<String> = None;
    let mut posted_url: Option<String> = None;
    let mut decision_summary = String::new();
    let mut engaged_post_ids: Vec<String> = Vec::new();
    let mut engagement_failures: Vec<String> = Vec::new();
    let mut feed_posts_raw: Vec<serde_json::Value> = Vec::new();

    let status_api_url = "https://www.moltbook.com/api/v1/agents/status";
    let feed_api_url = "https://www.moltbook.com/api/v1/feed?sort=new&limit=10";
    let create_post_api_url = "https://www.moltbook.com/api/v1/posts";

    let status_result =
        crate::integrations::Integration::execute(&connector, "status", &serde_json::json!({}))
            .await;
    let (connected, status_state, status_error) = match status_result {
        Ok(v) => (
            v.get("connected")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
            v.get("status")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown")
                .to_string(),
            v.get("error")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
        ),
        Err(e) => (false, "error".to_string(), Some(e.to_string())),
    };
    append_moltbook_activity(
        &storage,
        &run_id,
        "info",
        "status_checked",
        serde_json::json!({
            "action_kind": "read",
            "api_url": status_api_url,
            "connected": connected,
            "status": status_state.clone(),
            "error": status_error.clone()
        }),
    )
    .await;
    if !connected {
        let reason = if status_state == "not_configured" {
            "Moltbook API key is not configured. Save the key on the Moltbook page, then run again."
                .to_string()
        } else if status_state == "error" {
            format!(
                "Moltbook authentication failed{}",
                status_error
                    .as_ref()
                    .map(|e| format!(": {}", e))
                    .unwrap_or_else(String::new)
            )
        } else {
            "Could not connect to Moltbook.".to_string()
        };
        let next = moltbook_next_run_at(&settings.sync_frequency, now);
        let _ = storage
            .set(MOLTBOOK_NEXT_RUN_KEY, next.to_rfc3339().as_bytes())
            .await;
        let _ = storage
            .set(MOLTBOOK_LAST_STATUS_KEY, status_state.as_bytes())
            .await;
        append_moltbook_activity(
            &storage,
            &run_id,
            "warning",
            "not_connected",
            serde_json::json!({
                "action_kind": "read",
                "api_url": status_api_url,
                "status": status_state.clone(),
                "error": status_error.clone(),
                "reason": reason,
                "recovery_hint": "Open the Moltbook page, configure the API key, save, and ensure the agent is claimed."
            }),
        )
        .await;
        return serde_json::json!({
            "status": "not_connected",
            "run_id": run_id,
            "status_detail": status_state.clone(),
            "error": status_error.clone(),
            "reason": reason
        });
    }

    match crate::integrations::Integration::execute(
        &connector,
        "feed",
        &serde_json::json!({"sort":"new","limit":10}),
    )
    .await
    {
        Ok(feed) => {
            let posts = feed
                .get("posts")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            feed_posts_raw = posts.clone();
            read_count = posts.len();
            let read_posts: Vec<serde_json::Value> = posts
                .iter()
                .take(10)
                .map(|p| {
                    let submolt = p
                        .get("submolt")
                        .and_then(|s| {
                            s.get("name")
                                .and_then(|v| v.as_str())
                                .or_else(|| s.as_str())
                        })
                        .map(|s| s.to_string());
                    let post_id = p.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
                    let post_api_url = post_id.as_ref().map(|id| moltbook_post_api_url(id));
                    serde_json::json!({
                        "id": post_id,
                        "title": p.get("title").and_then(|v| v.as_str()),
                        "submolt": submolt,
                        "url": p.get("url").and_then(|v| v.as_str()),
                        "post_api_url": post_api_url,
                        "author": p.get("author")
                            .and_then(|a| a.get("name"))
                            .and_then(|v| v.as_str())
                    })
                })
                .collect();
            let samples: Vec<String> = posts
                .iter()
                .take(3)
                .filter_map(|p| {
                    p.get("title")
                        .and_then(|v| v.as_str())
                        .map(|s| s.chars().take(120).collect::<String>())
                })
                .collect();
            append_moltbook_activity(
                &storage,
                &run_id,
                "info",
                "feed_read",
                serde_json::json!({
                    "action_kind": "read",
                    "api_url": feed_api_url,
                    "count": read_count,
                    "sample_titles": samples,
                    "read_posts": read_posts
                }),
            )
            .await;
        }
        Err(e) => {
            append_moltbook_activity(
                &storage,
                &run_id,
                "error",
                "feed_read_failed",
                serde_json::json!({
                    "action_kind": "read",
                    "api_url": feed_api_url,
                    "error": e.to_string()
                }),
            )
            .await;
        }
    }

    let writes_allowed = moltbook_write_mode_enabled(&settings, trigger);
    let last_post = storage
        .get(MOLTBOOK_LAST_POST_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| String::from_utf8(v).ok())
        .and_then(|s| parse_utc_rfc3339(&s));
    let can_create_post = last_post
        .map(|lp| (now - lp).num_hours() >= 24)
        .unwrap_or(true);

    if !settings.write_enabled {
        decision_summary = "Autonomous engagement is disabled.".to_string();
        append_moltbook_activity(
            &storage,
            &run_id,
            "info",
            "engagement_skipped_disabled",
            serde_json::json!({
                "reason": decision_summary
            }),
        )
        .await;
    } else if settings.mode == "read_only" {
        decision_summary = format!(
            "Read-only mode: {} reviewed the feed without public engagement.",
            crate::branding::PRODUCT_NAME
        );
        append_moltbook_activity(
            &storage,
            &run_id,
            "info",
            "engagement_skipped_mode",
            serde_json::json!({
                "mode": settings.mode,
                "reason": decision_summary
            }),
        )
        .await;
    } else if settings.mode == "assist" && trigger != "manual" {
        decision_summary = "Assist mode only engages on manual runs.".to_string();
        append_moltbook_activity(
            &storage,
            &run_id,
            "info",
            "engagement_skipped_mode",
            serde_json::json!({
                "mode": settings.mode,
                "trigger": trigger,
                "reason": decision_summary
            }),
        )
        .await;
    } else if writes_allowed {
        let (llm, agent_name) = {
            let agent = state.agent.read().await;
            (
                agent
                    .llm_for_explicit_slot_or_primary(settings.model_slot_id.as_deref())
                    .clone(),
                agent.config.name.clone(),
            )
        };
        let recent_activity = build_moltbook_recent_activity_context(&storage).await;
        let submolt_context = build_moltbook_submolt_context(&feed_posts_raw);
        let agent_name_lower = agent_name.trim().to_lowercase();
        let mut post_context: HashMap<String, (String, String, String, String)> = HashMap::new();
        let planner_posts: Vec<serde_json::Value> = feed_posts_raw
            .iter()
            .take(8)
            .filter_map(|post| {
                let post_id = post.get("id").and_then(|v| v.as_str())?.trim().to_string();
                let title = post
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let submolt = post
                    .get("submolt")
                    .and_then(|value| {
                        value
                            .get("name")
                            .and_then(|name| name.as_str())
                            .or_else(|| value.as_str())
                    })
                    .unwrap_or("general")
                    .trim()
                    .to_string();
                let author = post
                    .get("author")
                    .and_then(|value| {
                        value
                            .get("name")
                            .and_then(|name| name.as_str())
                            .or_else(|| value.as_str())
                    })
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let post_url = post
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let preview = moltbook_post_preview(post);
                post_context.insert(
                    post_id.clone(),
                    (
                        submolt.clone(),
                        author.clone(),
                        post_url.clone(),
                        title.clone(),
                    ),
                );
                Some(serde_json::json!({
                    "id": post_id,
                    "title": title,
                    "author": author,
                    "submolt": submolt,
                    "url": post_url,
                    "preview": preview,
                    "comment_count": post.get("comment_count").cloned().unwrap_or(serde_json::Value::Null),
                    "score": post.get("score").cloned().unwrap_or(serde_json::Value::Null),
                    "upvotes": post.get("upvotes").cloned().unwrap_or(serde_json::Value::Null)
                }))
            })
            .collect();

        let has_planner_posts = !planner_posts.is_empty();
        let planner_payload = serde_json::json!({
            "trigger": trigger,
            "mode": settings.mode.clone(),
            "agent_name": agent_name.clone(),
            "can_create_post": can_create_post,
            "budgets": {
                "max_comments": 2,
                "max_upvotes": 3,
                "max_posts": 1
            },
            "recent_activity": recent_activity,
            "submolt_context": submolt_context,
            "feed": planner_posts
        });
        let planner_prompt = crate::branding::render_template(
            r#"You are __PRODUCT_NAME__'s Moltbook participation planner.
Return strict JSON only with this shape:
{"summary":"short string","actions":[{"action":"comment|upvote_post|create_post","post_id":"optional","parent_id":"optional","submolt":"optional","title":"optional","content":"optional","reason":"optional"}]}

Rules:
- Behave like a thoughtful operator, not a spam bot.
- Pick the best mix of original contribution and engagement from the current feed plus recent activity context.
- Prefer 0-4 total actions.
- Use "comment" for substantive replies, "upvote_post" for strong posts worth endorsing, and "create_post" when the feed supports a durable thread, synthesis, lesson, question, or perspective worth starting.
- Do not default to comments/upvotes just because they are cheaper than posting.
- Use recent_activity to avoid both silence and spam: if recent public activity is mostly comments/upvotes and light on original posts, a new post can be the right move; if there was a recent original post, prefer engagement unless a distinct thread is clearly justified.
- When creating a post, choose a fitting submolt from submolt_context when one is clearly implied by the feed. Use general only when no stronger fit is visible.
- Do not create filler check-ins or generic status posts.
- Never produce more than 2 comments, 3 upvotes, or 1 new post.
- Do not comment on or upvote the agent's own post.
- Do not invent post IDs or submolt names.
- If nothing should be done, return an empty actions array and explain why in summary.
- There is no downvote action available in this environment."#,
        );
        let llm_response = if has_planner_posts || (settings.mode == "autopost" && can_create_post)
        {
            llm.chat(&planner_prompt, &planner_payload.to_string(), &[], &[])
                .await
                .ok()
        } else {
            None
        };
        if let Some(ref response) = llm_response {
            let agent = state.agent.read().await;
            agent
                .record_llm_usage("moltbook", "engagement_planner", response)
                .await;
        }

        let mut plan = llm_response
            .as_ref()
            .and_then(|response| extract_json(&response.content))
            .and_then(|value| serde_json::from_value::<MoltbookEngagementPlan>(value).ok())
            .unwrap_or_default();

        if plan.summary.trim().is_empty() {
            plan.summary = if feed_posts_raw.is_empty() {
                "No feed posts were available to engage with.".to_string()
            } else {
                "Reviewed the feed and selected the safest next actions.".to_string()
            };
        }

        let already_plans_post = plan.actions.iter().any(|action| {
            matches!(
                action.action.trim().to_lowercase().as_str(),
                "create_post" | "post"
            )
        });

        if !already_plans_post
            && settings.mode == "autopost"
            && can_create_post
            && has_planner_posts
        {
            let post_probe_prompt = crate::branding::render_template(
                r#"You are __PRODUCT_NAME__'s Moltbook original-post planner.
Return strict JSON only with this shape:
{"summary":"short string","actions":[{"action":"create_post","submolt":"optional","title":"required if posting","content":"required if posting","reason":"optional"}]}

Rules:
- Decide whether this run merits one original Moltbook post.
- Only create a post if the current feed and recent activity support a substantive standalone thread.
- Good posts synthesize a pattern, tension, lesson, forecast, or open question from the feed.
- Use recent_activity to avoid spam: if there was a recent original post, skip unless this thread is clearly distinct.
- Use submolt_context to choose the best destination when there is a clear fit.
- Do not produce filler updates, check-ins, or posts that merely restate the feed.
- If no original post is justified, return an empty actions array."#,
            );
            let post_probe_response = llm
                .chat(&post_probe_prompt, &planner_payload.to_string(), &[], &[])
                .await
                .ok();
            if let Some(ref response) = post_probe_response {
                let agent = state.agent.read().await;
                agent
                    .record_llm_usage("moltbook", "original_post_planner", response)
                    .await;
            }
            let post_probe_plan = post_probe_response
                .as_ref()
                .and_then(|response| extract_json(&response.content))
                .and_then(|value| serde_json::from_value::<MoltbookEngagementPlan>(value).ok())
                .unwrap_or_default();
            let post_actions = post_probe_plan
                .actions
                .into_iter()
                .filter(|action| {
                    matches!(
                        action.action.trim().to_lowercase().as_str(),
                        "create_post" | "post"
                    )
                })
                .collect::<Vec<_>>();
            if !post_actions.is_empty() {
                let probe_summary = post_probe_plan.summary.trim().to_string();
                append_moltbook_activity(
                    &storage,
                    &run_id,
                    "info",
                    "original_post_candidate_selected",
                    serde_json::json!({
                        "summary": probe_summary,
                        "planned_actions": post_actions.iter().map(|action| serde_json::json!({
                            "action": action.action.clone(),
                            "submolt": action.submolt.clone(),
                            "title_preview": action.title.as_deref().map(|value| moltbook_text_preview(value, 120)),
                            "content_preview": action.content.as_deref().map(|value| moltbook_text_preview(value, 180)),
                            "reason": action.reason.clone()
                        })).collect::<Vec<_>>()
                    }),
                )
                .await;
                if !probe_summary.is_empty() {
                    plan.summary = if plan.summary.trim().is_empty() {
                        probe_summary
                    } else {
                        format!("{} {}", plan.summary.trim(), probe_summary)
                    };
                }
                plan.actions.extend(post_actions);
            } else {
                append_moltbook_activity(
                    &storage,
                    &run_id,
                    "info",
                    "original_post_candidate_skipped",
                    serde_json::json!({
                        "summary": post_probe_plan.summary
                    }),
                )
                .await;
            }
        }

        if !plan.actions.is_empty() || !plan.summary.trim().is_empty() {
            append_moltbook_activity(
                &storage,
                &run_id,
                "info",
                "engagement_plan_created",
                serde_json::json!({
                    "summary": plan.summary,
                    "planned_actions": plan.actions.iter().map(|action| serde_json::json!({
                        "action": action.action.clone(),
                        "post_id": action.post_id.clone(),
                        "submolt": action.submolt.clone(),
                        "title_preview": action.title.as_deref().map(|value| moltbook_text_preview(value, 120)),
                        "content_preview": action.content.as_deref().map(|value| moltbook_text_preview(value, 180)),
                        "reason": action.reason.clone()
                    })).collect::<Vec<_>>()
                }),
            )
            .await;
        }

        decision_summary = plan.summary.trim().to_string();

        let mut seen_actions = HashSet::new();
        let mut remaining_comments = 2usize;
        let mut remaining_upvotes = 3usize;
        let mut remaining_posts = 1usize;
        let mut post_skipped_for_cooldown = false;

        for action in plan.actions {
            let action_kind = action.action.trim().to_lowercase();
            match action_kind.as_str() {
                "comment" => {
                    if remaining_comments == 0 {
                        continue;
                    }
                    let Some(post_id) = action
                        .post_id
                        .clone()
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                    else {
                        continue;
                    };
                    let Some(content) = action
                        .content
                        .clone()
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                    else {
                        continue;
                    };
                    if !seen_actions.insert(format!("comment:{}", post_id)) {
                        continue;
                    }
                    if let Some((_, author, _, _)) = post_context.get(&post_id) {
                        if !author.is_empty()
                            && !agent_name_lower.is_empty()
                            && author.to_lowercase().contains(&agent_name_lower)
                        {
                            continue;
                        }
                    }
                    let content_privacy = sanitize_moltbook_public_text(&content);
                    if matches!(
                        content_privacy.decision,
                        crate::security::OutboundPrivacyDecision::Block
                    ) {
                        append_moltbook_activity(
                            &storage,
                            &run_id,
                            "warning",
                            "comment_blocked_privacy",
                            serde_json::json!({
                                "action_kind": "write",
                                "post_id": post_id,
                                "reason": action.reason.clone(),
                                "privacy": moltbook_privacy_details(&content_privacy)
                            }),
                        )
                        .await;
                        engagement_failures.push(format!(
                            "comment {}: blocked by outbound privacy gate",
                            post_id
                        ));
                        continue;
                    }
                    remaining_comments -= 1;
                    let post_api_url = moltbook_post_api_url(&post_id);
                    let comment_api_url = format!("{}/comments", post_api_url);
                    let post_meta = post_context.get(&post_id).cloned();
                    let post_url = post_meta.as_ref().map(|(_, _, url, _)| url.clone());
                    let sanitized_comment_content = content_privacy.sanitized_text.clone();
                    match crate::integrations::Integration::execute(
                        &connector,
                        "comment",
                        &serde_json::json!({
                            "post_id": post_id,
                            "parent_id": action.parent_id.clone(),
                            "content": sanitized_comment_content
                        }),
                    )
                    .await
                    {
                        Ok(_) => {
                            comment_count += 1;
                            collaboration_count += 1;
                            engaged_post_ids.push(post_id.clone());
                            let _ = storage
                                .set(MOLTBOOK_LAST_COMMENT_KEY, now.to_rfc3339().as_bytes())
                                .await;
                            append_moltbook_activity(
                                &storage,
                                &run_id,
                                "info",
                                "comment_created",
                                serde_json::json!({
                                    "action_kind": "write",
                                    "api_url": comment_api_url,
                                    "post_api_url": post_api_url,
                                    "post_id": post_id,
                                    "post_url": post_url,
                                    "post_title": post_meta.as_ref().map(|(_, _, _, title)| title.clone()),
                                    "submolt": post_meta.as_ref().map(|(submolt, _, _, _)| submolt.clone()),
                                    "author": post_meta.as_ref().map(|(_, author, _, _)| author.clone()),
                                    "content_preview": moltbook_text_preview(&content_privacy.sanitized_text, 220),
                                    "reason": action.reason.clone(),
                                    "privacy": moltbook_privacy_details(&content_privacy)
                                }),
                            )
                            .await;
                        }
                        Err(e) => {
                            let error = e.to_string();
                            engagement_failures.push(format!("comment {}: {}", post_id, error));
                            append_moltbook_activity(
                                &storage,
                                &run_id,
                                "warning",
                                "comment_failed",
                                serde_json::json!({
                                    "action_kind": "write",
                                    "api_url": comment_api_url,
                                    "post_api_url": post_api_url,
                                    "post_id": post_id,
                                    "post_url": post_url,
                                    "post_title": post_meta.as_ref().map(|(_, _, _, title)| title.clone()),
                                    "submolt": post_meta.as_ref().map(|(submolt, _, _, _)| submolt.clone()),
                                    "author": post_meta.as_ref().map(|(_, author, _, _)| author.clone()),
                                    "content_preview": moltbook_text_preview(&content_privacy.sanitized_text, 220),
                                    "reason": action.reason.clone(),
                                    "privacy": moltbook_privacy_details(&content_privacy),
                                    "error": error
                                }),
                            )
                            .await;
                        }
                    }
                }
                "upvote_post" | "upvote" => {
                    if remaining_upvotes == 0 {
                        continue;
                    }
                    let Some(post_id) = action
                        .post_id
                        .clone()
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                    else {
                        continue;
                    };
                    if !seen_actions.insert(format!("upvote:{}", post_id)) {
                        continue;
                    }
                    if let Some((_, author, _, _)) = post_context.get(&post_id) {
                        if !author.is_empty()
                            && !agent_name_lower.is_empty()
                            && author.to_lowercase().contains(&agent_name_lower)
                        {
                            continue;
                        }
                    }
                    remaining_upvotes -= 1;
                    let post_api_url = moltbook_post_api_url(&post_id);
                    let upvote_api_url = format!("{}/upvote", post_api_url);
                    let post_meta = post_context.get(&post_id).cloned();
                    let post_url = post_meta.as_ref().map(|(_, _, url, _)| url.clone());
                    match crate::integrations::Integration::execute(
                        &connector,
                        "upvote_post",
                        &serde_json::json!({ "post_id": post_id }),
                    )
                    .await
                    {
                        Ok(_) => {
                            upvote_count += 1;
                            collaboration_count += 1;
                            engaged_post_ids.push(post_id.clone());
                            let _ = storage
                                .set(MOLTBOOK_LAST_UPVOTE_KEY, now.to_rfc3339().as_bytes())
                                .await;
                            append_moltbook_activity(
                                &storage,
                                &run_id,
                                "info",
                                "post_upvoted",
                                serde_json::json!({
                                    "action_kind": "write",
                                    "api_url": upvote_api_url,
                                    "post_api_url": post_api_url,
                                    "post_id": post_id,
                                    "post_url": post_url,
                                    "post_title": post_meta.as_ref().map(|(_, _, _, title)| title.clone()),
                                    "submolt": post_meta.as_ref().map(|(submolt, _, _, _)| submolt.clone()),
                                    "author": post_meta.as_ref().map(|(_, author, _, _)| author.clone()),
                                    "reason": action.reason.clone()
                                }),
                            )
                            .await;
                        }
                        Err(e) => {
                            let error = e.to_string();
                            engagement_failures.push(format!("upvote {}: {}", post_id, error));
                            append_moltbook_activity(
                                &storage,
                                &run_id,
                                "warning",
                                "upvote_failed",
                                serde_json::json!({
                                    "action_kind": "write",
                                    "api_url": upvote_api_url,
                                    "post_api_url": post_api_url,
                                    "post_id": post_id,
                                    "post_url": post_url,
                                    "post_title": post_meta.as_ref().map(|(_, _, _, title)| title.clone()),
                                    "submolt": post_meta.as_ref().map(|(submolt, _, _, _)| submolt.clone()),
                                    "author": post_meta.as_ref().map(|(_, author, _, _)| author.clone()),
                                    "reason": action.reason.clone(),
                                    "error": error
                                }),
                            )
                            .await;
                        }
                    }
                }
                "create_post" | "post" => {
                    if remaining_posts == 0 {
                        continue;
                    }
                    if !can_create_post {
                        post_skipped_for_cooldown = true;
                        continue;
                    }
                    let Some(title) = action
                        .title
                        .clone()
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                    else {
                        continue;
                    };
                    let Some(content) = action
                        .content
                        .clone()
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                    else {
                        continue;
                    };
                    let submolt = action
                        .submolt
                        .clone()
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| "general".to_string());
                    if !seen_actions.insert(format!("post:{}:{}", submolt, title)) {
                        continue;
                    }
                    let title_privacy = sanitize_moltbook_public_text(&title);
                    let content_privacy = sanitize_moltbook_public_text(&content);
                    if matches!(
                        title_privacy.decision,
                        crate::security::OutboundPrivacyDecision::Block
                    ) || matches!(
                        content_privacy.decision,
                        crate::security::OutboundPrivacyDecision::Block
                    ) {
                        append_moltbook_activity(
                            &storage,
                            &run_id,
                            "warning",
                            "post_blocked_privacy",
                            serde_json::json!({
                                "action_kind": "write",
                                "api_url": create_post_api_url,
                                "request": {
                                    "submolt": submolt,
                                },
                                "reason": action.reason.clone(),
                                "privacy": {
                                    "title": moltbook_privacy_details(&title_privacy),
                                    "content": moltbook_privacy_details(&content_privacy)
                                }
                            }),
                        )
                        .await;
                        engagement_failures
                            .push("post: blocked by outbound privacy gate".to_string());
                        continue;
                    }
                    remaining_posts -= 1;
                    let sanitized_post_title = title_privacy.sanitized_text.clone();
                    let sanitized_post_content = content_privacy.sanitized_text.clone();
                    let post_preview = moltbook_text_preview(&sanitized_post_content, 280);
                    match crate::integrations::Integration::execute(
                        &connector,
                        "create_post",
                        &serde_json::json!({
                            "submolt": submolt,
                            "title": sanitized_post_title,
                            "content": sanitized_post_content
                        }),
                    )
                    .await
                    {
                        Ok(resp) => {
                            posted = true;
                            post_count += 1;
                            posted_id = resp
                                .get("post")
                                .and_then(|p| p.get("id"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            posted_api_url = posted_id
                                .as_ref()
                                .map(|id| format!("https://www.moltbook.com/api/v1/posts/{}", id));
                            posted_url = resp
                                .get("post")
                                .and_then(|p| p.get("url"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            let _ = storage
                                .set(MOLTBOOK_LAST_POST_KEY, now.to_rfc3339().as_bytes())
                                .await;
                            append_moltbook_activity(
                                &storage,
                                &run_id,
                                "info",
                                "post_created",
                                serde_json::json!({
                                    "action_kind": "write",
                                    "api_url": create_post_api_url,
                                    "post_id": posted_id,
                                    "request": {
                                        "submolt": submolt,
                                        "title": title_privacy.sanitized_text.clone(),
                                        "content_preview": post_preview
                                    },
                                    "reason": action.reason.clone(),
                                    "privacy": {
                                        "title": moltbook_privacy_details(&title_privacy),
                                        "content": moltbook_privacy_details(&content_privacy)
                                    },
                                    "post_api_url": posted_api_url,
                                    "post_url": posted_url
                                }),
                            )
                            .await;
                        }
                        Err(e) => {
                            let error = e.to_string();
                            engagement_failures.push(format!("post: {}", error));
                            append_moltbook_activity(
                                &storage,
                                &run_id,
                                "warning",
                                "post_failed",
                                serde_json::json!({
                                    "action_kind": "write",
                                    "api_url": create_post_api_url,
                                    "reason": action.reason.clone(),
                                    "privacy": {
                                        "title": moltbook_privacy_details(&title_privacy),
                                        "content": moltbook_privacy_details(&content_privacy)
                                    },
                                    "error": error
                                }),
                            )
                            .await;
                        }
                    }
                }
                _ => {}
            }
        }

        if post_skipped_for_cooldown {
            append_moltbook_activity(
                &storage,
                &run_id,
                "info",
                "post_skipped_cooldown",
                serde_json::json!({
                    "reason": "Skipped creating a new post because the 24-hour posting cooldown is still active."
                }),
            )
            .await;
        }

        if comment_count + upvote_count + post_count == 0 {
            if feed_posts_raw.is_empty() {
                decision_summary = "No feed posts were available to engage with.".to_string();
                append_moltbook_activity(
                    &storage,
                    &run_id,
                    "info",
                    "engagement_skipped_empty_feed",
                    serde_json::json!({
                        "reason": decision_summary
                    }),
                )
                .await;
            } else if engagement_failures.is_empty() {
                if decision_summary.trim().is_empty() {
                    decision_summary =
                        "Nothing in the current feed required a public action yet.".to_string();
                }
                append_moltbook_activity(
                    &storage,
                    &run_id,
                    "info",
                    "engagement_skipped_not_needed",
                    serde_json::json!({
                        "reason": decision_summary
                    }),
                )
                .await;
            }
        }
    }

    if comment_count + upvote_count + post_count > 0 {
        let _ = storage
            .set(MOLTBOOK_LAST_ENGAGEMENT_KEY, now.to_rfc3339().as_bytes())
            .await;
    }

    engaged_post_ids.sort();
    engaged_post_ids.dedup();
    if decision_summary.trim().is_empty() {
        decision_summary = if comment_count + upvote_count + post_count > 0 {
            format!(
                "Completed {} engagement action(s) after reviewing the feed.",
                comment_count + upvote_count + post_count
            )
        } else if !engagement_failures.is_empty() {
            format!(
                "Attempted engagement, but {} action(s) failed.",
                engagement_failures.len()
            )
        } else {
            "Reviewed the feed without public changes.".to_string()
        };
    }

    let mut run_links = Vec::new();
    let mut run_link_seen = HashSet::new();
    let mut run_api_links = Vec::new();
    let mut run_api_link_seen = HashSet::new();

    for post in feed_posts_raw.iter().take(10) {
        let title = post
            .get("title")
            .and_then(|value| value.as_str())
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .unwrap_or("Feed post")
            .to_string();
        let post_url = post
            .get("url")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        let post_id = post
            .get("id")
            .and_then(|value| value.as_str())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        push_moltbook_run_link(
            &mut run_links,
            &mut run_link_seen,
            format!("Post: {}", title),
            post_url,
        );
        push_moltbook_run_link(
            &mut run_api_links,
            &mut run_api_link_seen,
            format!("Post API: {}", title),
            post_id.as_ref().map(|id| moltbook_post_api_url(id)),
        );
    }

    for post_id in engaged_post_ids.iter() {
        push_moltbook_run_link(
            &mut run_links,
            &mut run_link_seen,
            format!("Engaged post {}", post_id),
            Some(moltbook_human_post_url(post_id)),
        );
        push_moltbook_run_link(
            &mut run_api_links,
            &mut run_api_link_seen,
            format!("Engaged post API {}", post_id),
            Some(moltbook_post_api_url(post_id)),
        );
    }

    push_moltbook_run_link(
        &mut run_links,
        &mut run_link_seen,
        "Created post",
        posted_url.clone(),
    );
    push_moltbook_run_link(
        &mut run_api_links,
        &mut run_api_link_seen,
        "Created post API",
        posted_api_url.clone(),
    );

    let external_memory = match distill_moltbook_memory_insights(
        state,
        MoltbookMemoryDistillationInput {
            trigger,
            settings: &settings,
            feed_posts_raw: &feed_posts_raw,
            decision_summary: &decision_summary,
            engaged_post_ids: &engaged_post_ids,
            comment_count,
            upvote_count,
            post_count,
            engagement_failures: &engagement_failures,
            posted_url: posted_url.as_deref(),
        },
    )
    .await
    {
        Ok(Some(insights)) => {
            let (sanitized_insights, privacy) = sanitize_moltbook_memory_insights(&insights);
            match sanitized_insights {
                Some(sanitized) => {
                    match persist_moltbook_external_knowledge(&storage, &sanitized).await {
                        Ok(item) => {
                            append_moltbook_activity(
                                &storage,
                                &run_id,
                                "info",
                                "external_memory_saved",
                                serde_json::json!({
                                    "knowledge_id": item.id.clone(),
                                    "title": item.title.clone(),
                                    "source": item.source.clone(),
                                    "tags": item.tags.clone(),
                                    "summary": sanitized.summary.clone(),
                                    "insights": sanitized.insights.clone(),
                                    "privacy": privacy.clone()
                                }),
                            )
                            .await;
                            serde_json::json!({
                                "status": "saved",
                                "store": "knowledge",
                                "knowledge_id": item.id,
                                "title": item.title,
                                "source": item.source,
                                "tags": item.tags,
                                "summary": sanitized.summary,
                                "insights": sanitized.insights,
                                "privacy": privacy
                            })
                        }
                        Err(error) => {
                            append_moltbook_activity(
                                &storage,
                                &run_id,
                                "warning",
                                "external_memory_save_failed",
                                serde_json::json!({
                                    "summary": sanitized.summary.clone(),
                                    "insights": sanitized.insights.clone(),
                                    "privacy": privacy.clone(),
                                    "error": error.clone()
                                }),
                            )
                            .await;
                            serde_json::json!({
                                "status": "save_failed",
                                "store": "knowledge",
                                "summary": sanitized.summary,
                                "insights": sanitized.insights,
                                "privacy": privacy,
                                "error": error
                            })
                        }
                    }
                }
                None => {
                    append_moltbook_activity(
                        &storage,
                        &run_id,
                        "warning",
                        "external_memory_blocked_privacy",
                        serde_json::json!({
                            "reason": "Distilled community learnings were blocked by the outbound privacy gate.",
                            "privacy": privacy.clone()
                        }),
                    )
                    .await;
                    serde_json::json!({
                        "status": "blocked",
                        "store": "knowledge",
                        "privacy": privacy
                    })
                }
            }
        }
        Ok(None) => {
            append_moltbook_activity(
                &storage,
                &run_id,
                "info",
                "external_memory_skipped",
                serde_json::json!({
                    "reason": "No durable Moltbook learnings were identified for source-scoped knowledge."
                }),
            )
            .await;
            serde_json::json!({ "status": "skipped" })
        }
        Err(error) => {
            append_moltbook_activity(
                &storage,
                &run_id,
                "warning",
                "external_memory_distill_failed",
                serde_json::json!({
                    "error": error
                }),
            )
            .await;
            serde_json::json!({
                "status": "distill_failed",
                "error": error
            })
        }
    };

    let next = moltbook_next_run_at(&settings.sync_frequency, now);
    let _ = storage
        .set(MOLTBOOK_NEXT_RUN_KEY, next.to_rfc3339().as_bytes())
        .await;
    let _ = storage
        .set(MOLTBOOK_LAST_RUN_KEY, now.to_rfc3339().as_bytes())
        .await;
    let _ = storage.set(MOLTBOOK_LAST_STATUS_KEY, b"ok").await;

    let result = serde_json::json!({
        "status": "ok",
        "run_id": run_id,
        "trigger": trigger,
        "read_count": read_count,
        "comment_count": comment_count,
        "upvote_count": upvote_count,
        "post_count": post_count,
        "collaboration_count": collaboration_count,
        "engagement_count": comment_count + upvote_count + post_count,
        "engaged_post_ids": engaged_post_ids,
        "engagement_failures": engagement_failures,
        "decision_summary": decision_summary,
        "posted": posted,
        "posted_id": posted_id,
        "post_api_url": posted_api_url,
        "post_url": posted_url,
        "links": run_links,
        "api_links": run_api_links,
        "external_memory": external_memory,
        "next_run_at": next.to_rfc3339(),
    });
    let _ = storage
        .set(MOLTBOOK_LAST_RUN_STATS_KEY, result.to_string().as_bytes())
        .await;
    append_moltbook_activity(&storage, &run_id, "info", "run_completed", result.clone()).await;
    result
}

/// Get Moltbook scheduler status/settings.
pub(super) async fn get_moltbook_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let (storage, config_dir, data_dir, has_connector) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
            agent.integrations.get("moltbook").is_some(),
        )
    };
    let settings = load_moltbook_settings(&storage).await;
    let has_api_key = has_moltbook_api_key(&config_dir, Some(&data_dir));
    let last_run_at = storage
        .get(MOLTBOOK_LAST_RUN_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| String::from_utf8(v).ok());
    let next_run_at = storage
        .get(MOLTBOOK_NEXT_RUN_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| String::from_utf8(v).ok());
    let last_status = storage
        .get(MOLTBOOK_LAST_STATUS_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| String::from_utf8(v).ok());
    let last_post_at = storage
        .get(MOLTBOOK_LAST_POST_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| String::from_utf8(v).ok());
    let last_comment_at = storage
        .get(MOLTBOOK_LAST_COMMENT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| String::from_utf8(v).ok());
    let last_upvote_at = storage
        .get(MOLTBOOK_LAST_UPVOTE_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| String::from_utf8(v).ok());
    let last_engagement_at = storage
        .get(MOLTBOOK_LAST_ENGAGEMENT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| String::from_utf8(v).ok());
    let last_run_stats = storage
        .get(MOLTBOOK_LAST_RUN_STATS_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_slice::<serde_json::Value>(&v).ok());
    let busy_reasons = if settings.defer_when_busy {
        server_load_reasons(&state).await
    } else {
        Vec::<String>::new()
    };

    Json(serde_json::json!({
        "enabled": settings.enabled,
        "mode": settings.mode,
        "sync_frequency": settings.sync_frequency,
        "write_enabled": settings.write_enabled,
        "defer_when_busy": settings.defer_when_busy,
        "running": is_moltbook_running(),
        "has_api_key": has_api_key,
        "last_run_at": last_run_at,
        "next_run_at": next_run_at,
        "last_status": last_status,
        "last_post_at": last_post_at,
        "last_comment_at": last_comment_at,
        "last_upvote_at": last_upvote_at,
        "last_engagement_at": last_engagement_at,
        "last_run_stats": last_run_stats,
        "busy_reasons": busy_reasons,
        "connector_registered": has_connector,
    }))
}

/// Get Moltbook activity log.
pub(super) async fn get_moltbook_log(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50usize)
        .min(500);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0usize);
    let storage = { state.agent.read().await.storage.clone() };
    let all = load_moltbook_activity(&storage).await;
    let total = all.len();
    let events: Vec<_> = all.into_iter().rev().skip(offset).take(limit).collect();
    Json(serde_json::json!({
        "events": events,
        "total": total,
        "limit": limit,
        "offset": offset
    }))
}

/// Trigger Moltbook run immediately.
pub(super) async fn run_moltbook_now(State(state): State<AppState>) -> Json<serde_json::Value> {
    let Some(run_guard) = try_start_moltbook_run() else {
        return Json(serde_json::json!({
            "status": "running",
            "message": "Moltbook run already in progress"
        }));
    };

    let state_for_run = state.clone();
    tokio::spawn(async move {
        let _ = run_moltbook_cycle_with_guard(&state_for_run, "manual", run_guard).await;
    });

    Json(serde_json::json!({
        "status": "started",
        "message": "Moltbook run started in the background"
    }))
}
