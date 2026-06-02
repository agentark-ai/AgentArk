//! Google Calendar Integration
//!
//! Provides calendar access: list events, create events, find free time, set reminders.

use super::oauth::{OAuthClient, OAuthConfig, OAuthTokens, TokenStorage};
use super::{Capability, Integration, IntegrationStatus};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Google Calendar event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub id: String,
    pub summary: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub all_day: bool,
    pub attendees: Vec<String>,
    pub html_link: Option<String>,
}

/// Request to create a new event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEventRequest {
    pub summary: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub attendees: Option<Vec<String>>,
    pub reminders: Option<Vec<i32>>, // Minutes before event
}

/// Free time slot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreeSlot {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub duration_minutes: i64,
}

/// Google Calendar connector
pub struct GoogleCalendarConnector {
    oauth_config: Option<OAuthConfig>,
    tokens: Arc<RwLock<Option<OAuthTokens>>>,
    token_storage: Option<TokenStorage>,
    http: reqwest::Client,
    oauth_client: OAuthClient,
}

impl GoogleCalendarConnector {
    const SERVICE_ID: &'static str = "google_calendar";
    const API_BASE: &'static str = "https://www.googleapis.com/calendar/v3";

    pub fn new() -> Self {
        Self {
            oauth_config: None,
            tokens: Arc::new(RwLock::new(None)),
            token_storage: None,
            http: crate::core::net::default_outgoing_http_client(),
            oauth_client: OAuthClient::new(),
        }
    }

    /// Get the OAuth authorization URL
    pub fn get_auth_url(&self, state: &str) -> Result<String> {
        let config = self
            .oauth_config
            .as_ref()
            .ok_or_else(|| anyhow!("OAuth not configured"))?;
        Ok(config.auth_url(state))
    }

    /// Handle OAuth callback with authorization code
    pub async fn handle_auth_callback(&self, code: &str) -> Result<()> {
        let config = self
            .oauth_config
            .as_ref()
            .ok_or_else(|| anyhow!("OAuth not configured"))?;

        let tokens = self.oauth_client.exchange_code(config, code).await?;

        // Save tokens
        if let Some(ref storage) = self.token_storage {
            storage.save_async(Self::SERVICE_ID, &tokens).await?;
        }

        *self.tokens.write().await = Some(tokens);
        Ok(())
    }

    /// Disconnect (revoke tokens)
    pub async fn disconnect(&self) -> Result<()> {
        if let Some(ref storage) = self.token_storage {
            storage.delete_async(Self::SERVICE_ID).await?;
        }
        *self.tokens.write().await = None;
        Ok(())
    }

    /// Get a valid access token (refreshing if needed)
    async fn get_access_token(&self) -> Result<String> {
        let mut tokens_guard = self.tokens.write().await;
        let tokens = tokens_guard
            .as_mut()
            .ok_or_else(|| anyhow!("Not authenticated with Google Calendar"))?;

        // Check if token needs refresh
        if tokens.is_expired() {
            if let Some(refresh_token) = tokens.refresh_token() {
                let config = self
                    .oauth_config
                    .as_ref()
                    .ok_or_else(|| anyhow!("OAuth not configured"))?;

                let new_tokens = self
                    .oauth_client
                    .refresh_token(config, refresh_token)
                    .await?;

                // Save refreshed tokens
                if let Some(ref storage) = self.token_storage {
                    storage.save_async(Self::SERVICE_ID, &new_tokens).await?;
                }

                *tokens = new_tokens;
            } else {
                return Err(anyhow!("Token expired and no refresh token available"));
            }
        }

        Ok(tokens.access_token().to_string())
    }

    /// List events in a time range
    pub async fn list_events(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<CalendarEvent>> {
        let token = self.get_access_token().await?;

        let url = format!(
            "{}/calendars/primary/events?timeMin={}&timeMax={}&singleEvents=true&orderBy=startTime",
            Self::API_BASE,
            start.to_rfc3339(),
            end.to_rfc3339()
        );

        let response = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow!("Failed to list events: {}", error_text));
        }

        #[derive(Deserialize)]
        struct EventsResponse {
            items: Option<Vec<GoogleEvent>>,
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct GoogleEvent {
            id: String,
            summary: Option<String>,
            description: Option<String>,
            location: Option<String>,
            start: EventTime,
            end: EventTime,
            attendees: Option<Vec<Attendee>>,
            html_link: Option<String>,
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct EventTime {
            date_time: Option<String>,
            date: Option<String>,
        }

        #[derive(Deserialize)]
        struct Attendee {
            email: String,
        }

        let events_response: EventsResponse = response.json().await?;

        let events = events_response
            .items
            .unwrap_or_default()
            .into_iter()
            .filter_map(|e| {
                let (start_dt, all_day) = if let Some(dt) = e.start.date_time {
                    (
                        DateTime::parse_from_rfc3339(&dt).ok()?.with_timezone(&Utc),
                        false,
                    )
                } else if let Some(d) = e.start.date {
                    // All-day event
                    let naive = chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok()?;
                    (Utc.from_utc_datetime(&naive.and_hms_opt(0, 0, 0)?), true)
                } else {
                    return None;
                };

                let end_dt = if let Some(dt) = e.end.date_time {
                    DateTime::parse_from_rfc3339(&dt).ok()?.with_timezone(&Utc)
                } else if let Some(d) = e.end.date {
                    let naive = chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok()?;
                    Utc.from_utc_datetime(&naive.and_hms_opt(0, 0, 0)?)
                } else {
                    start_dt + Duration::hours(1)
                };

                Some(CalendarEvent {
                    id: e.id,
                    summary: e.summary.unwrap_or_else(|| "(No title)".to_string()),
                    description: e.description,
                    location: e.location,
                    start: start_dt,
                    end: end_dt,
                    all_day,
                    attendees: e
                        .attendees
                        .map(|a| a.into_iter().map(|x| x.email).collect())
                        .unwrap_or_default(),
                    html_link: e.html_link,
                })
            })
            .collect();

        Ok(events)
    }

    /// Create a new event
    pub async fn create_event(&self, request: CreateEventRequest) -> Result<CalendarEvent> {
        let token = self.get_access_token().await?;

        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct CreateEventBody {
            summary: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            description: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            location: Option<String>,
            start: EventTimeBody,
            end: EventTimeBody,
            #[serde(skip_serializing_if = "Option::is_none")]
            attendees: Option<Vec<AttendeeBody>>,
            #[serde(skip_serializing_if = "Option::is_none")]
            reminders: Option<RemindersBody>,
        }

        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct EventTimeBody {
            date_time: String,
            time_zone: String,
        }

        #[derive(Serialize)]
        struct AttendeeBody {
            email: String,
        }

        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct RemindersBody {
            use_default: bool,
            overrides: Vec<ReminderOverride>,
        }

        #[derive(Serialize)]
        struct ReminderOverride {
            method: String,
            minutes: i32,
        }

        let body = CreateEventBody {
            summary: request.summary.clone(),
            description: request.description.clone(),
            location: request.location.clone(),
            start: EventTimeBody {
                date_time: request.start.to_rfc3339(),
                time_zone: "UTC".to_string(),
            },
            end: EventTimeBody {
                date_time: request.end.to_rfc3339(),
                time_zone: "UTC".to_string(),
            },
            attendees: request.attendees.as_ref().map(|a| {
                a.iter()
                    .map(|email| AttendeeBody {
                        email: email.clone(),
                    })
                    .collect()
            }),
            reminders: request.reminders.as_ref().map(|r| RemindersBody {
                use_default: false,
                overrides: r
                    .iter()
                    .map(|&m| ReminderOverride {
                        method: "popup".to_string(),
                        minutes: m,
                    })
                    .collect(),
            }),
        };

        let url = format!("{}/calendars/primary/events", Self::API_BASE);

        let response = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow!("Failed to create event: {}", error_text));
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct CreatedEvent {
            id: String,
            html_link: Option<String>,
        }

        let created: CreatedEvent = response.json().await?;

        Ok(CalendarEvent {
            id: created.id,
            summary: request.summary,
            description: request.description,
            location: request.location,
            start: request.start,
            end: request.end,
            all_day: false,
            attendees: request.attendees.unwrap_or_default(),
            html_link: created.html_link,
        })
    }

    /// Find free time slots in a date range
    pub async fn find_free_slots(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        min_duration_minutes: i64,
    ) -> Result<Vec<FreeSlot>> {
        let events = self.list_events(start, end).await?;

        let mut slots = Vec::new();
        let mut current = start;

        for event in &events {
            if event.start > current {
                let duration = (event.start - current).num_minutes();
                if duration >= min_duration_minutes {
                    slots.push(FreeSlot {
                        start: current,
                        end: event.start,
                        duration_minutes: duration,
                    });
                }
            }
            if event.end > current {
                current = event.end;
            }
        }

        // Check remaining time after last event
        if end > current {
            let duration = (end - current).num_minutes();
            if duration >= min_duration_minutes {
                slots.push(FreeSlot {
                    start: current,
                    end,
                    duration_minutes: duration,
                });
            }
        }

        Ok(slots)
    }

    /// Get today's events
    pub async fn today(&self) -> Result<Vec<CalendarEvent>> {
        let now = Utc::now();
        let start = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .map(|dt| Utc.from_utc_datetime(&dt))
            .unwrap_or(now);
        let end = start + Duration::days(1);
        self.list_events(start, end).await
    }

    /// Get this week's events
    pub async fn this_week(&self) -> Result<Vec<CalendarEvent>> {
        let now = Utc::now();
        let start = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .map(|dt| Utc.from_utc_datetime(&dt))
            .unwrap_or(now);
        let end = start + Duration::days(7);
        self.list_events(start, end).await
    }

    async fn verify_primary_calendar_access(&self) -> Result<()> {
        let token = self.get_access_token().await?;
        let response = self
            .http
            .get(format!("{}/calendars/primary?fields=id", Self::API_BASE))
            .header("Authorization", format!("Bearer {}", token))
            .timeout(std::time::Duration::from_secs(4))
            .send()
            .await?;

        if response.status().is_success() {
            return Ok(());
        }

        Err(anyhow!("Calendar API returned {}", response.status()))
    }
}

#[async_trait]
impl Integration for GoogleCalendarConnector {
    fn id(&self) -> &str {
        Self::SERVICE_ID
    }

    fn name(&self) -> &str {
        "Google Calendar"
    }

    fn description(&self) -> &str {
        "Access your Google Calendar - view events, create appointments, find free time"
    }

    fn icon(&self) -> &str {
        "📅"
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Read, Capability::Write, Capability::Search]
    }

    async fn status(&self) -> IntegrationStatus {
        if self.oauth_config.is_none() {
            return IntegrationStatus::NotConfigured;
        }

        let tokens = self.tokens.read().await;
        if tokens.is_none() {
            return IntegrationStatus::NeedsAuth;
        }
        drop(tokens);

        match self.verify_primary_calendar_access().await {
            Ok(()) => IntegrationStatus::Connected,
            Err(error) => {
                tracing::warn!("Google Calendar connectivity check failed: {}", error);
                IntegrationStatus::Error(format!("Connection failed: {}", error))
            }
        }
    }

    async fn execute(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value> {
        match action {
            "list_events" | "today" => {
                let events = self.today().await?;
                Ok(serde_json::to_value(events)?)
            }
            "this_week" => {
                let events = self.this_week().await?;
                Ok(serde_json::to_value(events)?)
            }
            "create_event" => {
                let request: CreateEventRequest = serde_json::from_value(params.clone())?;
                let event = self.create_event(request).await?;
                Ok(serde_json::to_value(event)?)
            }
            "find_free_time" => {
                let start = params
                    .get("start")
                    .and_then(|v| v.as_str())
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(Utc::now);

                let end = params
                    .get("end")
                    .and_then(|v| v.as_str())
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|| start + Duration::days(1));

                let min_duration = params
                    .get("min_duration_minutes")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(30);

                let slots = self.find_free_slots(start, end, min_duration).await?;
                Ok(serde_json::to_value(slots)?)
            }
            "get_auth_url" => {
                let state = params
                    .get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or("calendar_auth");
                let url = self.get_auth_url(state)?;
                Ok(serde_json::json!({ "url": url }))
            }
            "auth_callback" => {
                let code = params
                    .get("code")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("Missing authorization code"))?;
                self.handle_auth_callback(code).await?;
                Ok(serde_json::json!({ "status": "connected" }))
            }
            "disconnect" => {
                self.disconnect().await?;
                Ok(serde_json::json!({ "status": "disconnected" }))
            }
            _ => Err(anyhow!("Unknown action: {}", action)),
        }
    }
}

impl Default for GoogleCalendarConnector {
    fn default() -> Self {
        Self::new()
    }
}
