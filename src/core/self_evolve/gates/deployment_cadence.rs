use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

pub const STABLE_DEPLOYMENT_LEDGER_KEY: &str = "evolve_stable_deployment_ledger_v1";
pub const STABLE_DEPLOYMENT_OBSERVATION_WINDOW_HOURS: i64 = 72;

const STABLE_DEPLOYMENT_LEDGER_RETENTION_DAYS: i64 = 180;
const STABLE_DEPLOYMENT_LEDGER_MAX_ENTRIES: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StableDeploymentLedger {
    #[serde(default)]
    pub entries: Vec<StableDeploymentRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StableDeploymentRecord {
    #[serde(default)]
    pub deployed_at: String,
    #[serde(default)]
    pub surface: String,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub proposal_id: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StableDeploymentCadenceBlock {
    pub deployed_at: DateTime<Utc>,
    pub next_allowed_at: DateTime<Utc>,
    pub surface: String,
    pub action: String,
    pub proposal_id: Option<String>,
    pub version: Option<String>,
}

pub fn stable_deployment_cadence_block(
    ledger: &StableDeploymentLedger,
    now: DateTime<Utc>,
) -> Option<StableDeploymentCadenceBlock> {
    let (record, deployed_at) = ledger
        .entries
        .iter()
        .filter_map(|record| {
            parse_stable_deployment_time(&record.deployed_at)
                .map(|deployed_at| (record, deployed_at))
        })
        .max_by_key(|(_, deployed_at)| *deployed_at)?;
    let next_allowed_at = deployed_at + Duration::hours(STABLE_DEPLOYMENT_OBSERVATION_WINDOW_HOURS);
    if now >= next_allowed_at {
        return None;
    }
    Some(StableDeploymentCadenceBlock {
        deployed_at,
        next_allowed_at,
        surface: record.surface.clone(),
        action: record.action.clone(),
        proposal_id: record.proposal_id.clone(),
        version: record.version.clone(),
    })
}

pub fn stable_deployment_cadence_block_message(block: &StableDeploymentCadenceBlock) -> String {
    format!(
        "Stable deployment is paused until {}. The last stable deployment was {} for {}; use the observation window to monitor behavior and roll back if needed before deploying another production change.",
        block.next_allowed_at.to_rfc3339(),
        block.deployed_at.to_rfc3339(),
        stable_deployment_surface_label(&block.surface),
    )
}

pub async fn stable_deployment_cadence_block_for_storage(
    storage: &crate::storage::Storage,
    now: DateTime<Utc>,
) -> Result<Option<StableDeploymentCadenceBlock>> {
    let ledger = load_stable_deployment_ledger(storage).await?;
    Ok(stable_deployment_cadence_block(&ledger, now))
}

pub async fn record_stable_deployment(
    storage: &crate::storage::Storage,
    mut record: StableDeploymentRecord,
) -> Result<()> {
    normalize_stable_deployment_record(&mut record);
    if record.deployed_at.is_empty() {
        record.deployed_at = Utc::now().to_rfc3339();
    }
    let mut ledger = load_stable_deployment_ledger(storage).await?;
    ledger.entries.push(record);
    prune_stable_deployment_ledger(&mut ledger, Utc::now());
    storage
        .set(
            STABLE_DEPLOYMENT_LEDGER_KEY,
            &serde_json::to_vec(&ledger).context("serialize stable deployment ledger")?,
        )
        .await
        .context("store stable deployment ledger")?;
    Ok(())
}

async fn load_stable_deployment_ledger(
    storage: &crate::storage::Storage,
) -> Result<StableDeploymentLedger> {
    let Some(raw) = storage
        .get(STABLE_DEPLOYMENT_LEDGER_KEY)
        .await
        .context("load stable deployment ledger")?
    else {
        return Ok(StableDeploymentLedger::default());
    };
    serde_json::from_slice::<StableDeploymentLedger>(&raw).context("parse stable deployment ledger")
}

fn parse_stable_deployment_time(value: &str) -> Option<DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn normalize_stable_deployment_record(record: &mut StableDeploymentRecord) {
    record.deployed_at = record.deployed_at.trim().to_string();
    record.surface = record.surface.trim().to_string();
    record.action = record.action.trim().to_string();
    record.proposal_id = record
        .proposal_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    record.version = record
        .version
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
}

fn prune_stable_deployment_ledger(ledger: &mut StableDeploymentLedger, now: DateTime<Utc>) {
    let retention_floor = now - Duration::days(STABLE_DEPLOYMENT_LEDGER_RETENTION_DAYS);
    ledger.entries.retain(|record| {
        parse_stable_deployment_time(&record.deployed_at)
            .map(|deployed_at| deployed_at >= retention_floor)
            .unwrap_or(false)
    });
    ledger.entries.sort_by_key(|record| {
        parse_stable_deployment_time(&record.deployed_at).unwrap_or(DateTime::<Utc>::MIN_UTC)
    });
    if ledger.entries.len() > STABLE_DEPLOYMENT_LEDGER_MAX_ENTRIES {
        let retained = ledger
            .entries
            .split_off(ledger.entries.len() - STABLE_DEPLOYMENT_LEDGER_MAX_ENTRIES);
        ledger.entries = retained;
    }
}

fn stable_deployment_surface_label(surface: &str) -> &str {
    match surface {
        "routing_policy" => "routing policy",
        "tool_strategy" => "tool strategy",
        "prompt" => "primary prompt",
        "specialist_prompt" => "specialist prompt",
        "prompt_fragment" => "prompt fragments",
        _ => "an Evolve surface",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_deployment_gate_blocks_second_deploy_inside_observation_window() {
        let deployed_at = chrono::DateTime::parse_from_rfc3339("2026-05-30T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let now = deployed_at + chrono::Duration::hours(48);
        let ledger = StableDeploymentLedger {
            entries: vec![StableDeploymentRecord {
                deployed_at: deployed_at.to_rfc3339(),
                surface: "prompt".to_string(),
                action: "manual_promote".to_string(),
                proposal_id: Some("proposal-1".to_string()),
                version: Some("prompt-v2".to_string()),
            }],
        };

        let block = stable_deployment_cadence_block(&ledger, now)
            .expect("recent stable deployment should block another one");

        assert_eq!(block.surface, "prompt");
        assert_eq!(
            block.next_allowed_at,
            deployed_at + chrono::Duration::hours(STABLE_DEPLOYMENT_OBSERVATION_WINDOW_HOURS)
        );
    }

    #[test]
    fn stable_deployment_gate_allows_after_observation_window() {
        let deployed_at = chrono::DateTime::parse_from_rfc3339("2026-05-30T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let now = deployed_at
            + chrono::Duration::hours(STABLE_DEPLOYMENT_OBSERVATION_WINDOW_HOURS)
            + chrono::Duration::seconds(1);
        let ledger = StableDeploymentLedger {
            entries: vec![StableDeploymentRecord {
                deployed_at: deployed_at.to_rfc3339(),
                surface: "tool_strategy".to_string(),
                action: "auto_promote".to_string(),
                proposal_id: None,
                version: Some("routing-v2".to_string()),
            }],
        };

        assert!(stable_deployment_cadence_block(&ledger, now).is_none());
    }

    #[test]
    fn stable_deployment_gate_uses_latest_valid_deployment_across_surfaces() {
        let older = chrono::DateTime::parse_from_rfc3339("2026-05-20T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let newer = chrono::DateTime::parse_from_rfc3339("2026-05-30T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let now = newer + chrono::Duration::hours(1);
        let ledger = StableDeploymentLedger {
            entries: vec![
                StableDeploymentRecord {
                    deployed_at: older.to_rfc3339(),
                    surface: "prompt".to_string(),
                    action: "manual_promote".to_string(),
                    proposal_id: None,
                    version: None,
                },
                StableDeploymentRecord {
                    deployed_at: "not a timestamp".to_string(),
                    surface: "ignored".to_string(),
                    action: "manual_promote".to_string(),
                    proposal_id: None,
                    version: None,
                },
                StableDeploymentRecord {
                    deployed_at: newer.to_rfc3339(),
                    surface: "specialist_prompt".to_string(),
                    action: "auto_promote".to_string(),
                    proposal_id: None,
                    version: Some("specialist-v2".to_string()),
                },
            ],
        };

        let block = stable_deployment_cadence_block(&ledger, now)
            .expect("newest valid stable deployment should control cadence");

        assert_eq!(block.surface, "specialist_prompt");
        assert_eq!(block.version.as_deref(), Some("specialist-v2"));
    }
}
