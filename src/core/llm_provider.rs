use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const OPENAI_PROVIDER_ID: &str = "openai";
pub const OPENAI_SUBSCRIPTION_PROVIDER_ID: &str = "openai-subscription";
pub const OPENAI_COMPATIBLE_PROVIDER_ID: &str = "openai-compatible";
pub const OPENROUTER_PROVIDER_ID: &str = "openrouter";
pub const ANTHROPIC_PROVIDER_ID: &str = "anthropic";
pub const OLLAMA_PROVIDER_ID: &str = "ollama";
pub const HUGGINGFACE_PROVIDER_ID: &str = "huggingface";
pub const CODEX_CLI_BASE_URL: &str = "codex://cli";
pub const OPENAI_API_BASE_URL: &str = "https://api.openai.com/v1";
pub const OPENAI_CODEX_API_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
pub const OPENROUTER_API_BASE_URL: &str = "https://openrouter.ai/api/v1";
pub const HUGGINGFACE_API_BASE_URL: &str = "https://api-inference.huggingface.co/v1";

pub const OPENAI_DEVICE_AUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const OPENAI_DEVICE_USERCODE_URL: &str =
    "https://auth.openai.com/api/accounts/deviceauth/usercode";
pub const OPENAI_DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
pub const OPENAI_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const OPENAI_DEVICE_VERIFY_URL: &str = "https://auth.openai.com/codex/device";
pub const OPENAI_DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";

const TOKEN_REFRESH_SKEW_MS: u64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderDiscoveryKind {
    OpenAiCompatible,
    Anthropic,
    Ollama,
    OpenRouter,
    None,
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderCapabilities {
    pub discovery: ProviderDiscoveryKind,
    pub requires_base_url: bool,
    pub default_base_url: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderDescriptor {
    pub canonical_id: &'static str,
    pub capabilities: ProviderCapabilities,
}

const OPENAI_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    discovery: ProviderDiscoveryKind::OpenAiCompatible,
    requires_base_url: false,
    default_base_url: None,
};

const OPENAI_SUBSCRIPTION_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    discovery: ProviderDiscoveryKind::OpenAiCompatible,
    requires_base_url: false,
    default_base_url: Some(CODEX_CLI_BASE_URL),
};

const OPENAI_COMPATIBLE_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    discovery: ProviderDiscoveryKind::OpenAiCompatible,
    requires_base_url: true,
    default_base_url: None,
};

const OPENROUTER_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    discovery: ProviderDiscoveryKind::OpenRouter,
    requires_base_url: false,
    default_base_url: Some(OPENROUTER_API_BASE_URL),
};

const ANTHROPIC_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    discovery: ProviderDiscoveryKind::Anthropic,
    requires_base_url: false,
    default_base_url: None,
};

const OLLAMA_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    discovery: ProviderDiscoveryKind::Ollama,
    requires_base_url: true,
    default_base_url: None,
};

const HUGGINGFACE_CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    discovery: ProviderDiscoveryKind::OpenAiCompatible,
    requires_base_url: false,
    default_base_url: Some(HUGGINGFACE_API_BASE_URL),
};

pub fn provider_descriptor(provider: &str) -> Option<ProviderDescriptor> {
    match provider.trim().to_ascii_lowercase().as_str() {
        OPENAI_PROVIDER_ID => Some(ProviderDescriptor {
            canonical_id: OPENAI_PROVIDER_ID,
            capabilities: OPENAI_CAPABILITIES,
        }),
        OPENAI_SUBSCRIPTION_PROVIDER_ID => Some(ProviderDescriptor {
            canonical_id: OPENAI_SUBSCRIPTION_PROVIDER_ID,
            capabilities: OPENAI_SUBSCRIPTION_CAPABILITIES,
        }),
        OPENAI_COMPATIBLE_PROVIDER_ID | "openai_compatible" | "openai-compatible-hosted" => {
            Some(ProviderDescriptor {
                canonical_id: OPENAI_COMPATIBLE_PROVIDER_ID,
                capabilities: OPENAI_COMPATIBLE_CAPABILITIES,
            })
        }
        OPENROUTER_PROVIDER_ID => Some(ProviderDescriptor {
            canonical_id: OPENROUTER_PROVIDER_ID,
            capabilities: OPENROUTER_CAPABILITIES,
        }),
        ANTHROPIC_PROVIDER_ID => Some(ProviderDescriptor {
            canonical_id: ANTHROPIC_PROVIDER_ID,
            capabilities: ANTHROPIC_CAPABILITIES,
        }),
        OLLAMA_PROVIDER_ID => Some(ProviderDescriptor {
            canonical_id: OLLAMA_PROVIDER_ID,
            capabilities: OLLAMA_CAPABILITIES,
        }),
        HUGGINGFACE_PROVIDER_ID | "hf" | "hugging_face" | "hugging-face" => {
            Some(ProviderDescriptor {
                canonical_id: HUGGINGFACE_PROVIDER_ID,
                capabilities: HUGGINGFACE_CAPABILITIES,
            })
        }
        _ => None,
    }
}

pub fn canonical_provider_id(provider: &str) -> Option<&'static str> {
    provider_descriptor(provider).map(|descriptor| descriptor.canonical_id)
}

pub fn provider_allows_model_discovery(provider: &str) -> bool {
    provider_descriptor(provider)
        .map(|descriptor| descriptor.capabilities.discovery != ProviderDiscoveryKind::None)
        .unwrap_or(false)
}

pub fn is_openrouter_base_url(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(|host| host.to_ascii_lowercase()))
        .is_some_and(|host| host == "openrouter.ai" || host.ends_with(".openrouter.ai"))
}

pub fn is_huggingface_base_url(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(|host| host.to_ascii_lowercase()))
        .is_some_and(|host| {
            host == "api-inference.huggingface.co" || host.ends_with(".huggingface.co")
        })
}

pub fn is_codex_cli_base_url(url: &str) -> bool {
    url.trim().eq_ignore_ascii_case(CODEX_CLI_BASE_URL)
}

pub fn effective_openai_base_url(base_url: Option<&str>) -> &str {
    match base_url {
        Some(url) if is_codex_cli_base_url(url) => OPENAI_CODEX_API_BASE_URL,
        Some(url) => url,
        None => OPENAI_API_BASE_URL,
    }
}

pub fn openai_provider_label(base_url: Option<&str>) -> &'static str {
    match base_url {
        Some(url) if is_codex_cli_base_url(url) => OPENAI_SUBSCRIPTION_PROVIDER_ID,
        Some(url) if is_openrouter_base_url(url) => OPENROUTER_PROVIDER_ID,
        Some(url) if is_huggingface_base_url(url) => HUGGINGFACE_PROVIDER_ID,
        Some(_) => OPENAI_COMPATIBLE_PROVIDER_ID,
        None => OPENAI_PROVIDER_ID,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptCacheCapability {
    None,
    OpenAiAutomatic,
    OpenAiExplicitKey,
    AnthropicCacheControl,
    OpenRouterAnthropicCacheControl,
    OpenRouterProviderSpecific,
}

pub fn prompt_cache_capability_for_openai_request(
    provider_label: &'static str,
    is_openrouter: bool,
    model: &str,
) -> PromptCacheCapability {
    match provider_label {
        OPENAI_PROVIDER_ID | OPENAI_SUBSCRIPTION_PROVIDER_ID => {
            PromptCacheCapability::OpenAiExplicitKey
        }
        OPENROUTER_PROVIDER_ID if is_openrouter => {
            let route_provider = model
                .split('/')
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_ascii_lowercase())
                .unwrap_or_default();
            let route_discovery = provider_descriptor(route_provider.as_str())
                .map(|descriptor| descriptor.capabilities.discovery);
            if matches!(route_discovery, Some(ProviderDiscoveryKind::Anthropic)) {
                PromptCacheCapability::OpenRouterAnthropicCacheControl
            } else {
                PromptCacheCapability::OpenRouterProviderSpecific
            }
        }
        _ => PromptCacheCapability::None,
    }
}

pub fn display_openai_base_url(base_url: Option<&String>) -> Option<String> {
    match base_url {
        Some(url) if is_codex_cli_base_url(url) => None,
        Some(url) => Some(url.clone()),
        None => None,
    }
}

pub fn normalize_openai_base_url(
    provider: &str,
    base_url: Option<String>,
) -> std::result::Result<Option<String>, String> {
    let normalized = base_url.and_then(|url| {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let Some(descriptor) = provider_descriptor(provider) else {
        return Err(format!("Unknown provider: {}", provider));
    };

    match descriptor.canonical_id {
        OPENAI_SUBSCRIPTION_PROVIDER_ID => Ok(Some(CODEX_CLI_BASE_URL.to_string())),
        OPENROUTER_PROVIDER_ID => Ok(Some(
            normalized.unwrap_or_else(|| OPENROUTER_API_BASE_URL.to_string()),
        )),
        OPENAI_COMPATIBLE_PROVIDER_ID => {
            if normalized.is_none() && descriptor.capabilities.requires_base_url {
                Err("Base URL is required for OpenAI-Compatible providers".to_string())
            } else {
                Ok(normalized)
            }
        }
        OPENAI_PROVIDER_ID => Ok(None),
        _ => Ok(normalized.or_else(|| {
            descriptor
                .capabilities
                .default_base_url
                .map(|value| value.to_string())
        })),
    }
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn token_is_fresh(expires_at_ms: Option<u64>) -> bool {
    expires_at_ms
        .map(|expires_at_ms| expires_at_ms > unix_now_ms().saturating_add(TOKEN_REFRESH_SKEW_MS))
        .unwrap_or(false)
}

fn access_token_missing(access_token: &str) -> bool {
    access_token.trim().is_empty()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CodexCliAuthFile {
    #[serde(default)]
    openai: Option<CodexCliOpenAiAuth>,
    #[serde(rename = "OPENAI_API_KEY", default)]
    legacy_openai_api_key: Option<String>,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CodexCliOpenAiAuth {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    access: String,
    #[serde(default)]
    refresh: String,
    #[serde(default)]
    expires: Option<u64>,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct RefreshTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

pub fn codex_auth_file_path() -> Option<PathBuf> {
    let home = std::env::var("HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("USERPROFILE")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })?;
    Some(PathBuf::from(home).join(".codex").join("auth.json"))
}

fn load_codex_auth_file(path: &Path) -> Result<Option<CodexCliAuthFile>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(anyhow!(
                "Failed to read Codex auth file at {}: {}",
                path.display(),
                error
            ));
        }
    };

    let parsed: CodexCliAuthFile = serde_json::from_str(&raw).with_context(|| {
        format!(
            "Failed to parse Codex auth file at {} as JSON",
            path.display()
        )
    })?;
    Ok(Some(parsed))
}

fn write_codex_auth_file(path: &Path, auth: &CodexCliAuthFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Codex auth directory at {}",
                parent.display()
            )
        })?;
    }

    let payload = serde_json::to_string_pretty(auth)
        .context("Failed to serialize refreshed Codex auth state")?;
    crate::crypto::atomic_write_file(path, payload.as_bytes())
        .with_context(|| format!("Failed to write Codex auth file at {}", path.display()))
}

pub fn persist_codex_cli_oauth_tokens(
    access_token: &str,
    refresh_token: &str,
    expires_at_ms: u64,
) -> Result<()> {
    let Some(path) = codex_auth_file_path() else {
        return Ok(());
    };
    let mut auth_file = load_codex_auth_file(&path)?.unwrap_or_default();
    let mut openai_auth = auth_file.openai.take().unwrap_or_default();
    if openai_auth.r#type.trim().is_empty() {
        openai_auth.r#type = "oauth".to_string();
    }
    openai_auth.access = access_token.trim().to_string();
    if !refresh_token.trim().is_empty() {
        openai_auth.refresh = refresh_token.trim().to_string();
    }
    openai_auth.expires = Some(expires_at_ms);
    auth_file.openai = Some(openai_auth);
    write_codex_auth_file(&path, &auth_file)
}

async fn refresh_codex_cli_auth_token(
    client: &reqwest::Client,
    refresh_token: &str,
) -> Result<RefreshTokenResponse> {
    let response = client
        .post(OPENAI_OAUTH_TOKEN_URL)
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "client_id": OPENAI_DEVICE_AUTH_CLIENT_ID,
            "refresh_token": refresh_token,
        }))
        .send()
        .await
        .context("Failed to refresh OpenAI Subscription token")?;

    if !response.status().is_success() {
        let status = response.status();
        return Err(anyhow!(
            "OpenAI Subscription token refresh failed ({})",
            status
        ));
    }

    let refreshed: RefreshTokenResponse = response
        .json()
        .await
        .context("Failed to parse OpenAI Subscription refresh response")?;
    if refreshed.access_token.trim().is_empty() {
        return Err(anyhow!(
            "OpenAI Subscription refresh response did not include an access token"
        ));
    }
    Ok(refreshed)
}

pub async fn resolve_codex_cli_api_key(
    client: &reqwest::Client,
    force_refresh: bool,
) -> Result<Option<String>> {
    let Some(path) = codex_auth_file_path() else {
        return Ok(None);
    };
    let Some(mut auth_file) = load_codex_auth_file(&path)? else {
        return Ok(None);
    };

    if let Some(openai_auth) = auth_file.openai.as_mut() {
        if !force_refresh
            && !access_token_missing(&openai_auth.access)
            && token_is_fresh(openai_auth.expires)
        {
            return Ok(Some(openai_auth.access.trim().to_string()));
        }

        if !openai_auth.refresh.trim().is_empty() {
            let refreshed =
                refresh_codex_cli_auth_token(client, openai_auth.refresh.trim()).await?;
            openai_auth.access = refreshed.access_token.trim().to_string();
            if let Some(refresh_token) = refreshed.refresh_token {
                let trimmed = refresh_token.trim();
                if !trimmed.is_empty() {
                    openai_auth.refresh = trimmed.to_string();
                }
            }
            openai_auth.expires =
                Some(unix_now_ms().saturating_add(refreshed.expires_in.unwrap_or(3600) * 1000));
            if openai_auth.r#type.trim().is_empty() {
                openai_auth.r#type = "oauth".to_string();
            }
            let resolved_access = openai_auth.access.clone();
            write_codex_auth_file(&path, &auth_file)?;
            if !access_token_missing(&resolved_access) {
                return Ok(Some(resolved_access));
            }
        } else if !force_refresh
            && !access_token_missing(&openai_auth.access)
            && openai_auth.expires.is_none()
        {
            return Ok(Some(openai_auth.access.trim().to_string()));
        }
    }

    Ok(auth_file
        .legacy_openai_api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string()))
}

pub async fn force_refresh_codex_cli_api_key(client: &reqwest::Client) -> Result<Option<String>> {
    resolve_codex_cli_api_key(client, true).await
}

#[derive(Debug, Clone)]
pub struct ResolvedOpenAiRequestConfig {
    pub api_key: String,
    pub base_url: String,
    pub provider_label: &'static str,
    pub is_openrouter: bool,
    pub uses_codex_cli_oauth: bool,
    pub prompt_cache_capability: PromptCacheCapability,
}

pub async fn resolve_openai_request_config(
    client: &reqwest::Client,
    configured_api_key: &str,
    base_url: Option<&str>,
    model: &str,
) -> Result<ResolvedOpenAiRequestConfig> {
    let provider_label = openai_provider_label(base_url);
    let is_openrouter = base_url.is_some_and(is_openrouter_base_url);
    let uses_codex_cli_oauth = base_url.is_some_and(is_codex_cli_base_url);
    let api_key = if uses_codex_cli_oauth {
        resolve_codex_cli_api_key(client, false)
            .await?
            .ok_or_else(|| {
                anyhow!(
                    "OpenAI Subscription is not connected yet. Click 'Connect via Browser' and complete OAuth first."
                )
            })?
    } else {
        configured_api_key.trim().to_string()
    };

    if api_key.is_empty()
        && matches!(
            provider_label,
            OPENAI_PROVIDER_ID | OPENAI_SUBSCRIPTION_PROVIDER_ID | OPENROUTER_PROVIDER_ID
        )
    {
        return Err(anyhow!("{} API key is missing", provider_label));
    }

    Ok(ResolvedOpenAiRequestConfig {
        api_key,
        base_url: effective_openai_base_url(base_url)
            .trim_end_matches('/')
            .to_string(),
        provider_label,
        is_openrouter,
        uses_codex_cli_oauth,
        prompt_cache_capability: prompt_cache_capability_for_openai_request(
            provider_label,
            is_openrouter,
            model,
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CODEX_CLI_BASE_URL, HUGGINGFACE_API_BASE_URL, OPENAI_PROVIDER_ID, OPENROUTER_API_BASE_URL,
        OPENROUTER_PROVIDER_ID, PromptCacheCapability, normalize_openai_base_url,
        openai_provider_label, prompt_cache_capability_for_openai_request,
    };

    #[test]
    fn openai_label_detects_openrouter() {
        assert_eq!(
            openai_provider_label(Some("https://openrouter.ai/api/v1")),
            "openrouter"
        );
    }

    #[test]
    fn normalize_openrouter_supplies_default_base_url() {
        assert_eq!(
            normalize_openai_base_url("openrouter", None).unwrap(),
            Some(OPENROUTER_API_BASE_URL.to_string())
        );
    }

    #[test]
    fn prompt_cache_capability_is_resolved_from_provider_model_tuple() {
        assert_eq!(
            prompt_cache_capability_for_openai_request(OPENAI_PROVIDER_ID, false, "gpt-5"),
            PromptCacheCapability::OpenAiExplicitKey
        );
        assert_eq!(
            prompt_cache_capability_for_openai_request(
                OPENROUTER_PROVIDER_ID,
                true,
                "anthropic/claude-sonnet-4.6"
            ),
            PromptCacheCapability::OpenRouterAnthropicCacheControl
        );
        assert_eq!(
            prompt_cache_capability_for_openai_request(
                OPENROUTER_PROVIDER_ID,
                true,
                "deepseek/deepseek-v4-pro"
            ),
            PromptCacheCapability::OpenRouterProviderSpecific
        );
    }

    #[test]
    fn huggingface_label_detects_default_base_url() {
        assert_eq!(
            openai_provider_label(Some(HUGGINGFACE_API_BASE_URL)),
            "huggingface"
        );
        assert_eq!(
            normalize_openai_base_url("huggingface", None).unwrap(),
            Some(HUGGINGFACE_API_BASE_URL.to_string())
        );
    }
}
