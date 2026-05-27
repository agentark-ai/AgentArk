use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const SKILL_MARKETPLACES_KEY: &str = "skills:marketplaces:v1";
const MARKETPLACE_DEFAULT_FETCH_TIMEOUT_SECS: u64 = 60;
const MARKETPLACE_DEFAULT_MAX_BYTES: usize = 2 * 1024 * 1024;
const MARKETPLACE_MAX_REDIRECTS: usize = 3;
const MARKETPLACE_MESSAGE_MAX_CHARS: usize = 500;

fn marketplace_env_usize(name: &str, default: usize, min: usize, max: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}

fn marketplace_env_u64(name: &str, default: u64, min: u64, max: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}

fn marketplace_fetch_timeout_secs() -> u64 {
    marketplace_env_u64(
        "AGENTARK_SKILL_MARKETPLACE_FETCH_TIMEOUT_SECS",
        MARKETPLACE_DEFAULT_FETCH_TIMEOUT_SECS,
        5,
        600,
    )
}

fn marketplace_max_bytes() -> usize {
    marketplace_env_usize(
        "AGENTARK_SKILL_MARKETPLACE_MAX_BYTES",
        MARKETPLACE_DEFAULT_MAX_BYTES,
        64 * 1024,
        32 * 1024 * 1024,
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillMarketplaceInstaller {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub install_url: String,
    #[serde(default)]
    pub source_url: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub policy: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMarketplace {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub installers: Vec<SkillMarketplaceInstaller>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl Default for SkillMarketplace {
    fn default() -> Self {
        let now = now_rfc3339();
        Self {
            id: String::new(),
            name: String::new(),
            url: String::new(),
            enabled: true,
            installers: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
            last_synced_at: None,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SkillMarketplaceUpsertRequest {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    pub url: String,
    #[serde(default)]
    pub enabled: Option<bool>,
}

fn default_true() -> bool {
    true
}

pub async fn load_marketplaces(storage: &crate::storage::Storage) -> Result<Vec<SkillMarketplace>> {
    let Some(bytes) = storage.get_encrypted(SKILL_MARKETPLACES_KEY).await? else {
        return Ok(Vec::new());
    };
    serde_json::from_slice::<Vec<SkillMarketplace>>(&bytes)
        .context("failed to decode skill marketplaces")
}

pub async fn upsert_marketplace(
    storage: &crate::storage::Storage,
    existing_id: Option<&str>,
    request: SkillMarketplaceUpsertRequest,
) -> Result<(String, SkillMarketplace)> {
    let url = request.url.trim().to_string();
    if url.is_empty() {
        anyhow::bail!("Marketplace URL is required");
    }
    crate::core::net::validate_public_https_url(&url)
        .await
        .map_err(|error| anyhow!("Invalid marketplace URL: {}", error))?;

    let mut marketplaces = load_marketplaces(storage).await?;
    let now = now_rfc3339();
    let mut marketplace = if let Some(id) = existing_id {
        let index = marketplaces
            .iter()
            .position(|item| item.id == id)
            .ok_or_else(|| anyhow!("Marketplace not found"))?;
        let mut next = marketplaces.remove(index);
        next.updated_at = now.clone();
        next
    } else {
        SkillMarketplace {
            id: unique_marketplace_id(
                request
                    .id
                    .as_deref()
                    .or(request.name.as_deref())
                    .unwrap_or("marketplace"),
                &marketplaces,
            ),
            created_at: now.clone(),
            updated_at: now.clone(),
            ..SkillMarketplace::default()
        }
    };

    if let Some(name) = request
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        marketplace.name = name.to_string();
    }
    marketplace.url = url;
    marketplace.enabled = request.enabled.unwrap_or(marketplace.enabled);

    let status = match refresh_marketplace_from_source(marketplace.clone()).await {
        Ok(refreshed) => {
            marketplace = refreshed;
            "ok".to_string()
        }
        Err(error) => {
            marketplace.last_error = Some(sanitize_marketplace_message(&error.to_string()));
            marketplace.updated_at = now_rfc3339();
            "warning".to_string()
        }
    };
    if marketplace.name.trim().is_empty() {
        marketplace.name = marketplace.id.clone();
    }

    marketplaces.push(marketplace.clone());
    marketplaces.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    save_marketplaces(storage, &marketplaces).await?;
    Ok((status, marketplace))
}

pub async fn refresh_marketplace(
    storage: &crate::storage::Storage,
    id: &str,
) -> Result<SkillMarketplace> {
    let mut marketplaces = load_marketplaces(storage).await?;
    let index = marketplaces
        .iter()
        .position(|item| item.id == id)
        .ok_or_else(|| anyhow!("Marketplace not found"))?;
    let existing = marketplaces.remove(index);
    let refreshed = match refresh_marketplace_from_source(existing.clone()).await {
        Ok(refreshed) => refreshed,
        Err(error) => {
            let mut failed = existing;
            failed.last_error = Some(sanitize_marketplace_message(&error.to_string()));
            failed.updated_at = now_rfc3339();
            failed
        }
    };
    marketplaces.push(refreshed.clone());
    marketplaces.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    save_marketplaces(storage, &marketplaces).await?;
    Ok(refreshed)
}

pub async fn remove_marketplace(storage: &crate::storage::Storage, id: &str) -> Result<()> {
    let mut marketplaces = load_marketplaces(storage).await?;
    let original_len = marketplaces.len();
    marketplaces.retain(|item| item.id != id);
    if marketplaces.len() == original_len {
        anyhow::bail!("Marketplace not found");
    }
    save_marketplaces(storage, &marketplaces).await
}

async fn save_marketplaces(
    storage: &crate::storage::Storage,
    marketplaces: &[SkillMarketplace],
) -> Result<()> {
    let bytes = serde_json::to_vec(marketplaces).context("failed to encode skill marketplaces")?;
    storage.set_encrypted(SKILL_MARKETPLACES_KEY, &bytes).await
}

async fn refresh_marketplace_from_source(
    mut marketplace: SkillMarketplace,
) -> Result<SkillMarketplace> {
    let body = fetch_marketplace_text(&marketplace.url).await?;
    let manifest: Value =
        serde_json::from_str(&body).context("marketplace manifest is not valid JSON")?;
    if marketplace.name.trim().is_empty() {
        marketplace.name =
            marketplace_display_name(&manifest).unwrap_or_else(|| marketplace.id.clone());
    }
    marketplace.installers = parse_marketplace_installers(&manifest);
    if marketplace.installers.is_empty() {
        anyhow::bail!("Marketplace did not include any installable skill entries");
    }
    marketplace.updated_at = now_rfc3339();
    marketplace.last_synced_at = Some(now_rfc3339());
    marketplace.last_error = None;
    Ok(marketplace)
}

async fn fetch_marketplace_text(raw_url: &str) -> Result<String> {
    let mut current = crate::core::net::validate_public_https_url(raw_url).await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(
            marketplace_fetch_timeout_secs(),
        ))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!(
            "{}/{} ({}; skill marketplace fetcher)",
            crate::branding::PRODUCT_NAME,
            env!("CARGO_PKG_VERSION"),
            crate::branding::REPOSITORY_URL
        ))
        .build()
        .context("failed to initialize marketplace HTTP client")?;
    for _ in 0..=MARKETPLACE_MAX_REDIRECTS {
        let response = client
            .get(current.clone())
            .header(
                reqwest::header::ACCEPT,
                "application/json,text/plain,*/*;q=0.8",
            )
            .send()
            .await
            .context("failed to fetch marketplace manifest")?;
        if response.status().is_success() {
            return read_response_text_limited(response, marketplace_max_bytes()).await;
        }
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| anyhow!("HTTP {} (missing Location)", response.status()))?;
            let next = current
                .join(location)
                .context("marketplace redirect URL is invalid")?;
            current = crate::core::net::validate_public_https_url(next.as_str()).await?;
            continue;
        }
        anyhow::bail!("HTTP {}", response.status());
    }
    anyhow::bail!("Too many redirects")
}

async fn read_response_text_limited(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<String> {
    if response
        .content_length()
        .is_some_and(|length| length as usize > max_bytes)
    {
        anyhow::bail!(
            "Marketplace response exceeded the maximum allowed size of {} bytes",
            max_bytes
        );
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("failed to read marketplace response")?;
        bytes.extend_from_slice(&chunk);
        if bytes.len() > max_bytes {
            anyhow::bail!(
                "Marketplace response exceeded the maximum allowed size of {} bytes",
                max_bytes
            );
        }
    }
    String::from_utf8(bytes).context("marketplace response is not valid UTF-8")
}

fn parse_marketplace_installers(manifest: &Value) -> Vec<SkillMarketplaceInstaller> {
    let entries = marketplace_entries(manifest);
    let mut installers = Vec::new();
    for (idx, entry) in entries.iter().enumerate() {
        let Value::Object(map) = entry else {
            continue;
        };
        let source = map.get("source").and_then(Value::as_object);
        let install_url = installer_install_url(entry)
            .map(|url| url.trim().to_string())
            .unwrap_or_default();
        let source_url = installer_source_url(entry).unwrap_or_else(|| install_url.clone());
        let name = string_field(entry, &["name", "title", "displayName"])
            .unwrap_or_else(|| installer_name_from_url(&install_url, idx));
        let id = normalize_marketplace_id(
            &string_field(entry, &["id", "name"])
                .unwrap_or_else(|| format!("{}-{}", name, idx + 1)),
        );
        let category = string_field(entry, &["category", "group"])
            .or_else(|| {
                source.and_then(|src| {
                    src.get("category")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
            })
            .unwrap_or_default();
        installers.push(SkillMarketplaceInstaller {
            id: if id.is_empty() {
                format!("installer-{}", idx + 1)
            } else {
                id
            },
            name,
            description: string_field(entry, &["description", "summary"]).unwrap_or_default(),
            install_url,
            source_url,
            category,
            author: string_field(entry, &["author", "publisher", "owner"]).unwrap_or_default(),
            version: string_field(entry, &["version"]).unwrap_or_default(),
            tags: string_array_field(entry, &["tags", "keywords"]),
            policy: map.get("policy").cloned().unwrap_or(Value::Null),
        });
    }
    installers
}

fn marketplace_entries(manifest: &Value) -> Vec<Value> {
    match manifest {
        Value::Array(items) => items.clone(),
        Value::Object(map) => {
            for key in ["skills", "installers", "items", "entries", "plugins"] {
                if let Some(Value::Array(items)) = map.get(key) {
                    return items.clone();
                }
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn marketplace_display_name(manifest: &Value) -> Option<String> {
    let root = manifest.as_object()?;
    root.get("interface")
        .and_then(Value::as_object)
        .and_then(|interface| interface.get("displayName"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .or_else(|| {
            root.get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        })
}

fn installer_install_url(entry: &Value) -> Option<String> {
    if let Some(url) = string_field(
        entry,
        &[
            "install_url",
            "installUrl",
            "skill_url",
            "skillUrl",
            "url",
            "source_url",
            "sourceUrl",
            "raw_url",
            "rawUrl",
        ],
    ) {
        return Some(url);
    }
    let source = entry.get("source")?;
    if let Some(url) = string_field(
        source,
        &[
            "install_url",
            "installUrl",
            "skill_url",
            "skillUrl",
            "url",
            "source_url",
            "sourceUrl",
            "raw_url",
            "rawUrl",
        ],
    ) {
        return Some(url);
    }
    github_source_url(source)
}

fn installer_source_url(entry: &Value) -> Option<String> {
    string_field(
        entry,
        &[
            "source_url",
            "sourceUrl",
            "url",
            "install_url",
            "installUrl",
            "skill_url",
            "skillUrl",
        ],
    )
    .or_else(|| {
        entry.get("source").and_then(|source| {
            string_field(
                source,
                &[
                    "source_url",
                    "sourceUrl",
                    "url",
                    "install_url",
                    "installUrl",
                    "skill_url",
                    "skillUrl",
                    "path",
                ],
            )
            .or_else(|| github_source_url(source))
        })
    })
}

fn github_source_url(source: &Value) -> Option<String> {
    let map = source.as_object()?;
    let kind = map
        .get("source")
        .or_else(|| map.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if kind != "github" {
        return None;
    }
    let repo = string_field(source, &["repo", "repository"])?;
    let path = string_field(source, &["path"]).unwrap_or_default();
    let reference =
        string_field(source, &["ref", "branch", "tag"]).unwrap_or_else(|| "main".to_string());
    let clean_path = path.trim().trim_start_matches("./").trim_start_matches('/');
    if clean_path.is_empty() {
        return Some(format!(
            "https://github.com/{}/tree/{}",
            repo.trim(),
            reference
        ));
    }
    Some(format!(
        "https://github.com/{}/tree/{}/{}",
        repo.trim(),
        reference,
        clean_path
    ))
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    let map = value.as_object()?;
    for key in keys {
        if let Some(text) = map
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            return Some(text.to_string());
        }
    }
    None
}

fn string_array_field(value: &Value, keys: &[&str]) -> Vec<String> {
    let Some(map) = value.as_object() else {
        return Vec::new();
    };
    for key in keys {
        if let Some(Value::Array(items)) = map.get(*key) {
            return items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .collect();
        }
        if let Some(text) = map.get(*key).and_then(Value::as_str) {
            return text
                .split(',')
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .collect();
        }
    }
    Vec::new()
}

fn installer_name_from_url(url: &str, idx: usize) -> String {
    let last = url
        .trim_end_matches('/')
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or("");
    let candidate = last
        .strip_suffix(".md")
        .or_else(|| last.strip_suffix(".json"))
        .unwrap_or(last)
        .trim();
    if candidate.is_empty() {
        format!("installer-{}", idx + 1)
    } else {
        candidate.to_string()
    }
}

fn unique_marketplace_id(raw: &str, existing: &[SkillMarketplace]) -> String {
    let base = normalize_marketplace_id(raw);
    let base = if base.is_empty() {
        "marketplace".to_string()
    } else {
        base
    };
    let mut candidate = base.clone();
    let mut suffix = 2;
    while existing.iter().any(|item| item.id == candidate) {
        candidate = format!("{}-{}", base, suffix);
        suffix += 1;
    }
    candidate
}

fn normalize_marketplace_id(raw: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in raw.trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').chars().take(64).collect()
}

fn sanitize_marketplace_message(value: &str) -> String {
    let redacted = crate::security::redact_secret_input(value).text;
    let trimmed = redacted.trim();
    if trimmed.chars().count() <= MARKETPLACE_MESSAGE_MAX_CHARS {
        trimmed.to_string()
    } else {
        format!(
            "{}...",
            trimmed
                .chars()
                .take(MARKETPLACE_MESSAGE_MAX_CHARS)
                .collect::<String>()
        )
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}
