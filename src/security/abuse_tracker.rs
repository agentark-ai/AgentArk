//! Abuse-response approval loop.
//!
//! When the inbound intent classifier blocks the same source repeatedly in
//! a short window, this module (a) moves the source to `pending_approval`
//! and (b) writes a human-review request into `approval_log` so the operator
//! can explicitly resume or pause the source. There is no automatic unblock:
//! the operator makes the call. This prevents a single legitimate user who
//! trips the guard a handful of times from being silently locked out, while
//! still halting an obvious probing burst.

use anyhow::{bail, Result};
use chrono::{DateTime, Duration, Utc};
use once_cell::sync::Lazy;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::core::config::AbuseTrackerConfig;
use crate::storage::entities::{abuse_tracker_state, approval_log};

pub const APPROVAL_ACTION_NAME: &str = "security.abuse_review";
pub const APPROVAL_RULE_NAME: &str = "abuse_threshold_reached";
static ABUSE_TRACKER_STATE_LOCK: Lazy<tokio::sync::Mutex<()>> =
    Lazy::new(|| tokio::sync::Mutex::new(()));

/// Identity under which trips are aggregated. When `user_identity` is
/// known (e.g. logged-in user), trips accumulate per user across channels;
/// otherwise per channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceKey {
    pub channel_id: String,
    pub user_identity: Option<String>,
}

impl SourceKey {
    pub fn hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.channel_id.as_bytes());
        hasher.update(b"|");
        match &self.user_identity {
            Some(identity) => hasher.update(identity.as_bytes()),
            None => hasher.update(b""),
        }
        hex::encode(hasher.finalize())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum TrackerStatus {
    Normal,
    PendingApproval,
    Paused,
}

impl TrackerStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TrackerStatus::Normal => "normal",
            TrackerStatus::PendingApproval => "pending_approval",
            TrackerStatus::Paused => "paused",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "pending_approval" => TrackerStatus::PendingApproval,
            "paused" => TrackerStatus::Paused,
            _ => TrackerStatus::Normal,
        }
    }

    pub fn should_suppress_responses(&self) -> bool {
        matches!(self, TrackerStatus::PendingApproval | TrackerStatus::Paused)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TripOutcome {
    pub status: TrackerStatus,
    /// True when this trip caused a transition into `PendingApproval`; the
    /// caller should dispatch the operator-notification fan-out only on the
    /// transition edge, not on every subsequent blocked message.
    pub newly_pending: bool,
    pub trip_count_in_window: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct AbuseReview {
    pub source_key_hash: String,
    pub channel_id: String,
    pub user_identity: Option<String>,
    pub status: String,
    pub trip_count: usize,
    pub last_updated: String,
}

pub struct AbuseTracker<'a> {
    pub db: &'a DatabaseConnection,
    pub config: AbuseTrackerConfig,
}

impl<'a> AbuseTracker<'a> {
    pub fn new(db: &'a DatabaseConnection, config: AbuseTrackerConfig) -> Self {
        Self { db, config }
    }

    pub async fn current_status(&self, source: &SourceKey) -> Result<TrackerStatus> {
        let hash = source.hash();
        let existing = abuse_tracker_state::Entity::find_by_id(hash)
            .one(self.db)
            .await?;
        Ok(existing
            .map(|row| TrackerStatus::from_str(&row.status))
            .unwrap_or(TrackerStatus::Normal))
    }

    pub async fn list_reviews(&self) -> Result<Vec<AbuseReview>> {
        let rows = abuse_tracker_state::Entity::find()
            .filter(abuse_tracker_state::Column::Status.ne(TrackerStatus::Normal.as_str()))
            .order_by_desc(abuse_tracker_state::Column::LastUpdated)
            .all(self.db)
            .await?;

        Ok(rows
            .into_iter()
            .map(|row| AbuseReview {
                source_key_hash: row.source_key_hash,
                channel_id: row.channel_id,
                user_identity: row.user_identity,
                trip_count: parse_timestamps(&row.trip_timestamps_json).len(),
                status: row.status,
                last_updated: row.last_updated,
            })
            .collect())
    }

    /// Record a fresh trip for the given source and advance state if the
    /// window threshold is reached. Returns the resulting status plus a
    /// `newly_pending` edge indicator.
    pub async fn record_trip(&self, source: &SourceKey) -> Result<TripOutcome> {
        let _state_guard = ABUSE_TRACKER_STATE_LOCK.lock().await;
        let hash = source.hash();
        let now = Utc::now();
        let window = Duration::minutes(self.config.window_minutes.max(1) as i64);
        let threshold = self.config.trips_threshold.max(1);

        let existing = abuse_tracker_state::Entity::find_by_id(hash.clone())
            .one(self.db)
            .await?;
        let (mut timestamps, prior_status) = match &existing {
            Some(row) => (
                parse_timestamps(&row.trip_timestamps_json),
                TrackerStatus::from_str(&row.status),
            ),
            None => (Vec::new(), TrackerStatus::Normal),
        };

        // Slide the window forward and record the new trip.
        let cutoff = now - window;
        timestamps.retain(|t| *t >= cutoff);
        timestamps.push(now);

        let trip_count_in_window = timestamps.len() as u32;
        let mut next_status = prior_status.clone();
        let mut newly_pending = false;
        if trip_count_in_window >= threshold && prior_status == TrackerStatus::Normal {
            next_status = TrackerStatus::PendingApproval;
            newly_pending = true;
        }

        persist_state(
            self.db,
            &hash,
            source,
            &timestamps,
            &next_status,
            now,
            existing.is_some(),
        )
        .await?;

        if newly_pending {
            if let Err(error) =
                write_approval_request(self.db, source, trip_count_in_window, now).await
            {
                tracing::warn!(
                    target: "security.abuse",
                    source_hash = %hash,
                    error = %error,
                    "failed to record abuse approval request; state transition kept"
                );
            }
        }

        Ok(TripOutcome {
            status: next_status,
            newly_pending,
            trip_count_in_window,
        })
    }

    /// Operator approved: clear the window and reset to normal.
    #[allow(dead_code)]
    pub async fn approve(&self, source: &SourceKey) -> Result<()> {
        transition_status(self.db, source, TrackerStatus::Normal, true).await
    }

    /// Operator approved by source hash. This is the HTTP/UI path, where the
    /// review row itself is the durable identity.
    pub async fn approve_hash(&self, source_key_hash: &str) -> Result<()> {
        transition_hash(self.db, source_key_hash, TrackerStatus::Normal, true).await
    }

    /// Operator rejected: park the source in `Paused`. Messages from it
    /// continue to be refused until the operator explicitly clears the
    /// state.
    #[allow(dead_code)]
    pub async fn reject(&self, source: &SourceKey) -> Result<()> {
        transition_status(self.db, source, TrackerStatus::Paused, false).await
    }

    /// Operator rejected by source hash; subsequent messages stay paused.
    pub async fn reject_hash(&self, source_key_hash: &str) -> Result<()> {
        transition_hash(self.db, source_key_hash, TrackerStatus::Paused, false).await
    }
}

fn parse_timestamps(raw: &str) -> Vec<DateTime<Utc>> {
    serde_json::from_str::<Vec<String>>(raw)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| DateTime::parse_from_rfc3339(&value).ok())
        .map(|value| value.with_timezone(&Utc))
        .collect()
}

fn serialize_timestamps(values: &[DateTime<Utc>]) -> String {
    let strings: Vec<String> = values.iter().map(|t| t.to_rfc3339()).collect();
    serde_json::to_string(&strings).unwrap_or_else(|_| "[]".to_string())
}

async fn persist_state(
    db: &DatabaseConnection,
    hash: &str,
    source: &SourceKey,
    timestamps: &[DateTime<Utc>],
    status: &TrackerStatus,
    now: DateTime<Utc>,
    exists: bool,
) -> Result<()> {
    let trip_json = serialize_timestamps(timestamps);
    if exists {
        let model = abuse_tracker_state::ActiveModel {
            source_key_hash: ActiveValue::Unchanged(hash.to_string()),
            channel_id: ActiveValue::Set(source.channel_id.clone()),
            user_identity: ActiveValue::Set(source.user_identity.clone()),
            trip_timestamps_json: ActiveValue::Set(trip_json),
            status: ActiveValue::Set(status.as_str().to_string()),
            last_updated: ActiveValue::Set(now.to_rfc3339()),
        };
        model.update(db).await?;
    } else {
        let model = abuse_tracker_state::ActiveModel {
            source_key_hash: ActiveValue::Set(hash.to_string()),
            channel_id: ActiveValue::Set(source.channel_id.clone()),
            user_identity: ActiveValue::Set(source.user_identity.clone()),
            trip_timestamps_json: ActiveValue::Set(trip_json),
            status: ActiveValue::Set(status.as_str().to_string()),
            last_updated: ActiveValue::Set(now.to_rfc3339()),
        };
        model.insert(db).await?;
    }
    Ok(())
}

#[allow(dead_code)]
async fn transition_status(
    db: &DatabaseConnection,
    source: &SourceKey,
    new_status: TrackerStatus,
    clear_timestamps: bool,
) -> Result<()> {
    let hash = source.hash();
    transition_hash(db, &hash, new_status, clear_timestamps).await
}

async fn transition_hash(
    db: &DatabaseConnection,
    hash: &str,
    new_status: TrackerStatus,
    clear_timestamps: bool,
) -> Result<()> {
    let _state_guard = ABUSE_TRACKER_STATE_LOCK.lock().await;
    let now = Utc::now();
    let existing = abuse_tracker_state::Entity::find_by_id(hash.to_string())
        .one(db)
        .await?;
    let Some(existing_row) = existing.as_ref() else {
        bail!("Abuse review source not found");
    };
    let source = SourceKey {
        channel_id: existing_row.channel_id.clone(),
        user_identity: existing_row.user_identity.clone(),
    };
    let timestamps = if clear_timestamps {
        Vec::<DateTime<Utc>>::new()
    } else {
        parse_timestamps(&existing_row.trip_timestamps_json)
    };
    persist_state(
        db,
        hash,
        &source,
        &timestamps,
        &new_status,
        now,
        existing.is_some(),
    )
    .await?;
    resolve_pending_approval_requests(db, hash, &new_status, now).await
}

async fn resolve_pending_approval_requests(
    db: &DatabaseConnection,
    hash: &str,
    new_status: &TrackerStatus,
    now: DateTime<Utc>,
) -> Result<()> {
    // If there's a pending approval_log entry, resolve it so the operator
    // view reflects the new state.
    let pending = approval_log::Entity::find()
        .filter(approval_log::Column::ActionName.eq(APPROVAL_ACTION_NAME))
        .filter(approval_log::Column::RuleName.eq(APPROVAL_RULE_NAME))
        .filter(approval_log::Column::Status.eq("pending"))
        .all(db)
        .await?;
    for row in pending {
        if let Ok(args) = serde_json::from_str::<serde_json::Value>(&row.arguments) {
            if args
                .get("source_key_hash")
                .and_then(|v| v.as_str())
                .map(|value| value == hash)
                .unwrap_or(false)
            {
                let status_str = match new_status {
                    TrackerStatus::Normal => "approved",
                    TrackerStatus::Paused => "denied",
                    TrackerStatus::PendingApproval => continue,
                };
                let model = approval_log::ActiveModel {
                    id: ActiveValue::Unchanged(row.id.clone()),
                    status: ActiveValue::Set(status_str.to_string()),
                    resolved_at: ActiveValue::Set(Some(now.to_rfc3339())),
                    ..Default::default()
                };
                model.update(db).await?;
            }
        }
    }
    Ok(())
}

async fn write_approval_request(
    db: &DatabaseConnection,
    source: &SourceKey,
    trip_count: u32,
    now: DateTime<Utc>,
) -> Result<()> {
    let hash = source.hash();
    let arguments = serde_json::json!({
        "source_key_hash": hash,
        "channel_id": source.channel_id,
        "user_identity": source.user_identity,
        "trip_count": trip_count,
    })
    .to_string();
    let model = approval_log::ActiveModel {
        id: ActiveValue::Set(uuid::Uuid::new_v4().to_string()),
        action_name: ActiveValue::Set(APPROVAL_ACTION_NAME.to_string()),
        arguments: ActiveValue::Set(arguments),
        rule_name: ActiveValue::Set(APPROVAL_RULE_NAME.to_string()),
        requested_at: ActiveValue::Set(now.to_rfc3339()),
        resolved_at: ActiveValue::Set(None),
        resolved_by: ActiveValue::Set(None),
        status: ActiveValue::Set("pending".to_string()),
    };
    model.insert(db).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_hash_matches_for_equal_keys() {
        let a = SourceKey {
            channel_id: "web-ui".into(),
            user_identity: Some("alice".into()),
        };
        let b = SourceKey {
            channel_id: "web-ui".into(),
            user_identity: Some("alice".into()),
        };
        assert_eq!(a.hash(), b.hash());
    }

    #[test]
    fn source_hash_differs_for_different_identities() {
        let a = SourceKey {
            channel_id: "web-ui".into(),
            user_identity: Some("alice".into()),
        };
        let b = SourceKey {
            channel_id: "web-ui".into(),
            user_identity: Some("mallory".into()),
        };
        assert_ne!(a.hash(), b.hash());
    }

    #[test]
    fn tracker_status_mapping_is_stable() {
        assert_eq!(TrackerStatus::Normal.as_str(), "normal");
        assert_eq!(TrackerStatus::PendingApproval.as_str(), "pending_approval");
        assert_eq!(TrackerStatus::Paused.as_str(), "paused");
    }

    #[test]
    fn pending_and_paused_suppress_responses() {
        assert!(!TrackerStatus::Normal.should_suppress_responses());
        assert!(TrackerStatus::PendingApproval.should_suppress_responses());
        assert!(TrackerStatus::Paused.should_suppress_responses());
    }
}
