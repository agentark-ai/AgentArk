use serde::Serialize;

use super::llm::LlmClient;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HistoryTokenBudget {
    pub(crate) history_tokens: usize,
    pub(crate) summary_tokens: usize,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HistoryBudgetConfig {
    pub(crate) scope_env: &'static str,
    pub(crate) default_context_window_tokens: usize,
    pub(crate) default_budget_ratio_percent: usize,
    pub(crate) min_history_token_budget: usize,
    pub(crate) max_summary_tokens: usize,
}

pub(crate) fn estimate_tokens_from_text(value: &str) -> usize {
    value.chars().count().saturating_add(3) / 4
}

pub(crate) fn estimate_role_message_tokens(role: &str, content: &str) -> usize {
    estimate_tokens_from_text(role)
        .saturating_add(estimate_tokens_from_text(content))
        .saturating_add(4)
}

pub(crate) fn estimate_json_tokens<T: Serialize + ?Sized>(value: &T) -> usize {
    serde_json::to_string(value)
        .ok()
        .map(|raw| estimate_tokens_from_text(&raw))
        .unwrap_or(0)
}

pub(crate) fn read_usize_env(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
}

pub(crate) fn truncate_to_token_budget(value: &str, max_tokens: usize) -> String {
    truncate_chars_preserving_whitespace(value, max_tokens.saturating_mul(4))
}

pub(crate) fn truncate_point_tokens(value: &str, max_tokens: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let max_chars = max_tokens.saturating_mul(4);
    if compact.chars().count() <= max_chars {
        return compact;
    }
    let mut out = compact
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    out.push_str("...");
    out
}

pub(crate) fn context_window_tokens_for_llm(llm: &LlmClient, config: HistoryBudgetConfig) -> usize {
    let provider = llm.provider_name();
    let model = llm.model_name();
    configured_context_window_tokens(provider, model, config.scope_env)
        .or_else(|| context_window_from_model_hint(model))
        .unwrap_or_else(|| fallback_context_window_tokens(provider, config))
}

pub(crate) fn history_budget_for_llm(
    llm: &LlmClient,
    config: HistoryBudgetConfig,
    fixed_prompt_tokens: usize,
) -> HistoryTokenBudget {
    let history_budget_env = scoped_env_name(config.scope_env, "HISTORY_TOKEN_BUDGET");
    if let Some(history_tokens) = read_usize_env(&history_budget_env) {
        return HistoryTokenBudget {
            history_tokens,
            summary_tokens: summary_budget_from_history_budget(config, history_tokens),
        };
    }

    let context_tokens =
        context_window_tokens_for_llm(llm, config).max(config.min_history_token_budget);
    let ratio_env = scoped_env_name(config.scope_env, "HISTORY_BUDGET_RATIO_PERCENT");
    let ratio_percent = read_usize_env(&ratio_env)
        .unwrap_or(config.default_budget_ratio_percent)
        .clamp(5, 80);
    let reserved_output_env = scoped_env_name(config.scope_env, "RESERVED_OUTPUT_TOKENS");
    let reserved_output_tokens = read_usize_env(&reserved_output_env)
        .unwrap_or_else(|| (context_tokens / 8).clamp(1_024, 8_192));
    let available_tokens = context_tokens
        .saturating_sub(fixed_prompt_tokens)
        .saturating_sub(reserved_output_tokens);
    let proportional_budget = context_tokens.saturating_mul(ratio_percent) / 100;
    let history_tokens = available_tokens
        .min(proportional_budget)
        .max(available_tokens.min(config.min_history_token_budget))
        .max(256);

    HistoryTokenBudget {
        history_tokens,
        summary_tokens: summary_budget_from_history_budget(config, history_tokens),
    }
}

pub(crate) fn context_window_from_model_hint(model: &str) -> Option<usize> {
    model
        .to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|segment| !segment.is_empty())
        .filter_map(context_marker_tokens_from_segment)
        .max()
}

fn summary_budget_from_history_budget(config: HistoryBudgetConfig, history_tokens: usize) -> usize {
    let summary_env = scoped_env_name(config.scope_env, "HISTORY_SUMMARY_TOKEN_BUDGET");
    read_usize_env(&summary_env)
        .unwrap_or_else(|| history_tokens.saturating_mul(35) / 100)
        .clamp(256, config.max_summary_tokens)
        .min(history_tokens.max(1))
}

fn truncate_chars_preserving_whitespace(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out = value
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    out.push_str("...");
    out
}

fn scoped_env_name(scope: &str, suffix: &str) -> String {
    format!("AGENTARK_{}_{}", env_key_suffix(scope), suffix)
}

fn env_key_suffix(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn context_marker_tokens_from_segment(segment: &str) -> Option<usize> {
    let suffix = segment.chars().last()?;
    let multiplier = match suffix {
        'k' => 1_000usize,
        'm' => 1_000_000usize,
        _ => return None,
    };
    let stem = &segment[..segment.len().saturating_sub(suffix.len_utf8())];
    let digits = stem
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    let value = digits.parse::<usize>().ok()?;
    value
        .checked_mul(multiplier)
        .filter(|tokens| (2_000..=2_000_000).contains(tokens))
}

fn configured_context_window_tokens(provider: &str, model: &str, scope: &str) -> Option<usize> {
    let provider_model_key = format!(
        "AGENTARK_MODEL_CONTEXT_TOKENS_{}_{}",
        env_key_suffix(provider),
        env_key_suffix(model)
    );
    let model_key = format!("AGENTARK_MODEL_CONTEXT_TOKENS_{}", env_key_suffix(model));
    let scoped_context_key = scoped_env_name(scope, "CONTEXT_TOKENS");
    read_usize_env(&provider_model_key)
        .or_else(|| read_usize_env(&model_key))
        .or_else(|| read_usize_env(&scoped_context_key))
        .or_else(|| read_usize_env("AGENTARK_MODEL_CONTEXT_TOKENS"))
}

fn fallback_context_window_tokens(provider: &str, config: HistoryBudgetConfig) -> usize {
    match provider {
        "anthropic" => 200_000,
        "ollama" => 8_192,
        "openai" | "openai-subscription" | "openrouter" | "openai-compatible" => 128_000,
        _ => config.default_context_window_tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_context_hint_reads_only_explicit_token_markers() {
        assert_eq!(
            context_window_from_model_hint("provider/model-128k"),
            Some(128_000)
        );
        assert_eq!(
            context_window_from_model_hint("gemini-1.5-pro-1m"),
            Some(1_000_000)
        );
        assert_eq!(context_window_from_model_hint("claude-20250514"), None);
    }

    #[test]
    fn token_truncation_stays_within_estimate() {
        let rendered = truncate_to_token_budget(&"abcd ".repeat(500), 100);

        assert!(estimate_tokens_from_text(&rendered) <= 101);
    }
}
