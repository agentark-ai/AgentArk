//! Helpers for storing user-provided secrets safely.
//!
//! Design goals:
//! - Support action secret placeholders (`{{secret:KEY}}`, `{{env:ENV_NAME}}`) which expect
//!   `secret:KEY` / `env:ENV_NAME` keys in `secrets.enc`.
//! - Support legacy integration connectors that look up un-prefixed custom keys like `github_token`.
//! - Avoid storing secrets only under one namespace (to keep UX simple for chat-based flows).

use anyhow::Result;
use std::path::Path;

fn is_env_var_style_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn env_alias_to_custom_key(env: &str) -> Option<&'static str> {
    // Mirrors runtime legacy aliases + external integration secret names.
    Some(match env {
        "GITHUB_TOKEN" => "github_token",
        "NOTION_TOKEN" => "notion_token",
        "TWITTER_BEARER_TOKEN" => "twitter_bearer_token",
        "ONEPASSWORD_TOKEN" => "onepassword_token",
        "ONEPASSWORD_HOST" => "onepassword_host",
        "GOOGLE_PLACES_API_KEY" => "google_places_api_key",
        "TWILIO_ACCOUNT_SID" => "twilio_account_sid",
        "TWILIO_AUTH_TOKEN" => "twilio_auth_token",
        "TWILIO_FROM_NUMBER" => "twilio_from_number",
        "ORDERING_CONFIG_JSON" => "ordering_config",
        "SHOPIFY_ACCESS_TOKEN" => "shopify_access_token",
        "SHOPIFY_STORE_URL" => "shopify_store_url",
        "ORDERING_WEBHOOK_URL" => "ordering_webhook_url",
        "GARMIN_TOKEN" => "garmin_token",
        "GARMIN_API_BASE" => "garmin_api_base",
        "WHOOP_TOKEN" => "whoop_token",
        "GA4_ACCESS_TOKEN" => "ga4_access_token",
        "GA4_PROPERTY_ID" => "ga4_property_id",
        "GSC_ACCESS_TOKEN" => "gsc_access_token",
        "GSC_SITE_URL" => "gsc_site_url",
        "SOCIAL_TWITTER_BEARER_TOKEN" => "social_twitter_bearer_token",
        "SOCIAL_GA4_ACCESS_TOKEN" => "social_ga4_access_token",
        "SOCIAL_GA4_PROPERTY_ID" => "social_ga4_property_id",
        "MOLTBOOK_API_KEY" => "moltbook_api_key",
        _ => return None,
    })
}

fn keys_to_write(user_key: &str) -> Vec<String> {
    let k = user_key.trim();
    if k.is_empty() {
        return vec![];
    }

    // Allow explicit namespaces.
    if k.starts_with("env:") || k.starts_with("secret:") {
        return vec![k.to_string()];
    }

    if is_env_var_style_key(k) {
        let mut out = vec![format!("env:{}", k)];
        if let Some(custom) = env_alias_to_custom_key(k) {
            out.push(custom.to_string()); // legacy integration key
        }
        return out;
    }

    // Default: store both modern and legacy forms.
    vec![format!("secret:{}", k), k.to_string()]
}

/// Returns the concrete storage keys that will be written/checked for a user-provided key.
///
/// Examples:
/// - `GITHUB_TOKEN` -> `["env:GITHUB_TOKEN", "github_token"]`
/// - `github_token` -> `["secret:github_token", "github_token"]`
/// - `env:OPENAI_API_KEY` -> `["env:OPENAI_API_KEY"]`
pub fn storage_keys_for_user_key(user_key: &str) -> Vec<String> {
    keys_to_write(user_key)
}

/// Parse `set secret` command syntax:
/// - `set secret KEY=VALUE`
/// - `set secret KEY VALUE`
pub fn parse_set_secret_command(message: &str) -> Option<(String, String)> {
    let trimmed = message.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("set secret ") {
        return None;
    }
    let rest = trimmed[10..].trim(); // len("set secret ") == 10
    if rest.is_empty() {
        return None;
    }

    let (key, value) = if let Some(eq) = rest.find('=') {
        let (k, v) = rest.split_at(eq);
        (k.trim(), v[1..].trim())
    } else {
        let mut parts = rest.splitn(2, char::is_whitespace);
        let k = parts.next().unwrap_or("").trim();
        let v = parts.next().unwrap_or("").trim();
        (k, v)
    };

    if key.is_empty() || value.is_empty() {
        return None;
    }
    if key.chars().any(|c| c.is_whitespace()) {
        return None;
    }
    if key.contains('\n') || key.contains('\r') {
        return None;
    }
    Some((key.to_string(), value.to_string()))
}

/// Parse a request to reuse the currently configured LLM credential for a target key.
///
/// Supported forms:
/// - `use current llm key for OPENAI_API_KEY`
/// - `use current model key for OPENAI_API_KEY`
/// - `use llm key OPENAI_API_KEY`
pub fn parse_use_current_llm_key_command(message: &str) -> Option<String> {
    let trimmed = message.trim();
    let lower = trimmed.to_ascii_lowercase();
    let prefixes = [
        "use current llm key for ",
        "use current model key for ",
        "use llm key ",
        "use model key ",
    ];
    let mut rest = None;
    for prefix in prefixes {
        if lower.starts_with(prefix) && trimmed.len() >= prefix.len() {
            rest = Some(trimmed[prefix.len()..].trim());
            break;
        }
    }
    let key = rest?;
    if key.is_empty() {
        return None;
    }
    if key.chars().any(|c| c.is_whitespace()) {
        return None;
    }
    if key.contains('\n') || key.contains('\r') {
        return None;
    }
    Some(key.to_string())
}

pub fn store_user_secret(
    config_dir: &Path,
    data_dir: Option<&Path>,
    user_key: &str,
    value: &str,
) -> Result<Vec<String>> {
    let keys = keys_to_write(user_key);
    if keys.is_empty() {
        return Ok(vec![]);
    }
    let v = value.to_string();

    let mgr = crate::core::config::SecureConfigManager::new_with_data_dir(config_dir, data_dir)?;
    mgr.update_custom_secrets(|custom| {
        for k in &keys {
            custom.insert(k.to_string(), v.clone());
        }
        Ok(())
    })?;
    Ok(keys)
}

pub fn has_user_secret(custom: &std::collections::HashMap<String, String>, user_key: &str) -> bool {
    let keys = keys_to_write(user_key);
    keys.iter()
        .any(|k| custom.get(k).is_some_and(|v| !v.trim().is_empty()))
}
