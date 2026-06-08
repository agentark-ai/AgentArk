use anyhow::Result;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, Instant};

const MAX_RATE_LIMIT_RETRIES: usize = 3;
const MAX_RATE_LIMIT_TOTAL_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug)]
struct ChannelRateLimitState {
    next_allowed_at: Instant,
}

impl Default for ChannelRateLimitState {
    fn default() -> Self {
        Self {
            next_allowed_at: Instant::now(),
        }
    }
}

static CHANNEL_RATE_LIMITS: Lazy<DashMap<&'static str, Arc<Mutex<ChannelRateLimitState>>>> =
    Lazy::new(DashMap::new);

fn channel_state(channel: &'static str) -> Arc<Mutex<ChannelRateLimitState>> {
    CHANNEL_RATE_LIMITS
        .entry(channel)
        .or_insert_with(|| Arc::new(Mutex::new(ChannelRateLimitState::default())))
        .clone()
}

async fn wait_for_channel_window(channel: &'static str) {
    let state = channel_state(channel);
    loop {
        let delay = {
            let state = state.lock().await;
            state.next_allowed_at.checked_duration_since(Instant::now())
        };
        match delay {
            Some(delay) if !delay.is_zero() => sleep(delay).await,
            _ => break,
        }
    }
}

async fn extend_channel_backoff(channel: &'static str, delay: Duration) {
    let state = channel_state(channel);
    let mut state = state.lock().await;
    let candidate = Instant::now() + delay;
    if candidate > state.next_allowed_at {
        state.next_allowed_at = candidate;
    }
}

fn parse_seconds_duration(raw: &str) -> Option<Duration> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds.max(1)));
    }
    if let Ok(seconds) = value.parse::<f64>() {
        if seconds.is_finite() && seconds > 0.0 {
            let millis = (seconds * 1000.0).ceil() as u64;
            return Some(Duration::from_millis(millis.max(1_000)));
        }
    }
    None
}

fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    if let Some(value) = headers.get(reqwest::header::RETRY_AFTER) {
        if let Ok(raw) = value.to_str() {
            if let Some(duration) = parse_seconds_duration(raw) {
                return Some(duration);
            }
            if let Ok(when) = DateTime::parse_from_rfc2822(raw) {
                let delay = when.with_timezone(&Utc).signed_duration_since(Utc::now());
                if delay <= chrono::Duration::zero() {
                    return Some(Duration::from_secs(1));
                }
                if let Ok(duration) = delay.to_std() {
                    return Some(duration);
                }
            }
        }
    }
    for header_name in ["x-ratelimit-reset-after", "retry-after-ms"] {
        if let Some(value) = headers.get(header_name) {
            if let Ok(raw) = value.to_str() {
                if header_name == "retry-after-ms" {
                    if let Ok(milliseconds) = raw.trim().parse::<u64>() {
                        return Some(Duration::from_millis(milliseconds.max(1_000)));
                    }
                } else if let Some(duration) = parse_seconds_duration(raw) {
                    return Some(duration);
                }
            }
        }
    }
    None
}

fn default_backoff_for_retry(retry_count: usize) -> Duration {
    match retry_count {
        0 | 1 => Duration::from_secs(1),
        2 => Duration::from_secs(2),
        _ => Duration::from_secs(4),
    }
}

pub(crate) async fn send_with_bounded_retries(
    channel: &'static str,
    operation: &'static str,
    request: reqwest::RequestBuilder,
) -> Result<reqwest::Response> {
    let retry_template = request.try_clone();
    let started_at = Instant::now();
    let mut attempts = 0usize;
    let mut pending_request = Some(request);

    loop {
        wait_for_channel_window(channel).await;

        let request = if let Some(request) = pending_request.take() {
            request
        } else if let Some(template) = retry_template.as_ref().and_then(|value| value.try_clone()) {
            template
        } else {
            anyhow::bail!(
                "channel {} {} request body cannot be retried safely after rate limiting",
                channel,
                operation
            );
        };

        let response = request.send().await?;
        if response.status() != reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Ok(response);
        }

        attempts += 1;
        if attempts > MAX_RATE_LIMIT_RETRIES {
            tracing::warn!(
                channel,
                operation,
                attempts,
                "Outbound channel request exhausted bounded retries after rate limit"
            );
            return Ok(response);
        }

        let mut delay = parse_retry_after(response.headers())
            .unwrap_or_else(|| default_backoff_for_retry(attempts));
        let elapsed = started_at.elapsed();
        let remaining_budget = MAX_RATE_LIMIT_TOTAL_DELAY.saturating_sub(elapsed);
        if remaining_budget.is_zero() {
            tracing::warn!(
                channel,
                operation,
                attempts,
                "Outbound channel request exceeded total retry delay budget"
            );
            return Ok(response);
        }
        if delay > remaining_budget {
            delay = remaining_budget;
        }

        tracing::warn!(
            channel,
            operation,
            attempts,
            delay_ms = delay.as_millis() as u64,
            "Outbound channel request hit provider rate limit; retrying with backoff"
        );
        extend_channel_backoff(channel, delay).await;
        sleep(delay).await;
        pending_request = retry_template.as_ref().and_then(|value| value.try_clone());
        if pending_request.is_none() {
            return Ok(response);
        }
    }
}
