//! First-class pipeline primitives: DAG specs, retry/idempotency policies,
//! and typed signal ranking/consensus.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

fn default_retry_attempts() -> u32 {
    3
}

fn default_retry_initial_backoff_ms() -> u64 {
    1_000
}

fn default_retry_max_backoff_ms() -> u64 {
    30_000
}

fn default_retry_jitter_ratio() -> f64 {
    0.2
}

fn default_retry_statuses() -> Vec<u16> {
    vec![429, 500, 502, 503, 504]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    #[serde(default = "default_retry_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_retry_initial_backoff_ms")]
    pub initial_backoff_ms: u64,
    #[serde(default = "default_retry_max_backoff_ms")]
    pub max_backoff_ms: u64,
    #[serde(default = "default_retry_jitter_ratio")]
    pub jitter_ratio: f64,
    #[serde(default = "default_retry_statuses")]
    pub retry_on_status: Vec<u16>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: default_retry_attempts(),
            initial_backoff_ms: default_retry_initial_backoff_ms(),
            max_backoff_ms: default_retry_max_backoff_ms(),
            jitter_ratio: default_retry_jitter_ratio(),
            retry_on_status: default_retry_statuses(),
        }
    }
}

impl RetryPolicy {
    pub fn normalized(&self) -> Self {
        Self {
            max_attempts: self.max_attempts.clamp(1, 20),
            initial_backoff_ms: self.initial_backoff_ms.clamp(50, 300_000),
            max_backoff_ms: self.max_backoff_ms.clamp(100, 600_000),
            jitter_ratio: self.jitter_ratio.clamp(0.0, 1.0),
            retry_on_status: self.retry_on_status.clone(),
        }
    }
}

fn default_idempotency_key_template() -> String {
    "{{pipeline}}:{{node}}:{{date}}".to_string()
}

fn default_idempotency_ttl_secs() -> u64 {
    24 * 60 * 60
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdempotencyPolicy {
    #[serde(default = "default_idempotency_key_template")]
    pub key_template: String,
    #[serde(default = "default_idempotency_ttl_secs")]
    pub ttl_secs: u64,
}

impl Default for IdempotencyPolicy {
    fn default() -> Self {
        Self {
            key_template: default_idempotency_key_template(),
            ttl_secs: default_idempotency_ttl_secs(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum NodeKind {
    #[default]
    Action,
    ConnectorRequest,
    SignalConsensus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum NodeErrorMode {
    #[default]
    Fail,
    Continue,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PipelineNode {
    pub id: String,
    #[serde(default)]
    pub kind: NodeKind,
    /// Action name for `action` nodes. Optional for special kinds where the
    /// runtime chooses the executor.
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub retry: RetryPolicy,
    #[serde(default)]
    pub idempotency: Option<IdempotencyPolicy>,
    #[serde(default)]
    pub on_error: NodeErrorMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PipelineSpec {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub schedule_cron: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub nodes: Vec<PipelineNode>,
    #[serde(default)]
    pub outputs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledPipeline {
    pub name: String,
    pub ordered_nodes: Vec<String>,
    pub warnings: Vec<String>,
    pub node_count: usize,
}

pub fn compile_pipeline(spec: &PipelineSpec) -> Result<CompiledPipeline> {
    if spec.name.trim().is_empty() {
        return Err(anyhow!("Pipeline name is required"));
    }
    if spec.nodes.is_empty() {
        return Err(anyhow!("Pipeline must define at least one node"));
    }

    let mut nodes_by_id = HashMap::<String, &PipelineNode>::new();
    for node in &spec.nodes {
        let id = node.id.trim();
        if id.is_empty() {
            return Err(anyhow!("Pipeline node id cannot be empty"));
        }
        if nodes_by_id.insert(id.to_string(), node).is_some() {
            return Err(anyhow!("Duplicate pipeline node id: {}", id));
        }
        if node.kind == NodeKind::Action && node.action.trim().is_empty() {
            return Err(anyhow!(
                "Action node '{}' must define a non-empty action name",
                id
            ));
        }
    }

    let mut indegree = HashMap::<String, usize>::new();
    let mut outgoing = HashMap::<String, Vec<String>>::new();
    for node in &spec.nodes {
        indegree.entry(node.id.clone()).or_insert(0);
        for dep in &node.depends_on {
            if !nodes_by_id.contains_key(dep) {
                return Err(anyhow!(
                    "Node '{}' depends on unknown node '{}'",
                    node.id,
                    dep
                ));
            }
            *indegree.entry(node.id.clone()).or_insert(0) += 1;
            outgoing
                .entry(dep.clone())
                .or_default()
                .push(node.id.clone());
        }
    }

    let mut queue = VecDeque::new();
    for node in &spec.nodes {
        if indegree.get(&node.id).copied().unwrap_or(0) == 0 {
            queue.push_back(node.id.clone());
        }
    }

    let mut ordered = Vec::with_capacity(spec.nodes.len());
    while let Some(node_id) = queue.pop_front() {
        ordered.push(node_id.clone());
        if let Some(nexts) = outgoing.get(&node_id) {
            for next in nexts {
                if let Some(v) = indegree.get_mut(next) {
                    *v = v.saturating_sub(1);
                    if *v == 0 {
                        queue.push_back(next.clone());
                    }
                }
            }
        }
    }

    if ordered.len() != spec.nodes.len() {
        return Err(anyhow!(
            "Pipeline has a dependency cycle; topological order is not possible"
        ));
    }

    let mut warnings = Vec::new();
    let produced: HashSet<String> = spec.nodes.iter().map(|n| n.id.clone()).collect();
    for out in &spec.outputs {
        if !produced.contains(out) {
            warnings.push(format!(
                "Requested output '{}' does not match any pipeline node id",
                out
            ));
        }
    }

    Ok(CompiledPipeline {
        name: spec.name.clone(),
        ordered_nodes: ordered,
        warnings,
        node_count: spec.nodes.len(),
    })
}

pub fn render_template(input: &str, context: &BTreeMap<String, String>) -> String {
    let mut out = input.to_string();
    for (k, v) in context {
        let token = format!("{{{{{}}}}}", k);
        out = out.replace(&token, v);
    }
    out
}

fn default_weight_impact() -> f64 {
    0.45
}

fn default_weight_confidence() -> f64 {
    0.35
}

fn default_weight_effort() -> f64 {
    0.20
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalWeights {
    #[serde(default = "default_weight_impact")]
    pub impact: f64,
    #[serde(default = "default_weight_confidence")]
    pub confidence: f64,
    #[serde(default = "default_weight_effort")]
    pub effort: f64,
}

impl Default for SignalWeights {
    fn default() -> Self {
        Self {
            impact: default_weight_impact(),
            confidence: default_weight_confidence(),
            effort: default_weight_effort(),
        }
    }
}

impl SignalWeights {
    pub fn normalized(&self) -> Self {
        let i = self.impact.max(0.0);
        let c = self.confidence.max(0.0);
        let e = self.effort.max(0.0);
        let sum = i + c + e;
        if sum <= f64::EPSILON {
            return Self::default();
        }
        Self {
            impact: i / sum,
            confidence: c / sum,
            effort: e / sum,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Signal {
    pub id: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub summary: Option<String>,
    /// Preferred range: 0..100
    #[serde(default)]
    pub impact: Option<f64>,
    /// Preferred range: 0..1 (also accepts 0..100)
    #[serde(default)]
    pub confidence: Option<f64>,
    /// Preferred range: 0..100 where lower is better
    #[serde(default)]
    pub effort: Option<f64>,
    #[serde(default)]
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConsensusPerspective {
    pub name: String,
    #[serde(default)]
    pub weights: SignalWeights,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SignalConsensusRequest {
    #[serde(default)]
    pub signals: Vec<Signal>,
    #[serde(default)]
    pub weights: SignalWeights,
    #[serde(default)]
    pub perspectives: Vec<ConsensusPerspective>,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

fn default_top_k() -> usize {
    20
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedSignal {
    pub id: String,
    pub source: String,
    pub score: f64,
    pub impact: f64,
    pub confidence: f64,
    pub effort: f64,
    pub summary: Option<String>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerspectiveRanking {
    pub name: String,
    pub weights: SignalWeights,
    pub top: Vec<RankedSignal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalConsensusResult {
    pub total_signals: usize,
    pub top_k: usize,
    pub top: Vec<RankedSignal>,
    pub perspectives: Vec<PerspectiveRanking>,
    pub weights: SignalWeights,
}

fn as_0_100(v: Option<f64>, default_val: f64) -> f64 {
    let raw = v.unwrap_or(default_val);
    if raw <= 1.0 {
        (raw * 100.0).clamp(0.0, 100.0)
    } else {
        raw.clamp(0.0, 100.0)
    }
}

fn signal_score(signal: &Signal, weights: &SignalWeights) -> (f64, f64, f64, f64) {
    let impact = as_0_100(signal.impact, 50.0);
    let confidence = as_0_100(signal.confidence, 60.0);
    let effort = as_0_100(signal.effort, 50.0);
    let score = impact * weights.impact
        + confidence * weights.confidence
        + (100.0 - effort) * weights.effort;
    (score, impact, confidence, effort)
}

fn rank_signals(signals: &[Signal], weights: &SignalWeights, top_k: usize) -> Vec<RankedSignal> {
    let mut ranked = Vec::with_capacity(signals.len());
    for signal in signals {
        let (score, impact, confidence, effort) = signal_score(signal, weights);
        ranked.push(RankedSignal {
            id: signal.id.clone(),
            source: signal.source.clone(),
            score,
            impact,
            confidence,
            effort,
            summary: signal.summary.clone(),
            payload: signal.payload.clone(),
        });
    }
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked.truncate(top_k.min(ranked.len()));
    ranked
}

pub fn run_signal_consensus(request: &SignalConsensusRequest) -> Result<SignalConsensusResult> {
    if request.signals.is_empty() {
        return Err(anyhow!("signals cannot be empty"));
    }
    let weights = request.weights.normalized();
    let top_k = request.top_k.clamp(1, 200);

    let top = rank_signals(&request.signals, &weights, top_k);

    let mut perspective_rankings = Vec::new();
    for p in &request.perspectives {
        let p_weights = p.weights.normalized();
        let p_top = rank_signals(&request.signals, &p_weights, top_k);
        perspective_rankings.push(PerspectiveRanking {
            name: p.name.clone(),
            weights: p_weights,
            top: p_top,
        });
    }

    Ok(SignalConsensusResult {
        total_signals: request.signals.len(),
        top_k,
        top,
        perspectives: perspective_rankings,
        weights,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_pipeline_topological_order() {
        let spec = PipelineSpec {
            name: "daily-council".to_string(),
            nodes: vec![
                PipelineNode {
                    id: "collect".to_string(),
                    kind: NodeKind::Action,
                    action: "connector_request".to_string(),
                    ..Default::default()
                },
                PipelineNode {
                    id: "rank".to_string(),
                    kind: NodeKind::SignalConsensus,
                    depends_on: vec!["collect".to_string()],
                    ..Default::default()
                },
                PipelineNode {
                    id: "notify".to_string(),
                    kind: NodeKind::Action,
                    action: "daily_brief".to_string(),
                    depends_on: vec!["rank".to_string()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let compiled = compile_pipeline(&spec).expect("compile");
        assert_eq!(compiled.ordered_nodes, vec!["collect", "rank", "notify"]);
    }

    #[test]
    fn compile_pipeline_detects_cycle() {
        let spec = PipelineSpec {
            name: "bad".to_string(),
            nodes: vec![
                PipelineNode {
                    id: "a".to_string(),
                    action: "x".to_string(),
                    depends_on: vec!["b".to_string()],
                    ..Default::default()
                },
                PipelineNode {
                    id: "b".to_string(),
                    action: "y".to_string(),
                    depends_on: vec!["a".to_string()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert!(compile_pipeline(&spec).is_err());
    }

    #[test]
    fn signal_consensus_ranks_higher_impact_lower_effort() {
        let req = SignalConsensusRequest {
            signals: vec![
                Signal {
                    id: "low".to_string(),
                    source: "x".to_string(),
                    impact: Some(20.0),
                    confidence: Some(0.7),
                    effort: Some(80.0),
                    ..Default::default()
                },
                Signal {
                    id: "high".to_string(),
                    source: "x".to_string(),
                    impact: Some(85.0),
                    confidence: Some(0.8),
                    effort: Some(20.0),
                    ..Default::default()
                },
            ],
            top_k: 2,
            ..Default::default()
        };
        let out = run_signal_consensus(&req).expect("consensus");
        assert_eq!(out.top.first().map(|s| s.id.as_str()), Some("high"));
    }
}
