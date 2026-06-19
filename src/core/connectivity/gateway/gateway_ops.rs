//! Unified gateway operations snapshot.
//!
//! This module is intentionally read-only and low-risk. It composes the
//! existing control planes into one serializable overview without adding new
//! storage formats or network behavior.
use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{
    core::{
        connectivity::{browser_profiles::BrowserProfileControlPlane, gateway},
        model::model_failover::ModelFailoverControlPlane,
        orchestration::nodes::NodeControlPlane,
        Agent, AgentConfig,
    },
    sentinel::{self, DoctorFinding, PulseEvent},
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayOpsOverview {
    pub generated_at: String,
    #[serde(default)]
    pub service_summaries: Vec<GatewayOpsServiceSummary>,
    #[serde(default)]
    pub operator_checks: Vec<GatewayOpsOperatorCheck>,
    #[serde(default)]
    pub pulse_highlights: Vec<GatewayOpsHighlight>,
    #[serde(default)]
    pub doctor_highlights: Vec<GatewayOpsHighlight>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayOpsServiceSummary {
    pub id: String,
    pub title: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_count: Option<usize>,
    #[serde(default)]
    pub attention_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayOpsOperatorCheck {
    pub id: String,
    pub title: String,
    pub passed: bool,
    pub severity: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayOpsHighlight {
    pub source: String,
    pub severity: String,
    pub title: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

pub struct GatewayOpsControlPlane;

impl GatewayOpsControlPlane {
    pub async fn overview(agent: &Agent) -> Result<GatewayOpsOverview> {
        let pulse_log = sentinel::get_pulse_log(agent).await;
        Self::overview_from_parts(&agent.storage, &agent.config, Some(pulse_log.as_slice())).await
    }

    pub async fn overview_from_parts(
        storage: &crate::storage::Storage,
        config: &AgentConfig,
        pulse_log: Option<&[PulseEvent]>,
    ) -> Result<GatewayOpsOverview> {
        let channels = gateway::load_channels(storage, config).await?;
        let routing = gateway::load_routing(storage).await?;
        let nodes = NodeControlPlane::new(storage.clone()).status().await?;
        let browsers = BrowserProfileControlPlane::list(storage).await?;
        let model_failover = ModelFailoverControlPlane::list(storage).await?;

        let pulse_events = pulse_log.unwrap_or(&[]);
        let latest_pulse = latest_pulse_event(pulse_events);
        let pulse_highlights = build_pulse_highlights(latest_pulse);
        let doctor_highlights = latest_pulse
            .map(|event| build_doctor_highlights(&event.details.doctor_findings))
            .unwrap_or_default();

        let service_summaries = vec![
            build_gateway_channels_summary(&channels.summary),
            build_gateway_routing_summary(&routing.summary),
            build_nodes_summary(&nodes.summary),
            build_browser_summary(&browsers.summary),
            build_model_failover_summary(&model_failover.summary),
            build_pulse_summary(latest_pulse, pulse_events),
        ];

        let operator_checks = vec![
            build_gateway_channels_check(&channels.summary),
            build_gateway_routing_check(&routing.summary),
            build_nodes_check(&nodes.summary),
            build_browser_check(&browsers.summary),
            build_model_failover_check(&model_failover.summary),
            build_pulse_check(latest_pulse),
        ];

        Ok(GatewayOpsOverview {
            generated_at: chrono::Utc::now().to_rfc3339(),
            service_summaries,
            operator_checks,
            pulse_highlights,
            doctor_highlights,
        })
    }
}

fn latest_pulse_event(events: &[PulseEvent]) -> Option<&PulseEvent> {
    events
        .iter()
        .max_by(|left, right| left.timestamp.cmp(&right.timestamp))
}

fn severity_from_pulse_status(status: &str) -> &'static str {
    match status.trim().to_ascii_lowercase().as_str() {
        "error" => "error",
        "alert" => "warn",
        "ok" => "info",
        _ => "info",
    }
}

fn build_gateway_channels_summary(
    summary: &crate::core::connectivity::gateway::GatewayChannelsSummary,
) -> GatewayOpsServiceSummary {
    let status = if summary.supported == 0 {
        "unknown"
    } else if summary.attention_needed > 0 {
        "warn"
    } else {
        "ok"
    };
    GatewayOpsServiceSummary {
        id: "gateway_channels".to_string(),
        title: "Gateway Channels".to_string(),
        status: status.to_string(),
        summary: Some(format!(
            "{}/{} configured, {} connected",
            summary.configured, summary.supported, summary.connected
        )),
        details: Some(format!(
            "{} channels need attention",
            summary.attention_needed
        )),
        total_count: Some(summary.supported),
        attention_count: summary.attention_needed,
    }
}

fn build_gateway_routing_summary(
    summary: &crate::core::connectivity::gateway::GatewayRoutingSummary,
) -> GatewayOpsServiceSummary {
    let attention_count = summary.rules.saturating_sub(summary.enabled_rules);
    let status = if summary.rules == 0 {
        "unknown"
    } else if attention_count > 0 {
        "warn"
    } else {
        "ok"
    };
    GatewayOpsServiceSummary {
        id: "gateway_routing".to_string(),
        title: "Gateway Routing".to_string(),
        status: status.to_string(),
        summary: Some(format!(
            "{} rules, {} broadcast groups",
            summary.rules, summary.broadcast_groups
        )),
        details: Some(format!("{} enabled rules", summary.enabled_rules)),
        total_count: Some(summary.rules),
        attention_count,
    }
}

fn build_nodes_summary(
    summary: &crate::core::orchestration::nodes::NodeSummary,
) -> GatewayOpsServiceSummary {
    let attention_count = summary.degraded + summary.offline + summary.revoked;
    let status = if summary.total == 0 {
        "unknown"
    } else if attention_count > 0 {
        "warn"
    } else {
        "ok"
    };
    GatewayOpsServiceSummary {
        id: "nodes".to_string(),
        title: "Paired Nodes".to_string(),
        status: status.to_string(),
        summary: Some(format!(
            "{} paired, {} online, {} revoked",
            summary.paired, summary.online, summary.revoked
        )),
        details: Some(format!(
            "{} degraded, {} offline",
            summary.degraded, summary.offline
        )),
        total_count: Some(summary.total),
        attention_count,
    }
}

fn build_browser_summary(
    summary: &crate::core::connectivity::browser_profiles::BrowserProfileSummary,
) -> GatewayOpsServiceSummary {
    let status = if summary.total == 0 {
        "unknown"
    } else if summary.needs_attention > 0 {
        "warn"
    } else {
        "ok"
    };
    GatewayOpsServiceSummary {
        id: "browser_profiles".to_string(),
        title: "Browser Profiles".to_string(),
        status: status.to_string(),
        summary: Some(format!(
            "{} profiles, {} logged in",
            summary.total, summary.logged_in
        )),
        details: Some(format!(
            "{} locked, {} need attention",
            summary.locked, summary.needs_attention
        )),
        total_count: Some(summary.total),
        attention_count: summary.needs_attention + summary.locked,
    }
}

fn build_model_failover_summary(
    summary: &crate::core::model::model_failover::ModelFailoverSummary,
) -> GatewayOpsServiceSummary {
    let attention_count = summary.disabled_providers + summary.cooling_providers;
    let status = if summary.providers == 0 {
        "unknown"
    } else if attention_count > 0 {
        "warn"
    } else {
        "ok"
    };
    GatewayOpsServiceSummary {
        id: "model_failover".to_string(),
        title: "Model Failover".to_string(),
        status: status.to_string(),
        summary: Some(format!(
            "{} auth profiles, {} providers, {} chains",
            summary.auth_profiles, summary.providers, summary.chains
        )),
        details: Some(format!(
            "{} disabled, {} cooling",
            summary.disabled_providers, summary.cooling_providers
        )),
        total_count: Some(summary.providers),
        attention_count,
    }
}

fn build_pulse_summary(
    latest_pulse: Option<&PulseEvent>,
    pulse_events: &[PulseEvent],
) -> GatewayOpsServiceSummary {
    let (status, summary, details, attention_count) = match latest_pulse {
        Some(event) => {
            let attention = event.overdue_tasks + event.failed_tasks;
            let status = if event.status.trim().eq_ignore_ascii_case("error") {
                "error"
            } else if event.status.trim().eq_ignore_ascii_case("alert") || attention > 0 {
                "warn"
            } else {
                "ok"
            };
            (
                status,
                Some(format!("latest pulse: {}", event.message.trim())),
                Some(format!(
                    "{} overdue tasks, {} failed tasks, doctor score {}",
                    event.overdue_tasks, event.failed_tasks, event.details.doctor_score
                )),
                attention,
            )
        }
        None => (
            "unknown",
            Some("no pulse log available".to_string()),
            None,
            0,
        ),
    };

    GatewayOpsServiceSummary {
        id: "arkpulse".to_string(),
        title: "Pulse".to_string(),
        status: status.to_string(),
        summary,
        details,
        total_count: Some(pulse_events.len()),
        attention_count,
    }
}

fn build_gateway_channels_check(
    summary: &crate::core::connectivity::gateway::GatewayChannelsSummary,
) -> GatewayOpsOperatorCheck {
    let passed = summary.supported > 0 && summary.attention_needed == 0;
    GatewayOpsOperatorCheck {
        id: "gateway_channels_ready".to_string(),
        title: "Channels are configured".to_string(),
        passed,
        severity: if passed { "info" } else { "warn" }.to_string(),
        message: if passed {
            format!(
                "{} channels are configured and connected.",
                summary.connected
            )
        } else if summary.supported == 0 {
            "No gateway channels are registered yet.".to_string()
        } else {
            format!(
                "{} channel descriptors still need attention.",
                summary.attention_needed
            )
        },
        details: Some(format!(
            "{} supported, {} configured, {} connected",
            summary.supported, summary.configured, summary.connected
        )),
    }
}

fn build_gateway_routing_check(
    summary: &crate::core::connectivity::gateway::GatewayRoutingSummary,
) -> GatewayOpsOperatorCheck {
    let passed = summary.rules > 0 && summary.enabled_rules == summary.rules;
    GatewayOpsOperatorCheck {
        id: "gateway_routing_ready".to_string(),
        title: "Routing rules are active".to_string(),
        passed,
        severity: if passed { "info" } else { "warn" }.to_string(),
        message: if summary.rules == 0 {
            "No routing rules are defined yet.".to_string()
        } else if summary.enabled_rules != summary.rules {
            format!(
                "{} of {} routing rules are enabled.",
                summary.enabled_rules, summary.rules
            )
        } else {
            "Routing rules are active.".to_string()
        },
        details: Some(format!(
            "{} broadcast groups available",
            summary.broadcast_groups
        )),
    }
}

fn build_nodes_check(
    summary: &crate::core::orchestration::nodes::NodeSummary,
) -> GatewayOpsOperatorCheck {
    let passed = summary.total > 0 && summary.offline == 0 && summary.revoked == 0;
    GatewayOpsOperatorCheck {
        id: "nodes_ready".to_string(),
        title: "Paired nodes are healthy".to_string(),
        passed,
        severity: if passed { "info" } else { "warn" }.to_string(),
        message: if summary.total == 0 {
            "No paired nodes are registered yet.".to_string()
        } else if summary.offline > 0 || summary.revoked > 0 {
            format!(
                "{} offline, {} revoked, {} degraded",
                summary.offline, summary.revoked, summary.degraded
            )
        } else {
            "Paired nodes are healthy.".to_string()
        },
        details: Some(format!(
            "{} total, {} online, {} paired",
            summary.total, summary.online, summary.paired
        )),
    }
}

fn build_browser_check(
    summary: &crate::core::connectivity::browser_profiles::BrowserProfileSummary,
) -> GatewayOpsOperatorCheck {
    let passed = summary.total > 0 && summary.needs_attention == 0;
    GatewayOpsOperatorCheck {
        id: "browser_profiles_ready".to_string(),
        title: "Browser profiles are usable".to_string(),
        passed,
        severity: if passed { "info" } else { "warn" }.to_string(),
        message: if summary.total == 0 {
            "No browser profiles are registered yet.".to_string()
        } else if summary.needs_attention > 0 {
            format!(
                "{} browser profiles need attention.",
                summary.needs_attention
            )
        } else {
            "Browser profiles are usable.".to_string()
        },
        details: Some(format!(
            "{} locked, {} logged in",
            summary.locked, summary.logged_in
        )),
    }
}

fn build_model_failover_check(
    summary: &crate::core::model::model_failover::ModelFailoverSummary,
) -> GatewayOpsOperatorCheck {
    let passed =
        summary.providers > 0 && summary.disabled_providers == 0 && summary.cooling_providers == 0;
    GatewayOpsOperatorCheck {
        id: "model_failover_ready".to_string(),
        title: "Model failover is healthy".to_string(),
        passed,
        severity: if passed { "info" } else { "warn" }.to_string(),
        message: if summary.providers == 0 {
            "No model providers are registered yet.".to_string()
        } else if summary.disabled_providers > 0 || summary.cooling_providers > 0 {
            format!(
                "{} disabled, {} cooling",
                summary.disabled_providers, summary.cooling_providers
            )
        } else {
            "Model failover is healthy.".to_string()
        },
        details: Some(format!(
            "{} auth profiles, {} fallback chains",
            summary.auth_profiles, summary.chains
        )),
    }
}

fn build_pulse_check(latest_pulse: Option<&PulseEvent>) -> GatewayOpsOperatorCheck {
    match latest_pulse {
        Some(event) => {
            let passed = event.status.trim().eq_ignore_ascii_case("ok");
            GatewayOpsOperatorCheck {
                id: "arkpulse_recent".to_string(),
                title: "Latest Pulse run".to_string(),
                passed,
                severity: severity_from_pulse_status(&event.status).to_string(),
                message: event.message.trim().to_string(),
                details: Some(format!(
                    "{} overdue, {} failed, {} health checks, doctor score {}",
                    event.overdue_tasks,
                    event.failed_tasks,
                    event.details.health_checks.len(),
                    event.details.doctor_score
                )),
            }
        }
        None => GatewayOpsOperatorCheck {
            id: "arkpulse_recent".to_string(),
            title: "Latest Pulse run".to_string(),
            passed: false,
            severity: "info".to_string(),
            message: "No Pulse events are available yet.".to_string(),
            details: None,
        },
    }
}

fn build_pulse_highlights(latest_pulse: Option<&PulseEvent>) -> Vec<GatewayOpsHighlight> {
    let Some(event) = latest_pulse else {
        return Vec::new();
    };

    let mut highlights = Vec::new();
    let severity = severity_from_pulse_status(&event.status).to_string();
    highlights.push(GatewayOpsHighlight {
        source: "pulse".to_string(),
        severity: severity.clone(),
        title: format!("Pulse {}", event.status.trim()),
        message: event.message.trim().to_string(),
        target: None,
        note: if event.summary.trim().is_empty() {
            None
        } else {
            Some(event.summary.trim().to_string())
        },
    });

    if event.overdue_tasks > 0 {
        highlights.push(GatewayOpsHighlight {
            source: "pulse".to_string(),
            severity: "warn".to_string(),
            title: "Overdue tasks detected".to_string(),
            message: format!("{} tasks are overdue.", event.overdue_tasks),
            target: None,
            note: Some(format!(
                "{} failed tasks are also recorded.",
                event.failed_tasks
            )),
        });
    }

    if event.failed_tasks > 0 {
        highlights.push(GatewayOpsHighlight {
            source: "pulse".to_string(),
            severity: "warn".to_string(),
            title: "Failed tasks detected".to_string(),
            message: format!("{} tasks failed in the latest pulse.", event.failed_tasks),
            target: None,
            note: Some(format!("Doctor score: {}", event.details.doctor_score)),
        });
    }

    highlights
}

fn build_doctor_highlights(findings: &[DoctorFinding]) -> Vec<GatewayOpsHighlight> {
    let mut ordered = findings.to_vec();
    ordered.sort_by_key(|item| std::cmp::Reverse(severity_rank(&item.severity)));

    ordered
        .into_iter()
        .filter(|finding| finding.user_actionable)
        .take(5)
        .map(|finding| GatewayOpsHighlight {
            source: "doctor".to_string(),
            severity: normalize_doctor_severity(&finding.severity).to_string(),
            title: finding.title,
            message: finding.evidence,
            target: Some(finding.target),
            note: Some(format!(
                "root cause: {}; fix: {}",
                finding.root_cause, finding.fix_command
            )),
        })
        .collect()
}

fn normalize_doctor_severity(severity: &str) -> &'static str {
    match severity.trim().to_ascii_lowercase().as_str() {
        "critical" => "error",
        "high" => "warn",
        "medium" => "warn",
        "low" => "info",
        _ => "info",
    }
}

fn severity_rank(severity: &str) -> u8 {
    match severity.trim().to_ascii_lowercase().as_str() {
        "critical" => 4,
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}
