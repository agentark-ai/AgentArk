use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const ARKDISTILL_PROFILE_KEY: &str = "tool_output_distill_profile_v1";
pub const ARKDISTILL_PROFILE_BASELINE_SNAPSHOT_KEY: &str =
    "tool_output_distill_profile_baseline_snapshot_v1";
pub const ARKDISTILL_PROFILE_LAST_RESULT_KEY: &str = "tool_output_distill_profile_last_result_v1";
pub const ARKDISTILL_CANDIDATE_TYPE: &str = "tool_output_distill_profile";
pub const ARKDISTILL_EVENT_TYPE: &str = "arkdistill_tool_output";
pub const ARKDISTILL_DEFAULT_PROFILE_ID: &str = "builtin";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArkDistillLimits {
    #[serde(default = "default_max_string_chars")]
    pub max_string_chars: usize,
    #[serde(default = "default_head_chars")]
    pub head_chars: usize,
    #[serde(default = "default_tail_chars")]
    pub tail_chars: usize,
    #[serde(default = "default_max_array_items")]
    pub max_array_items: usize,
    #[serde(default = "default_max_object_keys")]
    pub max_object_keys: usize,
}

impl Default for ArkDistillLimits {
    fn default() -> Self {
        Self {
            max_string_chars: default_max_string_chars(),
            head_chars: default_head_chars(),
            tail_chars: default_tail_chars(),
            max_array_items: default_max_array_items(),
            max_object_keys: default_max_object_keys(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArkDistillRule {
    #[serde(default)]
    pub primitive: Option<String>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub field_names: Vec<String>,
    #[serde(default)]
    pub field_paths: Vec<String>,
    #[serde(default)]
    pub omit_field_names: Vec<String>,
    #[serde(default)]
    pub max_string_chars: Option<usize>,
    #[serde(default)]
    pub head_chars: Option<usize>,
    #[serde(default)]
    pub tail_chars: Option<usize>,
    #[serde(default = "default_true")]
    pub dedup_lines: bool,
    #[serde(default = "default_true")]
    pub fold_whitespace: bool,
    #[serde(default)]
    pub html_to_text: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArkDistillProfile {
    #[serde(default = "default_profile_version")]
    pub version: u32,
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub generic_limits: ArkDistillLimits,
    #[serde(default = "default_required_fields")]
    pub required_fields: Vec<String>,
    #[serde(default = "default_rules")]
    pub rules: Vec<ArkDistillRule>,
}

impl Default for ArkDistillProfile {
    fn default() -> Self {
        Self {
            version: default_profile_version(),
            profile_id: default_profile_id(),
            enabled: true,
            generic_limits: ArkDistillLimits::default(),
            required_fields: default_required_fields(),
            rules: default_rules(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ArkDistillStats {
    pub profile_id: String,
    pub original_chars: usize,
    pub distilled_chars: usize,
    pub saved_chars: usize,
    pub estimated_original_tokens: usize,
    pub estimated_distilled_tokens: usize,
    pub estimated_saved_tokens: usize,
    pub transformed_fields: Vec<String>,
    pub truncated_fields: usize,
    pub omitted_fields: usize,
    pub omitted_array_items: usize,
    pub omitted_object_keys: usize,
    pub deduplicated_lines: usize,
    pub json_valid: bool,
}

#[derive(Debug, Clone)]
pub struct ArkDistillOutput {
    pub value: serde_json::Value,
    pub stats: ArkDistillStats,
}

#[derive(Debug, Clone)]
pub struct ExternalArkDistillCandidate {
    pub source: String,
    pub profile: ArkDistillProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArkDistillProfileEval {
    pub profile_id: String,
    pub saved_chars: usize,
    pub estimated_saved_tokens: usize,
    pub required_fields_preserved: bool,
    pub json_valid: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArkDistillEvaluationResult {
    pub mode: String,
    pub request: String,
    pub baseline: ArkDistillProfileEval,
    pub best_candidate: ArkDistillProfileEval,
    pub candidate_source: String,
    pub promoted: bool,
    pub promotion_gate: String,
    pub promotion_gate_summary: String,
    pub promoted_profile: Option<ArkDistillProfile>,
}

pub fn parse_arkdistill_profile(raw: &[u8]) -> Option<ArkDistillProfile> {
    serde_json::from_slice::<ArkDistillProfile>(raw)
        .ok()
        .map(sanitize_arkdistill_profile)
}

pub async fn load_arkdistill_profile(storage: &crate::storage::Storage) -> ArkDistillProfile {
    storage
        .get(ARKDISTILL_PROFILE_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| parse_arkdistill_profile(&raw))
        .unwrap_or_default()
}

pub fn sanitize_arkdistill_profile(mut profile: ArkDistillProfile) -> ArkDistillProfile {
    if profile.version == 0 {
        profile.version = default_profile_version();
    }
    profile.profile_id = clean_identifier(&profile.profile_id, ARKDISTILL_DEFAULT_PROFILE_ID);
    profile.generic_limits.max_string_chars =
        profile.generic_limits.max_string_chars.clamp(256, 64_000);
    profile.generic_limits.head_chars = profile.generic_limits.head_chars.clamp(64, 48_000);
    profile.generic_limits.tail_chars = profile.generic_limits.tail_chars.clamp(0, 16_000);
    profile.generic_limits.max_array_items = profile.generic_limits.max_array_items.clamp(1, 128);
    profile.generic_limits.max_object_keys = profile.generic_limits.max_object_keys.clamp(8, 512);
    profile.required_fields = normalize_string_list(profile.required_fields, 128, 128);
    if profile.required_fields.is_empty() {
        profile.required_fields = default_required_fields();
    }
    profile.rules = profile
        .rules
        .into_iter()
        .take(96)
        .map(sanitize_rule)
        .collect();
    if profile.rules.is_empty() {
        profile.rules = default_rules();
    }
    profile
}

pub fn distill_tool_output_for_model(
    primitive: &str,
    action_name: &str,
    value: serde_json::Value,
    profile: &ArkDistillProfile,
) -> ArkDistillOutput {
    let profile = sanitize_arkdistill_profile(profile.clone());
    let original_chars = json_chars(&value);
    let mut stats = ArkDistillStats {
        profile_id: profile.profile_id.clone(),
        original_chars,
        estimated_original_tokens: estimate_tokens(original_chars),
        json_valid: true,
        ..ArkDistillStats::default()
    };
    let mut value = value;
    if profile.enabled {
        let required = normalized_set(&profile.required_fields);
        distill_value(
            primitive,
            action_name,
            &mut value,
            "",
            None,
            &profile,
            &required,
            &mut stats,
        );
    }
    stats.distilled_chars = json_chars(&value);
    stats.saved_chars = stats.original_chars.saturating_sub(stats.distilled_chars);
    stats.estimated_distilled_tokens = estimate_tokens(stats.distilled_chars);
    stats.estimated_saved_tokens = stats
        .estimated_original_tokens
        .saturating_sub(stats.estimated_distilled_tokens);
    stats.transformed_fields.sort();
    stats.transformed_fields.dedup();
    ArkDistillOutput { value, stats }
}

pub fn arkdistill_contract() -> serde_json::Value {
    serde_json::json!({
        "surface": "arkdistill_profile",
        "kv_key": ARKDISTILL_PROFILE_KEY,
        "candidate_type": ARKDISTILL_CANDIDATE_TYPE,
        "runtime_contract": "deterministic model-visible tool output compaction; no live LLM summarizer in the chat turn",
        "profile_fields": [
            "version",
            "profile_id",
            "enabled",
            "generic_limits",
            "required_fields",
            "rules"
        ],
        "rule_selectors": ["primitive", "action", "field_names", "field_paths"],
        "promotion_requirements": [
            "preserve JSON validity",
            "preserve required fields",
            "reduce model-visible context on noisy tool outputs",
            "avoid increased correction or retry rate"
        ]
    })
}

pub fn validate_arkdistill_candidate(profile: &ArkDistillProfile) -> Result<(), String> {
    let profile = sanitize_arkdistill_profile(profile.clone());
    if profile.profile_id.trim().is_empty() {
        return Err("ArkDistill profile requires a profile_id".to_string());
    }
    if profile
        .generic_limits
        .head_chars
        .saturating_add(profile.generic_limits.tail_chars)
        > profile
            .generic_limits
            .max_string_chars
            .saturating_add(16_000)
    {
        return Err("ArkDistill profile head/tail limits are inconsistent".to_string());
    }
    let eval = evaluate_profile_on_fixtures(&profile);
    if !eval.json_valid {
        return Err("ArkDistill profile produced invalid JSON on replay fixtures".to_string());
    }
    if !eval.required_fields_preserved {
        return Err("ArkDistill profile drops required fields on replay fixtures".to_string());
    }
    Ok(())
}

pub fn evaluate_external_arkdistill_candidates(
    request: &str,
    current_profile_raw: Option<&[u8]>,
    candidates: Vec<ExternalArkDistillCandidate>,
) -> anyhow::Result<ArkDistillEvaluationResult> {
    let baseline_profile = current_profile_raw
        .and_then(parse_arkdistill_profile)
        .unwrap_or_default();
    let baseline = evaluate_profile_on_fixtures(&baseline_profile);
    let mut best: Option<(String, ArkDistillProfile, ArkDistillProfileEval)> = None;
    for candidate in candidates {
        let profile = sanitize_arkdistill_profile(candidate.profile);
        if validate_arkdistill_candidate(&profile).is_err() {
            continue;
        }
        let eval = evaluate_profile_on_fixtures(&profile);
        if best.as_ref().is_none_or(|(_, _, current)| {
            eval.estimated_saved_tokens > current.estimated_saved_tokens
        }) {
            best = Some((candidate.source, profile, eval));
        }
    }
    let Some((candidate_source, profile, best_candidate)) = best else {
        anyhow::bail!("no valid ArkDistill profile candidates");
    };
    let meaningful_gain = best_candidate
        .estimated_saved_tokens
        .saturating_sub(baseline.estimated_saved_tokens)
        >= 100;
    let promoted =
        best_candidate.required_fields_preserved && best_candidate.json_valid && meaningful_gain;
    let promotion_gate = if promoted {
        "passed".to_string()
    } else if !best_candidate.required_fields_preserved {
        "rejected: required_fields_not_preserved".to_string()
    } else if !best_candidate.json_valid {
        "rejected: invalid_json".to_string()
    } else {
        "rejected: insufficient_context_savings_gain".to_string()
    };
    let promotion_gate_summary = if promoted {
        "ArkDistill profile improved fixture context savings while preserving required structured fields."
            .to_string()
    } else {
        "ArkDistill profile was not promoted because it did not improve savings enough under replay fixtures."
            .to_string()
    };
    Ok(ArkDistillEvaluationResult {
        mode: "arkdistill_profile".to_string(),
        request: crate::security::redact_pii(&request.chars().take(1_200).collect::<String>()),
        baseline,
        best_candidate,
        candidate_source,
        promoted,
        promotion_gate,
        promotion_gate_summary,
        promoted_profile: promoted.then_some(profile),
    })
}

fn evaluate_profile_on_fixtures(profile: &ArkDistillProfile) -> ArkDistillProfileEval {
    let mut saved_chars = 0usize;
    let mut estimated_saved_tokens = 0usize;
    let mut required_fields_preserved = true;
    for (primitive, action, value) in arkdistill_replay_fixtures() {
        let expected_status = value.pointer("/data/status").cloned();
        let expected_url = value.pointer("/data/url").cloned();
        let output = distill_tool_output_for_model(primitive, action, value, profile);
        saved_chars = saved_chars.saturating_add(output.stats.saved_chars);
        estimated_saved_tokens =
            estimated_saved_tokens.saturating_add(output.stats.estimated_saved_tokens);
        required_fields_preserved &= expected_status
            .as_ref()
            .is_none_or(|expected| output.value.pointer("/data/status") == Some(expected));
        required_fields_preserved &= expected_url
            .as_ref()
            .is_none_or(|expected| output.value.pointer("/data/url") == Some(expected));
    }
    ArkDistillProfileEval {
        profile_id: profile.profile_id.clone(),
        saved_chars,
        estimated_saved_tokens,
        required_fields_preserved,
        json_valid: true,
    }
}

fn arkdistill_replay_fixtures() -> Vec<(&'static str, &'static str, serde_json::Value)> {
    let repeated_log = (0..300)
        .map(|_| "Repeated browser trace line with boilerplate navigation and layout state")
        .collect::<Vec<_>>()
        .join("\n");
    vec![
        (
            "fetch",
            "page_fetch",
            serde_json::json!({
                "tool": "page_fetch",
                "status": "completed",
                "data": {
                    "url": "https://example.com/source",
                    "status": "ok",
                    "content": repeated_log,
                }
            }),
        ),
        (
            "browse",
            "browser_snapshot",
            serde_json::json!({
                "tool": "browser_snapshot",
                "status": "completed",
                "data": {
                    "url": "https://example.com/app",
                    "status": "ok",
                    "screenshot_base64": "A".repeat(10_000),
                    "body_text": "Loaded page",
                }
            }),
        ),
    ]
}

fn distill_value(
    primitive: &str,
    action_name: &str,
    value: &mut serde_json::Value,
    path: &str,
    key: Option<&str>,
    profile: &ArkDistillProfile,
    required: &HashSet<String>,
    stats: &mut ArkDistillStats,
) {
    match value {
        serde_json::Value::Object(object) => {
            let mut keys = object.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            if object.len() > profile.generic_limits.max_object_keys {
                let mut retained = serde_json::Map::new();
                for key in &keys {
                    if retained.len() >= profile.generic_limits.max_object_keys
                        && !is_required_field(path_for(path, key).as_str(), key, required)
                    {
                        continue;
                    }
                    if let Some(value) = object.get(key) {
                        retained.insert(key.clone(), value.clone());
                    }
                }
                stats.omitted_object_keys += object.len().saturating_sub(retained.len());
                *object = retained;
            }
            let keys = object.keys().cloned().collect::<Vec<_>>();
            for child_key in keys {
                let child_path = path_for(path, &child_key);
                if let Some(child) = object.get_mut(&child_key) {
                    distill_value(
                        primitive,
                        action_name,
                        child,
                        &child_path,
                        Some(&child_key),
                        profile,
                        required,
                        stats,
                    );
                }
            }
        }
        serde_json::Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                let child_path = format!("{}[{}]", path, index);
                distill_value(
                    primitive,
                    action_name,
                    item,
                    &child_path,
                    None,
                    profile,
                    required,
                    stats,
                );
            }
            if items.len() > profile.generic_limits.max_array_items {
                let omitted = items
                    .len()
                    .saturating_sub(profile.generic_limits.max_array_items);
                items.truncate(profile.generic_limits.max_array_items);
                items.push(serde_json::json!({ "arkdistill_omitted_items": omitted }));
                stats.omitted_array_items += omitted;
                stats.transformed_fields.push(path.to_string());
            }
        }
        serde_json::Value::String(text) => {
            let field_key = key.unwrap_or_default();
            if is_required_field(path, field_key, required) {
                return;
            }
            if rule_omits_field(primitive, action_name, path, field_key, &profile.rules) {
                let chars = text.chars().count();
                *value = serde_json::Value::String(format!("[base64 omitted: {} chars]", chars));
                stats.omitted_fields += 1;
                stats.transformed_fields.push(path.to_string());
                return;
            }
            let selected_rule =
                select_rule(primitive, action_name, path, field_key, &profile.rules);
            let limit = selected_rule
                .and_then(|rule| rule.max_string_chars)
                .unwrap_or(profile.generic_limits.max_string_chars);
            let original = text.clone();
            let mut next = original.clone();
            if selected_rule.is_some_and(|rule| rule.html_to_text) {
                next = html_to_text(&next);
            }
            if selected_rule.map(|rule| rule.dedup_lines).unwrap_or(true) {
                let (deduped, omitted) = dedup_lines(&next);
                if omitted > 0 {
                    next = deduped;
                    stats.deduplicated_lines += omitted;
                }
            }
            if selected_rule
                .map(|rule| rule.fold_whitespace)
                .unwrap_or(false)
            {
                next = fold_whitespace_preserving_lines(&next);
            }
            if next.chars().count() > limit {
                let head = selected_rule
                    .and_then(|rule| rule.head_chars)
                    .unwrap_or(profile.generic_limits.head_chars)
                    .min(limit);
                let tail = selected_rule
                    .and_then(|rule| rule.tail_chars)
                    .unwrap_or(profile.generic_limits.tail_chars)
                    .min(limit.saturating_sub(head));
                next = head_tail_excerpt(&next, head, tail);
                stats.truncated_fields += 1;
            }
            if next != original {
                *value = serde_json::Value::String(next);
                stats.transformed_fields.push(path.to_string());
            }
        }
        _ => {}
    }
}

fn sanitize_rule(mut rule: ArkDistillRule) -> ArkDistillRule {
    rule.primitive = normalize_selector(rule.primitive);
    rule.action = normalize_selector(rule.action);
    rule.field_names = normalize_string_list(rule.field_names, 96, 96);
    rule.field_paths = normalize_string_list(rule.field_paths, 96, 160);
    rule.omit_field_names = normalize_string_list(rule.omit_field_names, 96, 96);
    rule.max_string_chars = rule.max_string_chars.map(|value| value.clamp(128, 64_000));
    rule.head_chars = rule.head_chars.map(|value| value.clamp(32, 48_000));
    rule.tail_chars = rule.tail_chars.map(|value| value.clamp(0, 16_000));
    rule
}

fn select_rule<'a>(
    primitive: &str,
    action_name: &str,
    path: &str,
    field_key: &str,
    rules: &'a [ArkDistillRule],
) -> Option<&'a ArkDistillRule> {
    rules.iter().find(|rule| {
        rule_matches_tool(rule, primitive, action_name)
            && (matches_list(&rule.field_names, field_key) || matches_list(&rule.field_paths, path))
    })
}

fn rule_omits_field(
    primitive: &str,
    action_name: &str,
    path: &str,
    field_key: &str,
    rules: &[ArkDistillRule],
) -> bool {
    rules.iter().any(|rule| {
        rule_matches_tool(rule, primitive, action_name)
            && !rule.omit_field_names.is_empty()
            && (matches_list(&rule.omit_field_names, field_key)
                || matches_list(&rule.field_paths, path))
    })
}

fn rule_matches_tool(rule: &ArkDistillRule, primitive: &str, action_name: &str) -> bool {
    selector_matches(rule.primitive.as_deref(), primitive)
        && selector_matches(rule.action.as_deref(), action_name)
}

fn selector_matches(selector: Option<&str>, actual: &str) -> bool {
    selector
        .map(|value| value.eq_ignore_ascii_case(actual.trim()))
        .unwrap_or(true)
}

fn is_required_field(path: &str, field_key: &str, required: &HashSet<String>) -> bool {
    let field_key = field_key.trim().to_ascii_lowercase();
    let path = path.trim().to_ascii_lowercase();
    (!field_key.is_empty() && required.contains(&field_key))
        || (!path.is_empty() && required.contains(&path))
}

fn matches_list(values: &[String], actual: &str) -> bool {
    let actual = actual.trim().to_ascii_lowercase();
    !actual.is_empty() && values.iter().any(|value| value == &actual)
}

fn normalized_set(values: &[String]) -> HashSet<String> {
    values
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

fn normalize_string_list(values: Vec<String>, max_items: usize, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for value in values.into_iter().take(max_items) {
        let cleaned = value
            .trim()
            .chars()
            .take(max_chars)
            .collect::<String>()
            .to_ascii_lowercase();
        if !cleaned.is_empty() && seen.insert(cleaned.clone()) {
            out.push(cleaned);
        }
    }
    out
}

fn normalize_selector(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().chars().take(96).collect::<String>())
        .filter(|value| !value.is_empty())
}

fn clean_identifier(raw: &str, fallback: &str) -> String {
    let cleaned = raw
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        .take(96)
        .collect::<String>();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

fn dedup_lines(text: &str) -> (String, usize) {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut omitted = 0usize;
    for line in text.lines() {
        let normalized = line.trim();
        if normalized.is_empty() {
            if out.last().is_some_and(|last: &String| last.is_empty()) {
                omitted += 1;
                continue;
            }
            out.push(String::new());
            continue;
        }
        if seen.insert(normalized.to_string()) {
            out.push(line.to_string());
        } else {
            omitted += 1;
        }
    }
    if omitted > 0 {
        out.push(format!("...[omitted {} repeated lines]...", omitted));
    }
    (out.join("\n"), omitted)
}

fn fold_whitespace_preserving_lines(text: &str) -> String {
    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn html_to_text(text: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    fold_whitespace_preserving_lines(&out)
}

fn head_tail_excerpt(text: &str, head_chars: usize, tail_chars: usize) -> String {
    let total = text.chars().count();
    if total <= head_chars.saturating_add(tail_chars) {
        return text.to_string();
    }
    let head = text.chars().take(head_chars).collect::<String>();
    let tail = text
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!(
        "{}\n...[omitted {} chars by ArkDistill]...\n{}",
        head,
        total.saturating_sub(head_chars.saturating_add(tail_chars)),
        tail
    )
}

fn path_for(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        child.to_string()
    } else {
        format!("{}.{}", parent, child)
    }
}

fn json_chars(value: &serde_json::Value) -> usize {
    value.to_string().chars().count()
}

fn estimate_tokens(chars: usize) -> usize {
    chars.div_ceil(4)
}

fn default_profile_version() -> u32 {
    1
}

fn default_profile_id() -> String {
    ARKDISTILL_DEFAULT_PROFILE_ID.to_string()
}

fn default_true() -> bool {
    true
}

fn default_max_string_chars() -> usize {
    8_000
}

fn default_head_chars() -> usize {
    6_000
}

fn default_tail_chars() -> usize {
    1_200
}

fn default_max_array_items() -> usize {
    24
}

fn default_max_object_keys() -> usize {
    96
}

fn default_required_fields() -> Vec<String> {
    [
        "ok",
        "status",
        "tool",
        "primitive",
        "url",
        "id",
        "app_id",
        "download_url",
        "message",
        "reason",
        "error",
    ]
    .into_iter()
    .map(ToString::to_string)
    .collect()
}

fn default_rules() -> Vec<ArkDistillRule> {
    vec![
        ArkDistillRule {
            primitive: None,
            action: None,
            field_names: [
                "content",
                "body_text",
                "text",
                "markdown",
                "html",
                "stdout",
                "stderr",
                "logs",
                "trace",
                "raw",
            ]
            .into_iter()
            .map(ToString::to_string)
            .collect(),
            field_paths: Vec::new(),
            omit_field_names: Vec::new(),
            max_string_chars: Some(8_000),
            head_chars: Some(6_000),
            tail_chars: Some(1_200),
            dedup_lines: true,
            fold_whitespace: false,
            html_to_text: false,
        },
        ArkDistillRule {
            primitive: None,
            action: None,
            field_names: Vec::new(),
            field_paths: Vec::new(),
            omit_field_names: [
                "image_base64",
                "screenshot_base64",
                "audio_base64",
                "video_base64",
                "blob_base64",
            ]
            .into_iter()
            .map(ToString::to_string)
            .collect(),
            max_string_chars: None,
            head_chars: None,
            tail_chars: None,
            dedup_lines: false,
            fold_whitespace: false,
            html_to_text: false,
        },
        ArkDistillRule {
            primitive: None,
            action: None,
            field_names: ["html"].into_iter().map(ToString::to_string).collect(),
            field_paths: Vec::new(),
            omit_field_names: Vec::new(),
            max_string_chars: Some(8_000),
            head_chars: Some(6_000),
            tail_chars: Some(1_200),
            dedup_lines: true,
            fold_whitespace: true,
            html_to_text: true,
        },
    ]
}
