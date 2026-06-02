//! Social Analytics Integration
//!
//! Provides aggregated social/content performance summaries from available sources.

use super::{Capability, Integration, IntegrationStatus};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use std::path::{Path, PathBuf};

pub struct SocialAnalyticsConnector {
    http: reqwest::Client,
    config_dir: PathBuf,
}

impl SocialAnalyticsConnector {
    pub fn new_with_config_dir(config_dir: PathBuf) -> Self {
        Self {
            http: crate::core::net::default_outgoing_http_client(),
            config_dir,
        }
    }

    pub fn new() -> Self {
        let config_dir = crate::branding::project_dirs()
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Self::new_with_config_dir(config_dir)
    }

    fn load_secret(config_dir: &Path, env_key: &str, secret_key: &str) -> Option<String> {
        if let Ok(value) = std::env::var(env_key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        match crate::core::config::SecureConfigManager::new(config_dir) {
            Ok(manager) => manager.get_custom_secret(secret_key).ok().flatten(),
            Err(_) => None,
        }
    }

    fn twitter_token(&self) -> Option<String> {
        Self::load_secret(
            &self.config_dir,
            "SOCIAL_TWITTER_BEARER_TOKEN",
            "social_twitter_bearer_token",
        )
        .or_else(|| {
            Self::load_secret(
                &self.config_dir,
                "TWITTER_BEARER_TOKEN",
                "twitter_bearer_token",
            )
        })
    }

    fn ga4_token(&self) -> Option<String> {
        Self::load_secret(
            &self.config_dir,
            "SOCIAL_GA4_ACCESS_TOKEN",
            "social_ga4_access_token",
        )
        .or_else(|| Self::load_secret(&self.config_dir, "GA4_ACCESS_TOKEN", "ga4_access_token"))
    }

    fn ga4_property_id(&self) -> Option<String> {
        Self::load_secret(
            &self.config_dir,
            "SOCIAL_GA4_PROPERTY_ID",
            "social_ga4_property_id",
        )
        .or_else(|| Self::load_secret(&self.config_dir, "GA4_PROPERTY_ID", "ga4_property_id"))
    }

    async fn twitter_summary(
        &self,
        days: i64,
        post_limit: u64,
    ) -> Result<Option<serde_json::Value>> {
        let token = match self.twitter_token() {
            Some(t) => t,
            None => return Ok(None),
        };

        let me_resp = self
            .http
            .get("https://api.twitter.com/2/users/me?user.fields=id,username,name")
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?;
        if !me_resp.status().is_success() {
            let status = me_resp.status();
            let body = me_resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Twitter profile lookup failed ({}): {}",
                status,
                body
            ));
        }
        let me_json: serde_json::Value = me_resp.json().await?;
        let user = me_json
            .get("data")
            .cloned()
            .unwrap_or(serde_json::json!({}));
        let user_id = user
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Twitter profile missing user id"))?;

        let tweets_resp = self
            .http
            .get(format!(
                "https://api.twitter.com/2/users/{}/tweets?tweet.fields=created_at,public_metrics&max_results={}",
                user_id,
                post_limit.min(100)
            ))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?;
        if !tweets_resp.status().is_success() {
            let status = tweets_resp.status();
            let body = tweets_resp.text().await.unwrap_or_default();
            return Err(anyhow!("Twitter tweet fetch failed ({}): {}", status, body));
        }
        let tweets_json: serde_json::Value = tweets_resp.json().await?;
        let tweets = tweets_json
            .get("data")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let cutoff = Utc::now() - Duration::days(days.max(1));
        let mut posts = 0_u64;
        let mut likes = 0_u64;
        let mut replies = 0_u64;
        let mut reposts = 0_u64;
        let mut quotes = 0_u64;
        let mut impressions = 0_u64;

        for tweet in &tweets {
            let created_at = tweet
                .get("created_at")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let include = if created_at.is_empty() {
                true
            } else {
                DateTime::parse_from_rfc3339(created_at)
                    .map(|dt| dt.with_timezone(&Utc) >= cutoff)
                    .unwrap_or(true)
            };
            if !include {
                continue;
            }
            posts += 1;
            let metrics = tweet
                .get("public_metrics")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            likes += metrics
                .get("like_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            replies += metrics
                .get("reply_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            reposts += metrics
                .get("retweet_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            quotes += metrics
                .get("quote_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            impressions += metrics
                .get("impression_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
        }

        let engagements = likes + replies + reposts + quotes;
        let engagement_rate = if impressions > 0 {
            Some((engagements as f64) / (impressions as f64))
        } else {
            None
        };

        Ok(Some(serde_json::json!({
            "platform": "twitter",
            "account": {
                "id": user.get("id"),
                "username": user.get("username"),
                "name": user.get("name")
            },
            "window_days": days.max(1),
            "posts": posts,
            "engagements": engagements,
            "impressions": impressions,
            "engagement_rate": engagement_rate,
            "breakdown": {
                "likes": likes,
                "replies": replies,
                "reposts": reposts,
                "quotes": quotes
            }
        })))
    }

    async fn ga4_summary(&self, days: i64) -> Result<Option<serde_json::Value>> {
        let token = match self.ga4_token() {
            Some(t) => t,
            None => return Ok(None),
        };
        let property_id = match self.ga4_property_id() {
            Some(id) => id,
            None => return Ok(None),
        };

        let payload = serde_json::json!({
            "dimensions": [{"name": "date"}],
            "metrics": [{"name": "sessions"}, {"name": "activeUsers"}, {"name": "screenPageViews"}],
            "dateRanges": [{
                "startDate": format!("{}daysAgo", days.max(1)),
                "endDate": "today"
            }],
            "limit": "1000"
        });

        let url = format!(
            "https://analyticsdata.googleapis.com/v1beta/properties/{}:runReport",
            property_id
        );
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&payload)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("GA4 summary failed ({}): {}", status, body));
        }
        let body: serde_json::Value = resp.json().await?;

        let mut sessions_total = 0_u64;
        let mut active_users_total = 0_u64;
        let mut pageviews_total = 0_u64;
        if let Some(rows) = body.get("rows").and_then(|v| v.as_array()) {
            for row in rows {
                let values = row
                    .get("metricValues")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                sessions_total += values
                    .first()
                    .and_then(|v| v.get("value"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
                active_users_total += values
                    .get(1)
                    .and_then(|v| v.get("value"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
                pageviews_total += values
                    .get(2)
                    .and_then(|v| v.get("value"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
            }
        }

        Ok(Some(serde_json::json!({
            "platform": "ga4",
            "property_id": property_id,
            "window_days": days.max(1),
            "sessions": sessions_total,
            "active_users": active_users_total,
            "pageviews": pageviews_total
        })))
    }
}

#[async_trait]
impl Integration for SocialAnalyticsConnector {
    fn id(&self) -> &str {
        "social_analytics"
    }

    fn name(&self) -> &str {
        "Social Analytics"
    }

    fn description(&self) -> &str {
        "Cross-source social and content analytics rollups (Twitter + GA4 when configured)"
    }

    fn icon(&self) -> &str {
        "social"
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Read, Capability::Search]
    }

    async fn status(&self) -> IntegrationStatus {
        if self.twitter_token().is_some() || self.ga4_token().is_some() {
            IntegrationStatus::Connected
        } else {
            IntegrationStatus::NotConfigured
        }
    }

    async fn execute(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value> {
        match action {
            "summary" => {
                let days = params.get("days").and_then(|v| v.as_i64()).unwrap_or(7);
                let post_limit = params
                    .get("post_limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(100);
                let include_twitter = params
                    .get("include_twitter")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let include_ga4 = params
                    .get("include_ga4")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);

                let mut reports: Vec<serde_json::Value> = Vec::new();
                let mut errors: Vec<String> = Vec::new();

                if include_twitter {
                    match self.twitter_summary(days, post_limit).await {
                        Ok(Some(report)) => reports.push(report),
                        Ok(None) => {}
                        Err(e) => errors.push(format!("twitter: {}", e)),
                    }
                }
                if include_ga4 {
                    match self.ga4_summary(days).await {
                        Ok(Some(report)) => reports.push(report),
                        Ok(None) => {}
                        Err(e) => errors.push(format!("ga4: {}", e)),
                    }
                }

                if reports.is_empty() && errors.is_empty() {
                    return Err(anyhow!(
                        "No analytics sources configured. Configure Twitter and/or GA4 credentials first."
                    ));
                }

                Ok(serde_json::json!({
                    "window_days": days.max(1),
                    "sources": reports,
                    "errors": errors
                }))
            }
            _ => Err(anyhow!("Unknown Social Analytics action: {}", action)),
        }
    }
}

impl Default for SocialAnalyticsConnector {
    fn default() -> Self {
        Self::new()
    }
}
