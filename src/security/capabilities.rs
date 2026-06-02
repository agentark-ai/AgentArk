//! Shared security capability vocabulary and deterministic policy helpers.
//!
//! Free-form skill content is classified by the configured model in
//! `skill_review`. Other layers consume declared machine capabilities from
//! manifests/bindings and map them into this same vocabulary.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

use crate::security::action_guard::{
    ActionGuard, AnalysisFinding, FindingCategory, Permission, ThreatLevel,
};

pub const CAPABILITY_VOCABULARY: &[&str] = &[
    "reads-env",
    "reads-file",
    "reads-user-data",
    "reads-email",
    "reads-calendar",
    "reads-documents",
    "reads-memory",
    "reads-browser-data",
    "reads-secrets",
    "writes-file",
    "calls-network",
    "local-network",
    "sends-external",
    "writes-external",
    "publishes-public",
    "executes-shell",
    "captures-keystrokes",
    "captures-screen",
    "captures-audio",
    "uses-camera",
    "encodes-payload",
    "installs-package",
    "declares-lifecycle-hook",
    "uses-clipboard",
    "schedules-task",
    "sends-message",
    "browser-automation",
    "code-execution",
    "modifies-persistence",
    "uses-auth-profile",
    "requests-secrets",
    "unknown-high-risk",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityObservation {
    pub layer: String,
    pub entity_id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

impl CapabilityObservation {
    pub fn selector(&self) -> String {
        let kind = normalize_capability_kind(&self.kind);
        self.target
            .as_deref()
            .map(normalize_capability_target)
            .filter(|target| !target.is_empty())
            .map(|target| format!("{}:{}", kind, target))
            .unwrap_or(kind)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchedCapabilityRule {
    pub id: String,
    pub effect: String,
    pub message: String,
    pub severity: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityLayerReport {
    pub observations: Vec<CapabilityObservation>,
    pub blocked: bool,
    pub threat_level: ThreatLevel,
    pub risk_score_10: f32,
    pub risk_band: String,
    pub total_severity: u32,
    pub warnings: Vec<String>,
    pub findings: Vec<AnalysisFinding>,
    pub matched_rules: Vec<MatchedCapabilityRule>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityCorrelationEffect {
    Allow,
    RequireApproval,
    Block,
}

#[derive(Debug, Clone)]
pub struct CapabilityCorrelationDecision {
    pub effect: CapabilityCorrelationEffect,
    pub report: Option<CapabilityLayerReport>,
    pub message: Option<String>,
}

impl CapabilityCorrelationDecision {
    pub fn allow() -> Self {
        Self {
            effect: CapabilityCorrelationEffect::Allow,
            report: None,
            message: None,
        }
    }

    pub fn requires_approval(report: CapabilityLayerReport) -> Self {
        let message = report
            .warnings
            .first()
            .cloned()
            .unwrap_or_else(|| {
                "This action combines sensitive access with external delivery and requires explicit approval.".to_string()
            });
        Self {
            effect: CapabilityCorrelationEffect::RequireApproval,
            report: Some(report),
            message: Some(message),
        }
    }

    pub fn block(report: CapabilityLayerReport) -> Self {
        let message = report
            .warnings
            .first()
            .cloned()
            .unwrap_or_else(|| "Blocked by cross-layer capability policy.".to_string());
        Self {
            effect: CapabilityCorrelationEffect::Block,
            report: Some(report),
            message: Some(message),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RunCapabilityContext {
    observations: Vec<CapabilityObservation>,
}

impl RunCapabilityContext {
    pub fn observations(&self) -> &[CapabilityObservation] {
        &self.observations
    }

    pub fn extend(&mut self, observations: Vec<CapabilityObservation>) {
        let mut seen = self
            .observations
            .iter()
            .map(|observation| {
                format!(
                    "{}:{}:{}",
                    observation.layer,
                    observation.entity_id,
                    observation.selector()
                )
            })
            .collect::<BTreeSet<_>>();

        for observation in observations {
            let key = format!(
                "{}:{}:{}",
                observation.layer,
                observation.entity_id,
                observation.selector()
            );
            if seen.insert(key) {
                self.observations.push(observation);
            }
        }
    }

    pub fn retain_recent(&mut self, limit: usize) {
        if limit == 0 {
            self.observations.clear();
            return;
        }
        if self.observations.len() > limit {
            let drop_count = self.observations.len() - limit;
            self.observations.drain(0..drop_count);
        }
    }
}

#[derive(Debug, Clone)]
struct CapabilityRule {
    id: &'static str,
    effect: &'static str,
    all: &'static [&'static str],
    any: &'static [&'static str],
    message: &'static str,
    severity: u32,
}

pub fn normalize_capability_kind(raw: &str) -> String {
    raw.trim()
        .chars()
        .map(|ch| {
            if ch == '_' || ch.is_whitespace() {
                '-'
            } else {
                ch.to_ascii_lowercase()
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

pub fn normalize_capability_target(raw: &str) -> String {
    let trimmed = raw
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_end_matches('.');
    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_scheme)
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

pub fn normalize_capability_selector(raw: &str) -> String {
    let trimmed = raw.trim();
    let Some((kind, target)) = trimmed.split_once(':') else {
        return normalize_capability_kind(trimmed);
    };
    let kind = normalize_capability_kind(kind);
    let target = normalize_capability_target(target);
    if target.is_empty() {
        kind
    } else {
        format!("{}:{}", kind, target)
    }
}

pub fn canonical_capability_set() -> HashSet<String> {
    CAPABILITY_VOCABULARY
        .iter()
        .map(|item| item.to_string())
        .collect()
}

pub fn capability_severity(kind: &str) -> u32 {
    match kind {
        "captures-keystrokes" => 10,
        "captures-screen" | "captures-audio" | "uses-camera" => 9,
        "modifies-persistence" => 10,
        "executes-shell" | "code-execution" => 8,
        "encodes-payload" | "declares-lifecycle-hook" => 7,
        "reads-secrets" | "sends-external" | "publishes-public" => 7,
        "writes-file" | "installs-package" => 6,
        "writes-external" | "reads-user-data" | "reads-email" | "reads-browser-data" => 6,
        "calls-network" | "local-network" | "reads-env" | "requests-secrets" => 5,
        "reads-calendar" | "reads-documents" | "reads-memory" => 5,
        "reads-file" | "uses-clipboard" | "schedules-task" | "sends-message"
        | "browser-automation" | "uses-auth-profile" => 4,
        "unknown-high-risk" => 9,
        _ => 5,
    }
}

pub fn capability_category(kind: &str) -> FindingCategory {
    match kind {
        "executes-shell" | "code-execution" => FindingCategory::ShellExecution,
        "calls-network" | "local-network" => FindingCategory::NetworkAccess,
        "reads-file" | "writes-file" => FindingCategory::FileSystem,
        "reads-env" => FindingCategory::EnvironmentAccess,
        "requests-secrets" | "uses-auth-profile" | "reads-secrets" => {
            FindingCategory::CredentialPattern
        }
        "captures-keystrokes" => FindingCategory::Keylogging,
        "captures-screen" | "captures-audio" | "uses-camera" | "reads-user-data"
        | "reads-email" | "reads-calendar" | "reads-documents" | "reads-memory"
        | "reads-browser-data" | "sends-external" | "writes-external" | "publishes-public" => {
            FindingCategory::DataExfiltration
        }
        "encodes-payload" => FindingCategory::EncodedPayload,
        "installs-package" => FindingCategory::SupplyChain,
        "declares-lifecycle-hook" => FindingCategory::LifecycleHook,
        "modifies-persistence" => FindingCategory::Persistence,
        "uses-clipboard" | "schedules-task" | "sends-message" | "browser-automation" => {
            FindingCategory::ToolPermission
        }
        _ => FindingCategory::BundleShape,
    }
}

pub fn observation_selector_set(observations: &[CapabilityObservation]) -> HashSet<String> {
    let mut selectors = HashSet::new();
    for observation in observations {
        let kind = normalize_capability_kind(&observation.kind);
        selectors.insert(kind.clone());
        if let Some(target) = observation
            .target
            .as_deref()
            .map(normalize_capability_target)
            .filter(|target| !target.is_empty())
        {
            selectors.insert(format!("{}:{}", kind, target));
        }
    }
    selectors
}

fn default_capability_policy() -> Vec<CapabilityRule> {
    vec![
        CapabilityRule {
            id: "approve-sensitive-source-to-external-send",
            effect: "approval",
            all: &["sends-external"],
            any: &[
                "reads-user-data",
                "reads-email",
                "reads-calendar",
                "reads-documents",
                "reads-memory",
                "reads-browser-data",
                "reads-file",
            ],
            message: "Requires approval before combining sensitive data access with external delivery.",
            severity: 8,
        },
        CapabilityRule {
            id: "block-secret-access-to-external-send",
            effect: "block",
            all: &["sends-external"],
            any: &["reads-secrets", "reads-env"],
            message: "Blocks combinations that can expose credentials or authentication material to an external destination.",
            severity: 10,
        },
        CapabilityRule {
            id: "block-shell-secret-network",
            effect: "block",
            all: &["calls-network", "reads-secrets", "executes-shell"],
            any: &[],
            message: "Blocks combinations that can pair credential access, shell execution, and network egress.",
            severity: 10,
        },
        CapabilityRule {
            id: "block-code-secret-network",
            effect: "block",
            all: &["calls-network", "reads-secrets", "code-execution"],
            any: &[],
            message: "Blocks combinations that can pair credential access, code execution, and network egress.",
            severity: 10,
        },
        CapabilityRule {
            id: "block-keystrokes-with-network",
            effect: "block",
            all: &["captures-keystrokes", "calls-network"],
            any: &[],
            message: "Blocks capabilities that capture keystrokes and communicate over the network.",
            severity: 10,
        },
        CapabilityRule {
            id: "block-sensor-capture-with-network",
            effect: "block",
            all: &["calls-network"],
            any: &["captures-screen", "captures-audio", "uses-camera"],
            message: "Blocks capabilities that capture screen, microphone, or camera data and communicate over the network.",
            severity: 10,
        },
        CapabilityRule {
            id: "block-shell-env-network",
            effect: "block",
            all: &["executes-shell", "reads-env", "calls-network"],
            any: &[],
            message: "Blocks capability combinations that can combine shell execution, environment access, and network calls.",
            severity: 10,
        },
        CapabilityRule {
            id: "block-shell-file-network",
            effect: "block",
            all: &["executes-shell", "calls-network"],
            any: &["reads-file", "writes-file"],
            message: "Blocks capability combinations that can combine shell execution, file access, and network calls.",
            severity: 10,
        },
        CapabilityRule {
            id: "block-shell-encoded-payload",
            effect: "block",
            all: &["executes-shell", "encodes-payload"],
            any: &[],
            message: "Blocks capability combinations that pair shell execution with encoded or obfuscated payloads.",
            severity: 9,
        },
        CapabilityRule {
            id: "block-persistence-shell",
            effect: "block",
            all: &["modifies-persistence", "executes-shell"],
            any: &[],
            message: "Blocks capabilities that can install persistent behavior through shell execution.",
            severity: 10,
        },
        CapabilityRule {
            id: "warn-lifecycle-hook",
            effect: "warn",
            all: &["declares-lifecycle-hook"],
            any: &[],
            message: "Lifecycle hooks require review because they can run outside the visible task flow.",
            severity: 7,
        },
        CapabilityRule {
            id: "warn-package-install",
            effect: "warn",
            all: &["installs-package"],
            any: &[],
            message: "Package installation requires supply-chain review.",
            severity: 6,
        },
        CapabilityRule {
            id: "warn-shell",
            effect: "warn",
            all: &["executes-shell"],
            any: &[],
            message: "Shell execution requires source review.",
            severity: 6,
        },
        CapabilityRule {
            id: "block-unknown-high-risk",
            effect: "block",
            all: &["unknown-high-risk"],
            any: &[],
            message: "Blocks high-risk behavior outside the stable capability vocabulary.",
            severity: 9,
        },
    ]
}

fn capability_finding(observation: &CapabilityObservation) -> AnalysisFinding {
    let kind = normalize_capability_kind(&observation.kind);
    let evidence = observation
        .evidence
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(observation.target.as_deref())
        .unwrap_or(&kind);
    AnalysisFinding {
        category: capability_category(&kind),
        description: format!("Capability detected: {}", kind),
        matched_text: evidence.chars().take(160).collect(),
        line_number: 1,
        severity: capability_severity(&kind),
        file_path: None,
    }
}

pub fn evaluate_capability_observations(
    observations: Vec<CapabilityObservation>,
) -> CapabilityLayerReport {
    let selectors = observation_selector_set(&observations);
    let capability_kinds: HashSet<String> = observations
        .iter()
        .map(|observation| normalize_capability_kind(&observation.kind))
        .collect();
    let mut matched_rules = Vec::new();
    let mut warnings = Vec::new();
    let mut blocked = false;

    for rule in default_capability_policy() {
        let all_match = rule
            .all
            .iter()
            .map(|selector| normalize_capability_selector(selector))
            .all(|selector| selectors.contains(&selector));
        let any_match = rule.any.is_empty()
            || rule
                .any
                .iter()
                .map(|selector| normalize_capability_selector(selector))
                .any(|selector| selectors.contains(&selector));
        if !all_match || !any_match {
            continue;
        }
        if rule.effect == "block" {
            blocked = true;
        }
        warnings.push(rule.message.to_string());
        matched_rules.push(MatchedCapabilityRule {
            id: rule.id.to_string(),
            effect: rule.effect.to_string(),
            message: rule.message.to_string(),
            severity: rule.severity,
        });
    }

    let mut findings = observations
        .iter()
        .map(capability_finding)
        .collect::<Vec<_>>();
    findings.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| a.matched_text.cmp(&b.matched_text))
    });

    let capability_severity: u32 = capability_kinds
        .iter()
        .map(|kind| capability_severity(kind))
        .sum();
    let rule_severity: u32 = matched_rules.iter().map(|rule| rule.severity).sum();
    let total_severity = capability_severity.saturating_add(rule_severity);
    let mut score = ((total_severity as f32) / 4.0).min(10.0);
    if blocked {
        score = score.max(8.5);
    } else if !matched_rules.is_empty() {
        score = score.max(5.0);
    }
    let risk_score_10 = (score * 10.0).round() / 10.0;
    let risk_band = if risk_score_10 < 5.0 {
        "secure"
    } else if risk_score_10 < 8.0 {
        "review"
    } else {
        "risky"
    }
    .to_string();
    let threat_level = if blocked || risk_score_10 >= 8.0 {
        ThreatLevel::Malicious
    } else if risk_score_10 >= 5.0 {
        ThreatLevel::Suspicious
    } else {
        ThreatLevel::Clean
    };

    CapabilityLayerReport {
        observations,
        blocked,
        threat_level,
        risk_score_10,
        risk_band,
        total_severity,
        warnings,
        findings,
        matched_rules,
    }
}

fn push_observation(
    out: &mut Vec<CapabilityObservation>,
    seen: &mut BTreeSet<String>,
    layer: &str,
    entity_id: &str,
    kind: &str,
    target: Option<&str>,
    evidence: &str,
) {
    let kind = normalize_capability_kind(kind);
    let target = target
        .map(normalize_capability_target)
        .filter(|value| !value.is_empty());
    let dedupe_key = format!("{}:{}:{}", layer, kind, target.as_deref().unwrap_or(""));
    if !seen.insert(dedupe_key) {
        return;
    }
    out.push(CapabilityObservation {
        layer: layer.to_string(),
        entity_id: entity_id.to_string(),
        kind,
        target,
        evidence: Some(evidence.to_string()),
        confidence: Some(1.0),
    });
}

fn push_permission_observations(
    out: &mut Vec<CapabilityObservation>,
    seen: &mut BTreeSet<String>,
    layer: &str,
    entity_id: &str,
    permission: &Permission,
    evidence: &str,
) {
    match permission {
        Permission::Network => {
            push_observation(out, seen, layer, entity_id, "calls-network", None, evidence);
        }
        Permission::FileRead => {
            push_observation(out, seen, layer, entity_id, "reads-file", None, evidence);
        }
        Permission::FileWrite => {
            push_observation(out, seen, layer, entity_id, "writes-file", None, evidence);
        }
        Permission::Shell => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "executes-shell",
                None,
                evidence,
            );
        }
        Permission::Clipboard => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "uses-clipboard",
                None,
                evidence,
            );
        }
        Permission::Scheduler => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "schedules-task",
                None,
                evidence,
            );
        }
        Permission::Gmail => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "calls-network",
                Some("gmail.googleapis.com"),
                evidence,
            );
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "uses-auth-profile",
                None,
                evidence,
            );
            push_observation(out, seen, layer, entity_id, "sends-message", None, evidence);
        }
        Permission::CodeExecute => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "code-execution",
                None,
                evidence,
            );
        }
        Permission::LocalNetworkDiscovery => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "calls-network",
                Some("local-network"),
                evidence,
            );
        }
        Permission::ImageGeneration | Permission::Research => {
            push_observation(out, seen, layer, entity_id, "calls-network", None, evidence);
        }
        Permission::Custom(raw) => {
            let selector = normalize_capability_selector(raw);
            let (kind, target) = selector
                .split_once(':')
                .map(|(kind, target)| (kind, Some(target)))
                .unwrap_or_else(|| (selector.as_str(), None));
            if CAPABILITY_VOCABULARY.contains(&kind) {
                push_observation(out, seen, layer, entity_id, kind, target, evidence);
            } else {
                push_observation(
                    out,
                    seen,
                    layer,
                    entity_id,
                    "unknown-high-risk",
                    None,
                    evidence,
                );
            }
        }
    }
}

pub fn observations_from_declared_capabilities(
    layer: &str,
    entity_id: &str,
    capabilities: &[String],
) -> Vec<CapabilityObservation> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let action = crate::actions::ActionDef {
        name: entity_id.to_string(),
        capabilities: capabilities.to_vec(),
        ..crate::actions::ActionDef::default()
    };
    for raw in capabilities {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let handled_structured = push_structured_capability_observations(
            &mut out, &mut seen, layer, entity_id, &action, trimmed,
        );
        let permission = ActionGuard::parse_permission(trimmed);
        if !handled_structured || !matches!(&permission, Permission::Custom(_)) {
            let evidence = format!("declared capability '{}'", trimmed);
            push_permission_observations(
                &mut out,
                &mut seen,
                layer,
                entity_id,
                &permission,
                &evidence,
            );
        }
    }
    out
}

fn push_structured_capability_observations(
    out: &mut Vec<CapabilityObservation>,
    seen: &mut BTreeSet<String>,
    layer: &str,
    entity_id: &str,
    action: &crate::actions::ActionDef,
    raw: &str,
) -> bool {
    let capability = normalize_capability_kind(raw);
    let metadata = action.action_metadata();
    let evidence = format!("declared action capability '{}'", raw.trim());

    match capability.as_str() {
        "memory" => {
            push_observation(out, seen, layer, entity_id, "reads-memory", None, &evidence);
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "reads-user-data",
                Some("memory"),
                &evidence,
            );
            true
        }
        "documents" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "reads-documents",
                None,
                &evidence,
            );
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "reads-user-data",
                Some("documents"),
                &evidence,
            );
            true
        }
        "database-readonly" | "analytics" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "reads-user-data",
                Some("database"),
                &evidence,
            );
            true
        }
        "capability-inventory" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "reads-user-data",
                Some("capabilities"),
                &evidence,
            );
            true
        }
        "watcher-inventory" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "reads-user-data",
                Some("watchers"),
                &evidence,
            );
            true
        }
        "integration-inventory" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "reads-user-data",
                Some("integrations"),
                &evidence,
            );
            true
        }
        "platform-observability"
        | "app-registry"
        | "app-inventory"
        | "personal-activity"
        | "activity-insights"
        | "conversation-history"
        | "session-history"
        | "model-runtime"
        | "model-status"
        | "provider-status" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "reads-user-data",
                Some("agentark"),
                &evidence,
            );
            true
        }
        "agentark-capabilities" | "agentark-manual" | "documentation" | "time" => true,
        "gmail" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "calls-network",
                Some("gmail.googleapis.com"),
                &evidence,
            );
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "uses-auth-profile",
                None,
                &evidence,
            );
            if matches!(
                metadata.role,
                crate::actions::ActionRole::Delivery | crate::actions::ActionRole::Mutation
            ) {
                push_observation(
                    out,
                    seen,
                    layer,
                    entity_id,
                    "sends-message",
                    None,
                    &evidence,
                );
                push_observation(
                    out,
                    seen,
                    layer,
                    entity_id,
                    "sends-external",
                    Some("gmail.googleapis.com"),
                    &evidence,
                );
            } else {
                push_observation(out, seen, layer, entity_id, "reads-email", None, &evidence);
                push_observation(
                    out,
                    seen,
                    layer,
                    entity_id,
                    "reads-user-data",
                    Some("email"),
                    &evidence,
                );
            }
            true
        }
        "google-workspace" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "calls-network",
                Some("googleapis.com"),
                &evidence,
            );
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "uses-auth-profile",
                None,
                &evidence,
            );
            match metadata.role {
                crate::actions::ActionRole::DataSource | crate::actions::ActionRole::Inspection => {
                    push_observation(
                        out,
                        seen,
                        layer,
                        entity_id,
                        "reads-user-data",
                        Some("google_workspace"),
                        &evidence,
                    );
                }
                crate::actions::ActionRole::Delivery => {
                    push_observation(
                        out,
                        seen,
                        layer,
                        entity_id,
                        "sends-message",
                        None,
                        &evidence,
                    );
                    push_observation(
                        out,
                        seen,
                        layer,
                        entity_id,
                        "sends-external",
                        Some("google_workspace"),
                        &evidence,
                    );
                }
                crate::actions::ActionRole::Mutation => {
                    push_observation(
                        out,
                        seen,
                        layer,
                        entity_id,
                        "writes-external",
                        Some("google_workspace"),
                        &evidence,
                    );
                }
                _ => {}
            }
            true
        }
        "notify" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "sends-message",
                None,
                &evidence,
            );
            true
        }
        "clipboard-read" | "clipboard-write" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "uses-clipboard",
                None,
                &evidence,
            );
            true
        }
        "browser" | "browser-automation" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "browser-automation",
                None,
                &evidence,
            );
            if matches!(
                metadata.role,
                crate::actions::ActionRole::Inspection | crate::actions::ActionRole::DataSource
            ) {
                push_observation(
                    out,
                    seen,
                    layer,
                    entity_id,
                    "reads-browser-data",
                    None,
                    &evidence,
                );
            }
            true
        }
        "local-network" | "local-network-discovery" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "local-network",
                None,
                &evidence,
            );
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "calls-network",
                Some("local-network"),
                &evidence,
            );
            true
        }
        "messaging-send" | "telegram" | "whatsapp" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "sends-message",
                None,
                &evidence,
            );
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "sends-external",
                None,
                &evidence,
            );
            true
        }
        "custom-api" | "integration" | "broad-network" | "search" | "vision-ocr"
        | "video-generation" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "calls-network",
                None,
                &evidence,
            );
            true
        }
        "external-write" | "writes-external" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "writes-external",
                None,
                &evidence,
            );
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "calls-network",
                None,
                &evidence,
            );
            true
        }
        "home-assistant" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "local-network",
                None,
                &evidence,
            );
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "calls-network",
                Some("local-network"),
                &evidence,
            );
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "reads-user-data",
                Some("home-assistant"),
                &evidence,
            );
            true
        }
        "home-assistant-control" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "local-network",
                None,
                &evidence,
            );
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "calls-network",
                Some("local-network"),
                &evidence,
            );
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "writes-external",
                Some("home-assistant"),
                &evidence,
            );
            true
        }
        "app-hosting" => {
            if matches!(metadata.role, crate::actions::ActionRole::Inspection) {
                push_observation(out, seen, layer, entity_id, "reads-file", None, &evidence);
            } else {
                push_observation(out, seen, layer, entity_id, "writes-file", None, &evidence);
                push_observation(
                    out,
                    seen,
                    layer,
                    entity_id,
                    "modifies-persistence",
                    Some("app"),
                    &evidence,
                );
            }
            true
        }
        "scheduler" | "watcher" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "schedules-task",
                None,
                &evidence,
            );
            true
        }
        "skill-management"
        | "integration-builder"
        | "integration-admin"
        | "goal-management"
        | "self-evolve" => {
            if matches!(
                metadata.role,
                crate::actions::ActionRole::Inspection | crate::actions::ActionRole::DataSource
            ) {
                push_observation(
                    out,
                    seen,
                    layer,
                    entity_id,
                    "reads-user-data",
                    Some("agentark"),
                    &evidence,
                );
            } else {
                push_observation(
                    out,
                    seen,
                    layer,
                    entity_id,
                    "modifies-persistence",
                    Some("agentark"),
                    &evidence,
                );
            }
            true
        }
        "pdf-generation" | "document-generation" => {
            push_observation(out, seen, layer, entity_id, "writes-file", None, &evidence);
            true
        }
        "orchestration" | "swarm" | "delegate" | "multi-agent" | "agent-orchestration" => true,
        "local-cli" | "ssh" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "executes-shell",
                None,
                &evidence,
            );
            if capability == "ssh" {
                push_observation(
                    out,
                    seen,
                    layer,
                    entity_id,
                    "calls-network",
                    Some("ssh"),
                    &evidence,
                );
            }
            true
        }
        "requests-secrets" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "requests-secrets",
                None,
                &evidence,
            );
            true
        }
        "reads-secrets" => {
            push_observation(
                out,
                seen,
                layer,
                entity_id,
                "reads-secrets",
                None,
                &evidence,
            );
            true
        }
        _ => false,
    }
}

pub fn observations_from_action_def(
    layer: &str,
    action: &crate::actions::ActionDef,
    arguments: Option<&serde_json::Value>,
) -> Vec<CapabilityObservation> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let entity_id = action.name.as_str();

    for raw in &action.capabilities {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let handled_structured = push_structured_capability_observations(
            &mut out, &mut seen, layer, entity_id, action, trimmed,
        );
        let permission = ActionGuard::parse_permission(trimmed);
        if !handled_structured || !matches!(&permission, Permission::Custom(_)) {
            let evidence = format!("declared capability '{}'", trimmed);
            push_permission_observations(
                &mut out,
                &mut seen,
                layer,
                entity_id,
                &permission,
                &evidence,
            );
        }
    }

    let access = &action.authorization.access;
    for permission_id in &access.permission_ids {
        let evidence = format!("declared access permission '{}'", permission_id.trim());
        match normalize_capability_kind(permission_id).as_str() {
            "app-hosting" => {
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "writes-file",
                    None,
                    &evidence,
                );
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "modifies-persistence",
                    Some("app"),
                    &evidence,
                );
            }
            "browser-auto" | "browser-automation" => {
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "browser-automation",
                    None,
                    &evidence,
                );
            }
            "calendar-write" => {
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "calls-network",
                    Some("googleapis.com"),
                    &evidence,
                );
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "uses-auth-profile",
                    None,
                    &evidence,
                );
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "writes-external",
                    Some("google_calendar"),
                    &evidence,
                );
            }
            "capability-acquire" => {
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "modifies-persistence",
                    Some("agentark"),
                    &evidence,
                );
            }
            "google-workspace-command" => {
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "calls-network",
                    Some("googleapis.com"),
                    &evidence,
                );
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "uses-auth-profile",
                    None,
                    &evidence,
                );
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "writes-external",
                    Some("google_workspace"),
                    &evidence,
                );
            }
            "messaging-send" => {
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "sends-message",
                    None,
                    &evidence,
                );
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "sends-external",
                    None,
                    &evidence,
                );
            }
            "broad-network" => {
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "calls-network",
                    None,
                    &evidence,
                );
            }
            "auth-profile" | "uses-auth-profile" => {
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "uses-auth-profile",
                    None,
                    &evidence,
                );
            }
            "reads-secrets" => {
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "reads-secrets",
                    None,
                    &evidence,
                );
            }
            "requests-secrets" => {
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "requests-secrets",
                    None,
                    &evidence,
                );
            }
            "ssh" => {
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "executes-shell",
                    None,
                    &evidence,
                );
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "calls-network",
                    Some("ssh"),
                    &evidence,
                );
            }
            "watcher" => {
                push_observation(
                    &mut out,
                    &mut seen,
                    layer,
                    entity_id,
                    "schedules-task",
                    None,
                    &evidence,
                );
            }
            "swarm" => {}
            _ => {}
        }
    }

    if access.requires_ssh_connection {
        push_observation(
            &mut out,
            &mut seen,
            layer,
            entity_id,
            "executes-shell",
            None,
            "action access requires an SSH connection",
        );
        push_observation(
            &mut out,
            &mut seen,
            layer,
            entity_id,
            "calls-network",
            Some("ssh"),
            "action access requires an SSH connection",
        );
    }

    if action.authorization.requires_auth
        || !access.integration_ids.is_empty()
        || !access.extension_pack_ids.is_empty()
    {
        push_observation(
            &mut out,
            &mut seen,
            layer,
            entity_id,
            "uses-auth-profile",
            None,
            "action authorization requires configured auth",
        );
    }

    for target in &access.channel_targets {
        let argument_target = arguments
            .and_then(|value| value.get(target.argument_key.as_str()))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(target.default_target.as_str());
        let normalized_target = normalize_capability_target(argument_target);
        let in_process_delivery = matches!(
            normalized_target.as_str(),
            "" | "web" | "in_app" | "app" | "app_notification" | "app_notifications"
        );
        push_observation(
            &mut out,
            &mut seen,
            layer,
            entity_id,
            "sends-message",
            Some(argument_target),
            "action access declares a messaging channel target",
        );
        if !in_process_delivery {
            push_observation(
                &mut out,
                &mut seen,
                layer,
                entity_id,
                "sends-external",
                Some(argument_target),
                "action access declares an external messaging channel target",
            );
        }
    }

    if action.authorization.outbound.outbound_write {
        push_observation(
            &mut out,
            &mut seen,
            layer,
            entity_id,
            "writes-external",
            None,
            "action authorization declares outbound writes",
        );
        push_observation(
            &mut out,
            &mut seen,
            layer,
            entity_id,
            "sends-external",
            None,
            "action authorization declares outbound writes",
        );
        push_observation(
            &mut out,
            &mut seen,
            layer,
            entity_id,
            "calls-network",
            None,
            "action authorization declares outbound writes",
        );
    }

    if action.authorization.outbound.public_publish {
        push_observation(
            &mut out,
            &mut seen,
            layer,
            entity_id,
            "publishes-public",
            None,
            "action authorization declares public publishing",
        );
        push_observation(
            &mut out,
            &mut seen,
            layer,
            entity_id,
            "sends-external",
            None,
            "action authorization declares public publishing",
        );
        push_observation(
            &mut out,
            &mut seen,
            layer,
            entity_id,
            "calls-network",
            None,
            "action authorization declares public publishing",
        );
    }

    out
}

pub fn evaluate_declared_capabilities(
    layer: &str,
    entity_id: &str,
    capabilities: &[String],
) -> CapabilityLayerReport {
    evaluate_capability_observations(observations_from_declared_capabilities(
        layer,
        entity_id,
        capabilities,
    ))
}

pub fn evaluate_cross_layer_capabilities(
    observations: Vec<CapabilityObservation>,
) -> Option<CapabilityLayerReport> {
    let distinct_subjects = observations
        .iter()
        .map(|observation| (observation.layer.as_str(), observation.entity_id.as_str()))
        .collect::<HashSet<_>>();
    if distinct_subjects.len() < 2 {
        return None;
    }

    let selector_subjects = observations.iter().fold(
        HashMap::<String, HashSet<(String, String)>>::new(),
        |mut acc, observation| {
            acc.entry(normalize_capability_kind(&observation.kind))
                .or_default()
                .insert((observation.layer.clone(), observation.entity_id.clone()));
            if let Some(target) = observation
                .target
                .as_deref()
                .map(normalize_capability_target)
                .filter(|target| !target.is_empty())
            {
                acc.entry(format!(
                    "{}:{}",
                    normalize_capability_kind(&observation.kind),
                    target
                ))
                .or_default()
                .insert((observation.layer.clone(), observation.entity_id.clone()));
            }
            acc
        },
    );

    let selectors = observation_selector_set(&observations);
    let mut correlated_rules = Vec::new();
    for rule in default_capability_policy() {
        let all_selectors = rule
            .all
            .iter()
            .map(|selector| normalize_capability_selector(selector))
            .collect::<Vec<_>>();
        let any_selectors = rule
            .any
            .iter()
            .map(|selector| normalize_capability_selector(selector))
            .collect::<Vec<_>>();
        let all_match = all_selectors
            .iter()
            .all(|selector| selectors.contains(selector));
        let matched_any = if any_selectors.is_empty() {
            Vec::new()
        } else {
            any_selectors
                .iter()
                .filter(|selector| selectors.contains(*selector))
                .cloned()
                .collect::<Vec<_>>()
        };
        let any_match = any_selectors.is_empty() || !matched_any.is_empty();
        if !all_match || !any_match {
            continue;
        }

        let mut subjects = HashSet::new();
        for selector in all_selectors.iter().chain(matched_any.iter()) {
            if let Some(selector_subjects) = selector_subjects.get(selector) {
                subjects.extend(selector_subjects.iter().cloned());
            }
        }
        if subjects.len() >= 2 {
            correlated_rules.push(rule.id);
        }
    }

    if correlated_rules.is_empty() {
        return None;
    }

    let mut report = evaluate_capability_observations(observations);
    report
        .matched_rules
        .retain(|rule| correlated_rules.contains(&rule.id.as_str()));
    report.warnings = report
        .matched_rules
        .iter()
        .map(|rule| rule.message.to_string())
        .collect();
    report.blocked = report
        .matched_rules
        .iter()
        .any(|rule| rule.effect == "block");
    if report.blocked {
        report.threat_level = ThreatLevel::Malicious;
        report.risk_score_10 = report.risk_score_10.max(8.5);
        report.risk_band = "risky".to_string();
    } else {
        report.threat_level = ThreatLevel::Suspicious;
        report.risk_score_10 = report.risk_score_10.max(6.5);
        report.risk_band = "review".to_string();
    }
    Some(report)
}

pub fn evaluate_capability_correlation(
    prior_observations: &[CapabilityObservation],
    candidate_observations: &[CapabilityObservation],
) -> CapabilityCorrelationDecision {
    if candidate_observations.is_empty() {
        return CapabilityCorrelationDecision::allow();
    }
    let mut combined = prior_observations.to_vec();
    combined.extend(candidate_observations.iter().cloned());
    let Some(report) = evaluate_cross_layer_capabilities(combined) else {
        return CapabilityCorrelationDecision::allow();
    };
    if report.blocked {
        CapabilityCorrelationDecision::block(report)
    } else if !report.matched_rules.is_empty() {
        CapabilityCorrelationDecision::requires_approval(report)
    } else {
        CapabilityCorrelationDecision::allow()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_capabilities_block_shell_file_network_combo() {
        let report = evaluate_declared_capabilities(
            "plugin",
            "demo",
            &["shell".into(), "network".into(), "file_write".into()],
        );

        assert!(report.blocked);
        assert!(report
            .matched_rules
            .iter()
            .any(|rule| rule.id == "block-shell-file-network"));
    }

    #[test]
    fn cross_layer_correlation_requires_multiple_subjects() {
        let observations = [
            ("skill", "reader", "reads-file"),
            ("plugin", "sender", "calls-network"),
            ("plugin", "sender", "executes-shell"),
        ]
        .iter()
        .map(|(layer, entity_id, kind)| CapabilityObservation {
            layer: layer.to_string(),
            entity_id: entity_id.to_string(),
            kind: kind.to_string(),
            target: None,
            evidence: None,
            confidence: Some(1.0),
        })
        .collect::<Vec<_>>();

        let report = evaluate_cross_layer_capabilities(observations)
            .expect("cross-layer report should be produced");
        assert!(report.blocked);
        assert!(report
            .matched_rules
            .iter()
            .any(|rule| rule.id == "block-shell-file-network"));
    }

    #[test]
    fn cross_layer_sensitive_read_to_external_send_requires_approval() {
        let observations = [
            ("runtime", "memory_lookup", "reads-memory"),
            ("runtime", "memory_lookup", "reads-user-data"),
            ("runtime", "notify_user", "sends-external"),
        ]
        .iter()
        .map(|(layer, entity_id, kind)| CapabilityObservation {
            layer: layer.to_string(),
            entity_id: entity_id.to_string(),
            kind: kind.to_string(),
            target: None,
            evidence: None,
            confidence: Some(1.0),
        })
        .collect::<Vec<_>>();

        let report = evaluate_cross_layer_capabilities(observations)
            .expect("approval report should be produced");
        assert!(!report.blocked);
        assert!(report.matched_rules.iter().any(|rule| rule.id
            == "approve-sensitive-source-to-external-send"
            && rule.effect == "approval"));
    }

    #[test]
    fn candidate_composite_sensitive_read_to_external_send_requires_approval() {
        let candidate = [
            ("runtime", "watch", "sends-external"),
            ("runtime", "gmail_scan", "reads-email"),
            ("runtime", "gmail_scan", "reads-user-data"),
        ]
        .iter()
        .map(|(layer, entity_id, kind)| CapabilityObservation {
            layer: layer.to_string(),
            entity_id: entity_id.to_string(),
            kind: kind.to_string(),
            target: None,
            evidence: None,
            confidence: Some(1.0),
        })
        .collect::<Vec<_>>();

        let decision = evaluate_capability_correlation(&[], &candidate);
        assert!(matches!(
            decision.effect,
            CapabilityCorrelationEffect::RequireApproval
        ));
        assert!(decision.report.as_ref().is_some_and(|report| {
            report
                .matched_rules
                .iter()
                .any(|rule| rule.id == "approve-sensitive-source-to-external-send")
        }));
    }

    #[test]
    fn in_app_channel_target_is_not_external_delivery() {
        let action = crate::actions::ActionDef {
            name: "watch".to_string(),
            capabilities: vec!["watcher".to_string()],
            authorization: crate::actions::ActionAuthorization {
                access: crate::actions::ActionAccessMetadata {
                    channel_targets: vec![crate::actions::ActionChannelTarget {
                        argument_key: "notify_channel".to_string(),
                        default_target: "preferred".to_string(),
                    }],
                    ..crate::actions::ActionAccessMetadata::default()
                },
                ..crate::actions::ActionAuthorization::default()
            },
            ..crate::actions::ActionDef::default()
        };

        let observations = observations_from_action_def(
            "runtime",
            &action,
            Some(&serde_json::json!({ "notify_channel": "in_app" })),
        );
        let selectors = observation_selector_set(&observations);

        assert!(selectors.contains("sends-message:in_app"));
        assert!(!selectors.contains("sends-external"));
        assert!(!selectors.contains("sends-external:in_app"));
    }

    #[test]
    fn action_def_observations_use_structured_metadata() {
        let mut action = crate::actions::ActionDef {
            name: "memory_lookup".to_string(),
            capabilities: vec!["memory".to_string()],
            ..crate::actions::ActionDef::default()
        };
        action.authorization.requires_auth = false;

        let observations = observations_from_action_def("runtime", &action, None);
        let selectors = observation_selector_set(&observations);

        assert!(selectors.contains("reads-memory"));
        assert!(selectors.contains("reads-user-data:memory"));
    }

    #[test]
    fn builtin_action_capability_aliases_do_not_emit_unknown_high_risk() {
        let aliases = [
            "agent_orchestration",
            "agentark_capabilities",
            "agentark_manual",
            "analytics",
            "app_inventory",
            "app_registry",
            "app_hosting",
            "activity_insights",
            "capability_inventory",
            "clipboard_read",
            "clipboard_write",
            "code_execute",
            "conversation_history",
            "database_readonly",
            "delegate",
            "document_generation",
            "documentation",
            "documents",
            "file_read",
            "file_write",
            "gmail",
            "goal_management",
            "google_workspace",
            "home_assistant",
            "home_assistant_control",
            "image_generation",
            "integration_admin",
            "integration_builder",
            "integration_inventory",
            "local_cli",
            "local_network",
            "local_network_discovery",
            "memory",
            "model_runtime",
            "model_status",
            "multi_agent",
            "network",
            "notify",
            "orchestration",
            "pdf_generation",
            "platform_observability",
            "personal_activity",
            "provider_status",
            "scheduler",
            "search",
            "self_evolve",
            "session_history",
            "shell",
            "skill_management",
            "ssh",
            "swarm",
            "telegram",
            "time",
            "video_generation",
            "vision_ocr",
            "watcher",
            "watcher_inventory",
            "whatsapp",
        ];

        for alias in aliases {
            let action = crate::actions::ActionDef {
                name: format!("probe_{}", alias),
                capabilities: vec![alias.to_string()],
                ..crate::actions::ActionDef::default()
            };
            let selectors =
                observation_selector_set(&observations_from_action_def("runtime", &action, None));
            assert!(
                !selectors.contains("unknown-high-risk"),
                "built-in capability alias '{}' emitted unknown-high-risk: {:?}",
                alias,
                selectors
            );
        }
    }

    #[test]
    fn declared_action_capability_aliases_do_not_emit_unknown_high_risk() {
        let capabilities = [
            "app_hosting".to_string(),
            "database_readonly".to_string(),
            "google_workspace".to_string(),
            "search".to_string(),
            "vision_ocr".to_string(),
        ];
        let selectors = observation_selector_set(&observations_from_declared_capabilities(
            "custom_api",
            "declared_alias_probe",
            &capabilities,
        ));

        assert!(!selectors.contains("unknown-high-risk"));
        assert!(selectors.contains("calls-network"));
        assert!(selectors.contains("reads-user-data:database"));
    }

    #[test]
    fn custom_api_action_capabilities_use_generic_external_api_vocabulary() {
        let capabilities = [
            "custom_api".to_string(),
            "integration".to_string(),
            "network".to_string(),
            "external_write".to_string(),
        ];
        let selectors = observation_selector_set(&observations_from_declared_capabilities(
            "custom_api",
            "api__project_tool__post_items",
            &capabilities,
        ));

        assert!(!selectors.contains("unknown-high-risk"));
        assert!(selectors.contains("calls-network"));
        assert!(selectors.contains("writes-external"));
    }

    #[test]
    fn unknown_action_capability_still_maps_to_unknown_high_risk() {
        let action = crate::actions::ActionDef {
            name: "custom_unknown".to_string(),
            capabilities: vec!["totally_new_host_control".to_string()],
            ..crate::actions::ActionDef::default()
        };
        let selectors =
            observation_selector_set(&observations_from_action_def("runtime", &action, None));

        assert!(selectors.contains("unknown-high-risk"));
    }

    #[test]
    fn builtin_access_permission_ids_stay_in_known_vocabulary() {
        let permission_ids = [
            "app_hosting",
            "browser_auto",
            "calendar_write",
            "capability_acquire",
            "google_workspace_command",
            "ssh",
            "swarm",
            "watcher",
        ];

        for permission_id in permission_ids {
            let action = crate::actions::ActionDef {
                name: format!("probe_{}", permission_id),
                authorization: crate::actions::ActionAuthorization {
                    access: crate::actions::ActionAccessMetadata {
                        permission_ids: vec![permission_id.to_string()],
                        ..crate::actions::ActionAccessMetadata::default()
                    },
                    ..crate::actions::ActionAuthorization::default()
                },
                ..crate::actions::ActionDef::default()
            };
            let selectors =
                observation_selector_set(&observations_from_action_def("runtime", &action, None));
            assert!(
                !selectors.contains("unknown-high-risk"),
                "built-in access permission '{}' emitted unknown-high-risk: {:?}",
                permission_id,
                selectors
            );
        }
    }

    #[test]
    fn http_get_after_file_read_does_not_trip_unknown_high_risk_policy() {
        let file_read = crate::actions::ActionDef {
            name: "file_read".to_string(),
            capabilities: vec!["file_read".to_string()],
            ..crate::actions::ActionDef::default()
        };
        let http_get = crate::actions::ActionDef {
            name: "http_get".to_string(),
            capabilities: vec!["network".to_string(), "search".to_string()],
            ..crate::actions::ActionDef::default()
        };

        let prior = observations_from_action_def("runtime", &file_read, None);
        let candidate = observations_from_action_def("runtime", &http_get, None);
        let decision = evaluate_capability_correlation(&prior, &candidate);

        assert_eq!(decision.effect, CapabilityCorrelationEffect::Allow);
    }

    #[test]
    fn run_context_deduplicates_observations() {
        let observation = CapabilityObservation {
            layer: "runtime".to_string(),
            entity_id: "memory_lookup".to_string(),
            kind: "reads-memory".to_string(),
            target: None,
            evidence: None,
            confidence: Some(1.0),
        };
        let mut context = RunCapabilityContext::default();
        context.extend(vec![observation.clone(), observation]);

        assert_eq!(context.observations().len(), 1);
    }
}
