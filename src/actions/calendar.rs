//! Google Calendar integration (list, create, find free time)
//! Mirrors the Gmail OAuth pattern for token management.

use anyhow::{anyhow, Result};
use std::path::Path;

const CALENDAR_SECRET_KEY: &str = "calendar_tokens";
const CALENDAR_API_BASE: &str = "https://www.googleapis.com/calendar/v3";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CalendarTokens {
    access_token: String,
    refresh_token: String,
    expires_at: i64,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
    #[serde(default)]
    refresh_token: Option<String>,
}

fn get_oauth_client(config_dir: &Path) -> Result<(String, String)> {
    if let (Ok(id), Ok(secret)) = (
        std::env::var("CALENDAR_CLIENT_ID"),
        std::env::var("CALENDAR_CLIENT_SECRET"),
    ) {
        return Ok((id, secret));
    }
    // Fall back to calendar-specific config, then Gmail config (same Google project)
    let manager = crate::core::config::SecureConfigManager::new(config_dir)?;
    if let Some(json_str) = manager.get_custom_secret("calendar_oauth_config")? {
        let v: serde_json::Value = serde_json::from_str(&json_str)?;
        let client_id = v
            .get("client_id")
            .and_then(|c| c.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow!("Missing client_id"))?;
        let client_secret = v
            .get("client_secret")
            .and_then(|c| c.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow!("Missing client_secret"))?;
        return Ok((client_id, client_secret));
    }
    // Try Gmail credentials (same Google Cloud project often has both APIs)
    if let Some(json_str) = manager.get_custom_secret("gmail_oauth_config")? {
        let v: serde_json::Value = serde_json::from_str(&json_str)?;
        let client_id = v
            .get("client_id")
            .and_then(|c| c.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow!("Missing client_id"))?;
        let client_secret = v
            .get("client_secret")
            .and_then(|c| c.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow!("Missing client_secret"))?;
        return Ok((client_id, client_secret));
    }
    if let Some(config) =
        crate::actions::google_workspace::load_workspace_client_config(config_dir)?
    {
        return Ok((config.client_id, config.client_secret));
    }
    Err(anyhow!(
        "Calendar OAuth credentials not configured. Connect Google Workspace or add Calendar credentials."
    ))
}

async fn load_tokens(config_dir: &Path) -> Result<CalendarTokens> {
    let manager = crate::core::config::SecureConfigManager::new(config_dir)?;
    if let Some(payload) = manager.get_custom_secret(CALENDAR_SECRET_KEY)? {
        return Ok(serde_json::from_str(&payload)?);
    }
    Err(anyhow!(
        "Calendar not connected. Go to Settings > Integrations > Calendar to connect."
    ))
}

async fn save_tokens(config_dir: &Path, tokens: &CalendarTokens) -> Result<()> {
    let manager = crate::core::config::SecureConfigManager::new(config_dir)?;
    manager.set_custom_secret(CALENDAR_SECRET_KEY, Some(serde_json::to_string(tokens)?))?;
    Ok(())
}

pub(crate) async fn ensure_access_token(config_dir: &Path) -> Result<String> {
    let mut tokens = match load_tokens(config_dir).await {
        Ok(tokens) => tokens,
        Err(_) => {
            return crate::actions::google_workspace::ensure_access_token_for_bundles(
                config_dir,
                &["calendar"],
            )
            .await;
        }
    };
    let now = chrono::Utc::now().timestamp();

    if tokens.expires_at > now + 60 {
        return Ok(tokens.access_token);
    }

    let (client_id, client_secret) = get_oauth_client(config_dir)?;
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
        return Err(anyhow!(
            "Failed to refresh calendar token: {}",
            resp.status()
        ));
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

async fn list_events_json(
    config_dir: &Path,
    start: &str,
    end: &str,
    max_results: u32,
) -> Result<serde_json::Value> {
    if crate::actions::google_workspace::gws_backend_available().await {
        let argv = vec![
            "calendar".to_string(),
            "events".to_string(),
            "list".to_string(),
            "--params".to_string(),
            serde_json::json!({
                "calendarId": "primary",
                "timeMin": start,
                "timeMax": end,
                "singleEvents": true,
                "orderBy": "startTime",
                "maxResults": max_results
            })
            .to_string(),
        ];
        match crate::actions::google_workspace::gws_json_command(config_dir, &argv, &["calendar"])
            .await
        {
            Ok(data) => return Ok(data),
            Err(error) => {
                tracing::warn!(
                    "calendar events gws path failed, falling back to direct Calendar API: {}",
                    error
                );
            }
        }
    }

    let token = ensure_access_token(config_dir).await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let url = format!(
        "{}/calendars/primary/events?timeMin={}&timeMax={}&singleEvents=true&orderBy=startTime&maxResults={}",
        CALENDAR_API_BASE,
        urlencoding::encode(start),
        urlencoding::encode(end),
        max_results,
    );
    let resp = client.get(&url).bearer_auth(token).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("Calendar API error: {}", resp.status()));
    }
    Ok(resp.json().await?)
}

/// List today's calendar events
pub async fn calendar_today(config_dir: &Path, _arguments: &serde_json::Value) -> Result<String> {
    let now = chrono::Utc::now();
    let start = now.format("%Y-%m-%dT00:00:00Z").to_string();
    let end = now.format("%Y-%m-%dT23:59:59Z").to_string();

    fetch_events(config_dir, &start, &end).await
}

/// List events in a date range
pub async fn calendar_list(config_dir: &Path, arguments: &serde_json::Value) -> Result<String> {
    let now = chrono::Utc::now();

    let start_default = now.to_rfc3339();
    let start = arguments
        .get("start")
        .and_then(|v| v.as_str())
        .unwrap_or(&start_default);
    let end_default = (now + chrono::Duration::days(7)).to_rfc3339();
    let end = arguments
        .get("end")
        .and_then(|v| v.as_str())
        .unwrap_or(&end_default);

    fetch_events(config_dir, start, end).await
}

async fn fetch_events(config_dir: &Path, start: &str, end: &str) -> Result<String> {
    let data = list_events_json(config_dir, start, end, 50).await?;
    let items = data.get("items").and_then(|v| v.as_array());

    let Some(events) = items else {
        return Ok("No events found.".to_string());
    };

    if events.is_empty() {
        return Ok("No events found in this time range.".to_string());
    }

    let mut output = format!("Found {} event(s):\n\n", events.len());
    for e in events {
        let raw_summary = e
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("(No title)");
        let start_time = e
            .get("start")
            .and_then(|s| s.get("dateTime").or_else(|| s.get("date")))
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let end_time = e
            .get("end")
            .and_then(|s| s.get("dateTime").or_else(|| s.get("date")))
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let raw_location = e.get("location").and_then(|v| v.as_str()).unwrap_or("");

        // Event titles and locations are authored by meeting organizers —
        // anyone who can send a calendar invite, including outside parties —
        // so wrap them as untrusted data.
        let summary = crate::security::sanitize_untrusted_output("calendar_event", raw_summary);
        output.push_str(&format!("- {} ({} to {})", summary, start_time, end_time));
        if !raw_location.is_empty() {
            let location =
                crate::security::sanitize_untrusted_output("calendar_location", raw_location);
            output.push_str(&format!(" @ {}", location));
        }
        output.push('\n');
    }

    Ok(output)
}

/// Create a calendar event
pub async fn calendar_create(config_dir: &Path, arguments: &serde_json::Value) -> Result<String> {
    let summary = arguments
        .get("summary")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing 'summary' for event"))?;
    let start = arguments
        .get("start")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing 'start' time (ISO format)"))?;
    let end = arguments
        .get("end")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing 'end' time (ISO format)"))?;

    let mut body = serde_json::json!({
        "summary": summary,
        "start": { "dateTime": start, "timeZone": "UTC" },
        "end": { "dateTime": end, "timeZone": "UTC" },
    });

    if let Some(desc) = arguments.get("description").and_then(|v| v.as_str()) {
        body["description"] = serde_json::json!(desc);
    }
    if let Some(loc) = arguments.get("location").and_then(|v| v.as_str()) {
        body["location"] = serde_json::json!(loc);
    }
    if let Some(attendees) = arguments.get("attendees").and_then(|v| v.as_array()) {
        let emails: Vec<serde_json::Value> = attendees
            .iter()
            .filter_map(|a| a.as_str().map(|e| serde_json::json!({"email": e})))
            .collect();
        if !emails.is_empty() {
            body["attendees"] = serde_json::json!(emails);
        }
    }

    let created: serde_json::Value = if crate::actions::google_workspace::gws_backend_available()
        .await
    {
        let argv = vec![
            "calendar".to_string(),
            "events".to_string(),
            "insert".to_string(),
            "--params".to_string(),
            serde_json::json!({ "calendarId": "primary" }).to_string(),
            "--json".to_string(),
            body.to_string(),
        ];
        match crate::actions::google_workspace::gws_json_command(config_dir, &argv, &["calendar"])
            .await
        {
            Ok(data) => data,
            Err(error) => {
                tracing::warn!(
                    "calendar create gws path failed, falling back to direct Calendar API: {}",
                    error
                );
                let token = ensure_access_token(config_dir).await?;
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(15))
                    .build()?;
                let url = format!("{}/calendars/primary/events", CALENDAR_API_BASE);
                let resp = client
                    .post(&url)
                    .bearer_auth(token)
                    .json(&body)
                    .send()
                    .await?;
                if !resp.status().is_success() {
                    let err = resp.text().await.unwrap_or_default();
                    return Err(anyhow!("Failed to create event: {}", err));
                }
                resp.json().await?
            }
        }
    } else {
        let token = ensure_access_token(config_dir).await?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        let url = format!("{}/calendars/primary/events", CALENDAR_API_BASE);
        let resp = client
            .post(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let err = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Failed to create event: {}", err));
        }
        resp.json().await?
    };
    let link = created
        .get("htmlLink")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    Ok(format!(
        "Event '{}' created ({} to {}). Link: {}",
        summary, start, end, link
    ))
}

/// Find free time slots
pub async fn calendar_free(config_dir: &Path, arguments: &serde_json::Value) -> Result<String> {
    let now = chrono::Utc::now();

    let start_default = now.to_rfc3339();
    let start = arguments
        .get("start")
        .and_then(|v| v.as_str())
        .unwrap_or(&start_default);
    let end_default = (now + chrono::Duration::days(1)).to_rfc3339();
    let end = arguments
        .get("end")
        .and_then(|v| v.as_str())
        .unwrap_or(&end_default);
    let min_duration = arguments
        .get("min_duration_minutes")
        .and_then(|v| v.as_i64())
        .unwrap_or(30);

    let data = list_events_json(config_dir, start, end, 100).await?;
    let items = data.get("items").and_then(|v| v.as_array());

    // Parse event times
    let mut busy_ranges: Vec<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> =
        Vec::new();
    if let Some(events) = items {
        for e in events {
            let s = e
                .get("start")
                .and_then(|s| s.get("dateTime"))
                .and_then(|v| v.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&chrono::Utc));
            let e = e
                .get("end")
                .and_then(|s| s.get("dateTime"))
                .and_then(|v| v.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&chrono::Utc));
            if let (Some(s), Some(e)) = (s, e) {
                busy_ranges.push((s, e));
            }
        }
    }

    // Find gaps
    let range_start = chrono::DateTime::parse_from_rfc3339(start)
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or(now);
    let range_end = chrono::DateTime::parse_from_rfc3339(end)
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or(now + chrono::Duration::days(1));

    let mut slots = Vec::new();
    let mut cursor = range_start;

    for (busy_start, busy_end) in &busy_ranges {
        if *busy_start > cursor {
            let gap_minutes = (*busy_start - cursor).num_minutes();
            if gap_minutes >= min_duration {
                slots.push(format!(
                    "{} to {} ({} min free)",
                    cursor.format("%H:%M"),
                    busy_start.format("%H:%M"),
                    gap_minutes
                ));
            }
        }
        if *busy_end > cursor {
            cursor = *busy_end;
        }
    }

    if range_end > cursor {
        let gap_minutes = (range_end - cursor).num_minutes();
        if gap_minutes >= min_duration {
            slots.push(format!(
                "{} to {} ({} min free)",
                cursor.format("%H:%M"),
                range_end.format("%H:%M"),
                gap_minutes
            ));
        }
    }

    if slots.is_empty() {
        Ok(format!(
            "No free slots of {}+ minutes found in the given range.",
            min_duration
        ))
    } else {
        Ok(format!(
            "Free time slots ({}+ min):\n{}",
            min_duration,
            slots.join("\n")
        ))
    }
}
