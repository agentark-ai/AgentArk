//! Gmail integration (scan and reply)

use anyhow::{anyhow, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::Path;

const GMAIL_SECRET_KEY: &str = "gmail_tokens";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GMAIL_API_BASE: &str = "https://gmail.googleapis.com/gmail/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GmailTokens {
    access_token: String,
    refresh_token: String,
    expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
    #[serde(default)]
    refresh_token: Option<String>,
}

fn get_oauth_client_with_config(config_dir: &Path) -> Result<(String, String)> {
    if let (Ok(id), Ok(secret)) = (
        std::env::var("GMAIL_CLIENT_ID"),
        std::env::var("GMAIL_CLIENT_SECRET"),
    ) {
        return Ok((id, secret));
    }

    let manager = crate::core::config::SecureConfigManager::new(config_dir)?;
    if let Some(json_str) = manager.get_custom_secret("gmail_oauth_config")? {
        let v: serde_json::Value = serde_json::from_str(&json_str)?;
        let client_id = v
            .get("client_id")
            .and_then(|c| c.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow!("Missing client_id in gmail config"))?;
        let client_secret = v
            .get("client_secret")
            .and_then(|c| c.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow!("Missing client_secret in gmail config"))?;
        return Ok((client_id, client_secret));
    }

    if let Some(config) =
        crate::actions::google_workspace::load_workspace_client_config(config_dir)?
    {
        return Ok((config.client_id, config.client_secret));
    }

    Err(anyhow!(
        "Gmail OAuth credentials not configured. Connect Google Workspace or add Gmail credentials."
    ))
}

async fn load_tokens(config_dir: &Path) -> Result<GmailTokens> {
    let manager = crate::core::config::SecureConfigManager::new(config_dir)?;
    if let Some(payload) = manager.get_custom_secret(GMAIL_SECRET_KEY)? {
        let tokens: GmailTokens = serde_json::from_str(&payload)?;
        return Ok(tokens);
    }

    Err(anyhow!("Gmail tokens not found"))
}

async fn save_tokens(config_dir: &Path, tokens: &GmailTokens) -> Result<()> {
    let manager = crate::core::config::SecureConfigManager::new(config_dir)?;
    let payload = serde_json::to_string(tokens)?;
    manager.set_custom_secret(GMAIL_SECRET_KEY, Some(payload))?;
    Ok(())
}

pub(crate) async fn ensure_legacy_access_token(config_dir: &Path) -> Result<String> {
    let mut tokens = load_tokens(config_dir).await?;
    let now = chrono::Utc::now().timestamp();

    if tokens.expires_at > now + 60 {
        return Ok(tokens.access_token);
    }

    let (client_id, client_secret) = get_oauth_client_with_config(config_dir)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let params = [
        ("client_id", client_id.as_str()),
        ("client_secret", client_secret.as_str()),
        ("refresh_token", tokens.refresh_token.as_str()),
        ("grant_type", "refresh_token"),
    ];

    let resp = client.post(TOKEN_URL).form(&params).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("Failed to refresh token: {}", resp.status()));
    }

    let token_resp: TokenResponse = resp.json().await?;
    tokens.access_token = token_resp.access_token;
    tokens.expires_at = now + token_resp.expires_in;
    if let Some(refresh) = token_resp.refresh_token {
        tokens.refresh_token = refresh;
    }

    save_tokens(config_dir, &tokens).await?;
    Ok(tokens.access_token)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GmailDeliverySource {
    #[default]
    Auto,
    Gmail,
    GoogleWorkspace,
}

pub(crate) async fn ensure_access_token_for_source(
    config_dir: &Path,
    source: GmailDeliverySource,
) -> Result<String> {
    match source {
        GmailDeliverySource::Auto => match ensure_legacy_access_token(config_dir).await {
            Ok(token) => Ok(token),
            Err(_) => {
                crate::actions::google_workspace::ensure_access_token_for_bundles(
                    config_dir,
                    &["gmail"],
                )
                .await
            }
        },
        GmailDeliverySource::Gmail => ensure_legacy_access_token(config_dir).await,
        GmailDeliverySource::GoogleWorkspace => {
            crate::actions::google_workspace::ensure_access_token_for_bundles(
                config_dir,
                &["gmail"],
            )
            .await
        }
    }
}

pub(crate) async fn ensure_access_token(config_dir: &Path) -> Result<String> {
    ensure_access_token_for_source(config_dir, GmailDeliverySource::Auto).await
}

pub(crate) async fn gmail_profile_email_for_source(
    config_dir: &Path,
    source: GmailDeliverySource,
) -> Result<String> {
    let access_token = ensure_access_token_for_source(config_dir, source).await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let resp = client
        .get(format!("{}/users/me/profile", GMAIL_API_BASE))
        .bearer_auth(access_token)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("Gmail profile failed: {}", resp.status()));
    }
    #[derive(Debug, Deserialize)]
    struct ProfileResp {
        #[serde(default)]
        email_address: String,
    }
    let profile: ProfileResp = resp.json().await?;
    Ok(profile.email_address)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GmailScanMode {
    #[default]
    Auto,
    Recent,
    Search,
    Triage,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GmailScanArgs {
    #[serde(default)]
    pub mode: GmailScanMode,
    pub query: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub max_results: Option<u32>,
}

pub(crate) fn effective_scan_mode(args: &GmailScanArgs) -> GmailScanMode {
    match args.mode {
        GmailScanMode::Auto => {
            let has_query = args
                .query
                .as_deref()
                .is_some_and(|query| !query.trim().is_empty());
            if has_query || !args.labels.is_empty() {
                GmailScanMode::Search
            } else if args.max_results.is_some() {
                GmailScanMode::Recent
            } else {
                GmailScanMode::Triage
            }
        }
        explicit => explicit,
    }
}

#[derive(Debug, Deserialize)]
pub struct GmailReplyArgs {
    pub to: String,
    pub subject: String,
    pub body: String,
    pub thread_id: Option<String>,
    #[serde(default)]
    pub html_body: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub delivery_source: GmailDeliverySource,
}

#[derive(Debug, Deserialize)]
struct GmailListResponse {
    messages: Option<Vec<GmailMessageRef>>,
}

#[derive(Debug, Deserialize)]
struct GmailMessageRef {
    id: String,
}

#[derive(Debug, Deserialize)]
struct GmailFullMessage {
    id: String,
    #[serde(default, rename = "threadId")]
    thread_id: String,
    #[serde(default, rename = "labelIds")]
    label_ids: Vec<String>,
    #[serde(default)]
    snippet: String,
    #[serde(default)]
    payload: GmailPayload,
}

#[derive(Debug, Deserialize, Default)]
struct GmailPayload {
    #[serde(default)]
    headers: Vec<GmailHeader>,
}

#[derive(Debug, Deserialize)]
struct GmailHeader {
    name: String,
    value: String,
}

fn header_value(headers: &[GmailHeader], name: &str) -> String {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.clone())
        .unwrap_or_default()
}

fn render_message_summary(meta: GmailFullMessage) -> String {
    let subject = header_value(&meta.payload.headers, "Subject");
    let from = header_value(&meta.payload.headers, "From");
    let date = header_value(&meta.payload.headers, "Date");
    let labels = meta.label_ids.join(", ");
    let mut lines = vec![
        format!("- From: {}", from),
        format!("  Subject: {}", subject),
        format!("  Date: {}", date),
        format!("  Labels: {}", labels),
        format!("  Id: {}", meta.id),
        format!("  ThreadId: {}", meta.thread_id),
    ];
    if !meta.snippet.trim().is_empty() {
        // Email body previews are author-controlled; wrap them so the model
        // treats their contents as data rather than instructions.
        let wrapped =
            crate::security::sanitize_untrusted_output("email_snippet", meta.snippet.trim());
        lines.push(format!("  Snippet: {}", wrapped));
    }
    lines.join("\n")
}

async fn fetch_message_ids(
    client: &reqwest::Client,
    access_token: &str,
    query: Option<&str>,
    labels: &[String],
    max_results: u32,
) -> Result<Vec<String>> {
    let mut url = reqwest::Url::parse(&format!("{}/users/me/messages", GMAIL_API_BASE))?;
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("maxResults", &max_results.to_string());
        for label in labels {
            qp.append_pair("labelIds", label);
        }
        if let Some(q) = query {
            qp.append_pair("q", q);
        }
    }

    let resp = client.get(url).bearer_auth(access_token).send().await?;
    if !resp.status().is_success() {
        return Ok(vec![]);
    }

    let list: GmailListResponse = resp.json().await?;
    Ok(list
        .messages
        .unwrap_or_default()
        .into_iter()
        .map(|m| m.id)
        .collect())
}

async fn fetch_message_metadata(
    client: &reqwest::Client,
    access_token: &str,
    msg_id: &str,
) -> Option<GmailFullMessage> {
    let url = format!(
        "{}/users/me/messages/{}?format=metadata&metadataHeaders=Subject&metadataHeaders=From&metadataHeaders=Date",
        GMAIL_API_BASE, msg_id
    );
    let resp = client
        .get(&url)
        .bearer_auth(access_token)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json().await.ok()
}

async fn fetch_message_ids_via_gws(
    config_dir: &Path,
    query: Option<&str>,
    labels: &[String],
    max_results: u32,
) -> Result<Vec<String>> {
    let mut params = serde_json::json!({
        "userId": "me",
        "maxResults": max_results
    });
    if let Some(query) = query.filter(|value| !value.trim().is_empty()) {
        params["q"] = serde_json::Value::String(query.to_string());
    }
    if !labels.is_empty() {
        params["labelIds"] = serde_json::Value::Array(
            labels
                .iter()
                .map(|label| serde_json::Value::String(label.clone()))
                .collect(),
        );
    }
    let argv = vec![
        "gmail".to_string(),
        "users".to_string(),
        "messages".to_string(),
        "list".to_string(),
        "--params".to_string(),
        params.to_string(),
    ];
    let data =
        crate::actions::google_workspace::gws_json_command(config_dir, &argv, &["gmail"]).await?;
    let parsed: GmailListResponse = serde_json::from_value(data)
        .map_err(|error| anyhow!("Invalid gws Gmail list response: {}", error))?;
    Ok(parsed
        .messages
        .unwrap_or_default()
        .into_iter()
        .map(|message| message.id)
        .collect())
}

async fn fetch_message_metadata_via_gws(
    config_dir: &Path,
    msg_id: &str,
) -> Result<Option<GmailFullMessage>> {
    let argv = vec![
        "gmail".to_string(),
        "users".to_string(),
        "messages".to_string(),
        "get".to_string(),
        "--params".to_string(),
        serde_json::json!({
            "userId": "me",
            "id": msg_id,
            "format": "metadata",
            "metadataHeaders": ["Subject", "From", "Date"]
        })
        .to_string(),
    ];
    let data =
        crate::actions::google_workspace::gws_json_command(config_dir, &argv, &["gmail"]).await?;
    let parsed: GmailFullMessage = serde_json::from_value(data)
        .map_err(|error| anyhow!("Invalid gws Gmail metadata response: {}", error))?;
    Ok(Some(parsed))
}

async fn gmail_scan_via_gws(config_dir: &Path, args: &GmailScanArgs) -> Result<String> {
    let normalized_query = args
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let effective_mode = effective_scan_mode(args);
    let mut seen = std::collections::HashSet::new();
    let mut ordered_ids: Vec<String> = Vec::new();

    match effective_mode {
        GmailScanMode::Search => {
            let labels = if args.labels.is_empty() {
                vec!["INBOX".to_string()]
            } else {
                args.labels.clone()
            };
            let ids = fetch_message_ids_via_gws(
                config_dir,
                normalized_query,
                &labels,
                args.max_results.unwrap_or(20),
            )
            .await?;
            for id in ids {
                if seen.insert(id.clone()) {
                    ordered_ids.push(id);
                }
            }
        }
        GmailScanMode::Recent => {
            let labels = if args.labels.is_empty() {
                vec!["INBOX".to_string()]
            } else {
                args.labels.clone()
            };
            let ids = fetch_message_ids_via_gws(
                config_dir,
                None,
                &labels,
                args.max_results.unwrap_or(10),
            )
            .await?;
            for id in ids {
                if seen.insert(id.clone()) {
                    ordered_ids.push(id);
                }
            }
        }
        GmailScanMode::Triage | GmailScanMode::Auto => {
            let inbox = vec!["INBOX".to_string()];
            let (important, primary, recent, starred) = tokio::join!(
                fetch_message_ids_via_gws(config_dir, Some("is:unread is:important"), &inbox, 15),
                fetch_message_ids_via_gws(
                    config_dir,
                    Some("is:unread category:primary"),
                    &inbox,
                    15
                ),
                fetch_message_ids_via_gws(config_dir, Some("is:unread newer_than:3d"), &inbox, 20),
                fetch_message_ids_via_gws(config_dir, Some("is:starred newer_than:7d"), &inbox, 5),
            );
            for batch in [important, primary, recent, starred] {
                for id in batch.unwrap_or_default() {
                    if seen.insert(id.clone()) {
                        ordered_ids.push(id);
                    }
                }
            }
        }
    }

    if ordered_ids.is_empty() {
        return Ok("No messages found.".to_string());
    }

    let metadata_futures: Vec<_> = ordered_ids
        .iter()
        .map(|id| fetch_message_metadata_via_gws(config_dir, id))
        .collect();
    let metadata_results = futures::future::join_all(metadata_futures).await;
    let summaries = metadata_results
        .into_iter()
        .filter_map(Result::ok)
        .flatten()
        .map(render_message_summary)
        .collect::<Vec<_>>();
    if summaries.is_empty() {
        Ok("No messages found.".to_string())
    } else {
        Ok(summaries.join("\n\n"))
    }
}

pub async fn gmail_scan(config_dir: &Path, args: &serde_json::Value) -> Result<String> {
    let args: GmailScanArgs = serde_json::from_value(args.clone())
        .map_err(|e| anyhow!("Invalid Gmail scan args: {}", e))?;
    if crate::actions::google_workspace::gws_backend_available().await {
        match gmail_scan_via_gws(config_dir, &args).await {
            Ok(result) => return Ok(result),
            Err(error) => {
                tracing::warn!(
                    "gmail_scan gws path failed, falling back to direct Gmail API: {}",
                    error
                );
            }
        }
    }
    let normalized_query = args
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let effective_mode = effective_scan_mode(&args);

    let access_token = ensure_access_token(config_dir).await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let mut seen = std::collections::HashSet::new();
    let mut ordered_ids: Vec<String> = Vec::new();

    match effective_mode {
        GmailScanMode::Search => {
            let labels = if args.labels.is_empty() {
                vec!["INBOX".to_string()]
            } else {
                args.labels.clone()
            };
            let ids = fetch_message_ids(
                &client,
                &access_token,
                normalized_query,
                &labels,
                args.max_results.unwrap_or(20),
            )
            .await?;
            for id in ids {
                if seen.insert(id.clone()) {
                    ordered_ids.push(id);
                }
            }
        }
        GmailScanMode::Recent => {
            let labels = if args.labels.is_empty() {
                vec!["INBOX".to_string()]
            } else {
                args.labels.clone()
            };
            let ids = fetch_message_ids(
                &client,
                &access_token,
                None,
                &labels,
                args.max_results.unwrap_or(10),
            )
            .await?;
            for id in ids {
                if seen.insert(id.clone()) {
                    ordered_ids.push(id);
                }
            }
        }
        GmailScanMode::Triage | GmailScanMode::Auto => {
            let inbox = vec!["INBOX".to_string()];
            let (important, primary, recent, starred) = tokio::join!(
                fetch_message_ids(
                    &client,
                    &access_token,
                    Some("is:unread is:important"),
                    &inbox,
                    15
                ),
                fetch_message_ids(
                    &client,
                    &access_token,
                    Some("is:unread category:primary"),
                    &inbox,
                    15
                ),
                fetch_message_ids(
                    &client,
                    &access_token,
                    Some("is:unread newer_than:3d"),
                    &inbox,
                    20
                ),
                fetch_message_ids(
                    &client,
                    &access_token,
                    Some("is:starred newer_than:7d"),
                    &inbox,
                    5
                ),
            );

            for batch in [important, primary, recent, starred] {
                for id in batch.unwrap_or_default() {
                    if seen.insert(id.clone()) {
                        ordered_ids.push(id);
                    }
                }
            }
        }
    }

    if ordered_ids.is_empty() {
        return Ok("No messages found.".to_string());
    }

    let metadata_futures: Vec<_> = ordered_ids
        .iter()
        .map(|id| fetch_message_metadata(&client, &access_token, id))
        .collect();
    let metadata_results = futures::future::join_all(metadata_futures).await;

    let summaries = metadata_results
        .into_iter()
        .flatten()
        .map(render_message_summary)
        .collect::<Vec<_>>();

    if summaries.is_empty() {
        Ok("No messages found.".to_string())
    } else {
        Ok(summaries.join("\n\n"))
    }
}

pub async fn gmail_reply(config_dir: &Path, args: &serde_json::Value) -> Result<String> {
    let args: GmailReplyArgs = serde_json::from_value(args.clone())
        .map_err(|e| anyhow!("Invalid Gmail reply args: {}", e))?;

    let access_token = ensure_access_token_for_source(config_dir, args.delivery_source).await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()?;
    let from = match args
        .from
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(from) => from.to_string(),
        None => gmail_profile_email_for_source(config_dir, args.delivery_source).await?,
    };
    let message = crate::core::email_delivery::build_email_message(
        &from,
        &args.to,
        &args.subject,
        &args.body,
        args.html_body.as_deref(),
        None,
    )?;
    let raw_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(message.formatted());
    let mut body = serde_json::json!({
        "raw": raw_b64
    });
    if let Some(thread_id) = &args.thread_id {
        body["threadId"] = serde_json::Value::String(thread_id.clone());
    }

    let resp = client
        .post(format!("{}/users/me/messages/send", GMAIL_API_BASE))
        .bearer_auth(&access_token)
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(anyhow!("Gmail send failed: {}", resp.status()));
    }

    Ok("Reply sent successfully.".to_string())
}

#[cfg(test)]
mod tests {
    use super::{effective_scan_mode, GmailScanArgs, GmailScanMode};

    #[test]
    fn gmail_scan_auto_mode_uses_recent_when_only_max_results_is_set() {
        let args = GmailScanArgs {
            mode: GmailScanMode::Auto,
            query: None,
            labels: Vec::new(),
            max_results: Some(5),
        };
        assert_eq!(effective_scan_mode(&args), GmailScanMode::Recent);
    }

    #[test]
    fn gmail_scan_auto_mode_uses_search_when_query_or_labels_are_present() {
        let with_query = GmailScanArgs {
            mode: GmailScanMode::Auto,
            query: Some("from:alice@example.com".to_string()),
            labels: Vec::new(),
            max_results: Some(5),
        };
        assert_eq!(effective_scan_mode(&with_query), GmailScanMode::Search);

        let with_labels = GmailScanArgs {
            mode: GmailScanMode::Auto,
            query: None,
            labels: vec!["STARRED".to_string()],
            max_results: None,
        };
        assert_eq!(effective_scan_mode(&with_labels), GmailScanMode::Search);
    }

    #[test]
    fn gmail_scan_auto_mode_defaults_to_triage_without_filters() {
        let args = GmailScanArgs {
            mode: GmailScanMode::Auto,
            query: None,
            labels: Vec::new(),
            max_results: None,
        };
        assert_eq!(effective_scan_mode(&args), GmailScanMode::Triage);
    }
}
