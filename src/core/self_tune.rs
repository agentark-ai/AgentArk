//! Self-Tune: Adaptive learning from user interactions
//!
//! Tracks tool success rates, learns user communication style,
//! adjusts autonomy confidence, and generates prompt hints —
//! all automatically from usage patterns.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const STYLE_PROFILE_KEY: &str = "self_tune:style_profile";
const TOOL_BIASES_KEY: &str = "self_tune:tool_biases";
const AUTONOMY_CONFIDENCE_KEY: &str = "self_tune:autonomy_confidence";
const TUNE_INTERACTION_COUNT_KEY: &str = "self_tune:interaction_count";

/// Analyze style every N interactions
const TUNE_INTERVAL: u64 = 20;

/// Consecutive autonomous successes before suggesting threshold increase
const AUTONOMY_THRESHOLD: u64 = 30;
/// Never auto-suggest above this risk score
const AUTONOMY_CEILING: u32 = 75;
/// Step size for autonomy suggestion increases
const AUTONOMY_STEP: u32 = 5;

// ── Data Structures ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserStyleProfile {
    pub preferred_length: String,
    pub preferred_format: String,
    pub domains: Vec<String>,
    pub tone_hints: Vec<String>,
    pub messages_analyzed: u64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolBiases {
    pub tools: HashMap<String, ToolStats>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolStats {
    pub successes: u64,
    pub failures: u64,
    pub avg_latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutonomyConfidence {
    pub consecutive_successes: u64,
    pub suggested_max_score: u32,
    pub last_rejection_at: Option<String>,
    pub updated_at: String,
}

// ── Tool Outcome Tracking ────────────────────────────────────────────────────

pub async fn record_tool_outcome(
    storage: &crate::storage::Storage,
    tool_name: &str,
    success: bool,
    latency_ms: u64,
) {
    let mut biases = load_tool_biases(storage).await;
    let stats = biases.tools.entry(tool_name.to_string()).or_default();
    if success {
        stats.successes += 1;
    } else {
        stats.failures += 1;
    }
    let total = stats.successes + stats.failures;
    if total > 0 {
        stats.avg_latency_ms =
            (stats.avg_latency_ms.saturating_mul(total - 1) + latency_ms) / total;
    }
    biases.updated_at = chrono::Utc::now().to_rfc3339();
    save_tool_biases(storage, &biases).await;
}

// ── Autonomy Confidence ──────────────────────────────────────────────────────

pub async fn record_autonomous_success(storage: &crate::storage::Storage) {
    let mut conf = load_autonomy_confidence(storage).await;
    conf.consecutive_successes += 1;
    if conf.consecutive_successes >= AUTONOMY_THRESHOLD {
        let current = conf.suggested_max_score;
        conf.suggested_max_score = (current + AUTONOMY_STEP).min(AUTONOMY_CEILING);
        conf.consecutive_successes = 0;
        tracing::info!(
            "Self-tune: autonomy confidence raised to {} after {} consecutive successes",
            conf.suggested_max_score,
            AUTONOMY_THRESHOLD
        );
    }
    conf.updated_at = chrono::Utc::now().to_rfc3339();
    save_autonomy_confidence(storage, &conf).await;
}

pub async fn record_user_rejection(storage: &crate::storage::Storage) {
    let mut conf = load_autonomy_confidence(storage).await;
    conf.consecutive_successes = 0;
    conf.last_rejection_at = Some(chrono::Utc::now().to_rfc3339());
    conf.updated_at = chrono::Utc::now().to_rfc3339();
    save_autonomy_confidence(storage, &conf).await;
}

// ── User Style Analysis (LLM-driven) ────────────────────────────────────────

pub async fn analyze_user_style(
    storage: &crate::storage::Storage,
    encrypted_storage: &crate::storage::encrypted::EncryptedStorage,
    llm: &crate::core::LlmClient,
) -> Result<UserStyleProfile> {
    let recent = encrypted_storage
        .get_recent_user_messages_decrypted(40)
        .await
        .unwrap_or_default();
    if recent.len() < 5 {
        return Ok(load_style_profile(storage).await);
    }

    let sample: Vec<String> = recent
        .iter()
        .take(20)
        .map(|m| m.content.chars().take(200).collect::<String>())
        .collect();
    let sample_text = sample.join("\n---\n");

    let prompt = format!(
        "Analyze these user messages and return a JSON object with these exact fields:\n\
        {{\"preferred_length\":\"concise|moderate|detailed\",\
        \"preferred_format\":\"bullets|prose|structured|mixed\",\
        \"domains\":[\"top 3 work domains\"],\
        \"tone_hints\":[\"any explicit feedback about response style\"]}}\n\
        \nMessages:\n{}",
        sample_text
    );

    let resp = llm
        .chat(
            "You analyze user communication patterns. Return JSON only, no markdown.",
            &prompt,
            &[],
            &[],
        )
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("Self-tune: LLM style analysis failed: {}", e);
            return Ok(load_style_profile(storage).await);
        }
    };

    let json_text = extract_json_object(&resp.content).unwrap_or_default();
    let mut profile: UserStyleProfile = serde_json::from_str(&json_text).unwrap_or_default();
    profile.messages_analyzed = recent.len() as u64;
    profile.updated_at = chrono::Utc::now().to_rfc3339();

    // Merge with existing tone hints (don't lose old feedback)
    let existing = load_style_profile(storage).await;
    for hint in &existing.tone_hints {
        if !profile.tone_hints.contains(hint) && profile.tone_hints.len() < 10 {
            profile.tone_hints.push(hint.clone());
        }
    }

    save_style_profile(storage, &profile).await;
    tracing::info!(
        "Self-tune: updated style profile (length={}, format={}, domains={:?})",
        profile.preferred_length,
        profile.preferred_format,
        profile.domains
    );
    Ok(profile)
}

// ── Prompt Block Generation ──────────────────────────────────────────────────

pub async fn build_self_tune_prompt_block(storage: &crate::storage::Storage) -> String {
    let mut lines = Vec::new();

    let style = load_style_profile(storage).await;
    if !style.preferred_length.is_empty() && style.messages_analyzed >= 5 {
        lines.push(format!(
            "- User prefers {} responses in {} format.",
            style.preferred_length, style.preferred_format
        ));
    }
    if !style.domains.is_empty() {
        lines.push(format!(
            "- User's primary domains: {}.",
            style.domains.join(", ")
        ));
    }
    for hint in style.tone_hints.iter().take(3) {
        if !hint.trim().is_empty() {
            lines.push(format!("- User feedback: {}", hint));
        }
    }

    let biases = load_tool_biases(storage).await;
    for (tool, stats) in &biases.tools {
        let total = stats.successes + stats.failures;
        if total >= 10 {
            let rate = stats.successes as f64 / total as f64;
            if rate < 0.5 {
                lines.push(format!(
                    "- Tool '{}' has low success rate ({:.0}%). Consider alternatives.",
                    tool,
                    rate * 100.0
                ));
            }
        }
    }

    let autonomy = load_autonomy_confidence(storage).await;
    if autonomy.suggested_max_score > 0 {
        lines.push(format!(
            "- Recent autonomous successes suggest the user may tolerate a higher autonomy ceiling (suggested max score: {}), but do not change settings unless the user explicitly asks.",
            autonomy.suggested_max_score
        ));
    }
    if autonomy.last_rejection_at.is_some() {
        lines.push(
            "- The user recently rejected an autonomous action. Be more conservative with proactive execution and prefer explicit confirmation for risky work."
                .to_string(),
        );
    }

    if lines.is_empty() {
        return String::new();
    }

    format!(
        "\n## Learned Preferences (auto-tuned)\n{}\n",
        lines.join("\n")
    )
}

// ── Interaction Counter ──────────────────────────────────────────────────────

pub async fn on_interaction_completed(
    storage: &crate::storage::Storage,
    encrypted_storage: &crate::storage::encrypted::EncryptedStorage,
    llm: &crate::core::LlmClient,
) {
    let count: u64 = storage
        .get(TUNE_INTERACTION_COUNT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let next = count + 1;
    let _ = storage
        .set(TUNE_INTERACTION_COUNT_KEY, next.to_string().as_bytes())
        .await;

    if next % TUNE_INTERVAL == 0 {
        tracing::info!(
            "Self-tune: analyzing user style after {} interactions",
            next
        );
        if let Err(e) = analyze_user_style(storage, encrypted_storage, llm).await {
            tracing::warn!("Self-tune style analysis failed: {}", e);
        }
    }
}

// ── Storage Helpers ──────────────────────────────────────────────────────────

async fn load_style_profile(storage: &crate::storage::Storage) -> UserStyleProfile {
    storage
        .get(STYLE_PROFILE_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

async fn save_style_profile(storage: &crate::storage::Storage, profile: &UserStyleProfile) {
    if let Ok(data) = serde_json::to_vec(profile) {
        let _ = storage.set(STYLE_PROFILE_KEY, &data).await;
    }
}

async fn load_tool_biases(storage: &crate::storage::Storage) -> ToolBiases {
    storage
        .get(TOOL_BIASES_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

async fn save_tool_biases(storage: &crate::storage::Storage, biases: &ToolBiases) {
    if let Ok(data) = serde_json::to_vec(biases) {
        let _ = storage.set(TOOL_BIASES_KEY, &data).await;
    }
}

async fn load_autonomy_confidence(storage: &crate::storage::Storage) -> AutonomyConfidence {
    storage
        .get(AUTONOMY_CONFIDENCE_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

async fn save_autonomy_confidence(storage: &crate::storage::Storage, conf: &AutonomyConfidence) {
    if let Ok(data) = serde_json::to_vec(conf) {
        let _ = storage.set(AUTONOMY_CONFIDENCE_KEY, &data).await;
    }
}

fn extract_json_object(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let mut depth = 0;
    for (i, ch) in text[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(text[start..start + i + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}
