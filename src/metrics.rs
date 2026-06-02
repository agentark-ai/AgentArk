use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::time::Duration;

use once_cell::sync::Lazy;

const HTTP_DURATION_BUCKETS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];
const LLM_DURATION_BUCKETS: [f64; 10] = [0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0];
const CONTAINER_DURATION_BUCKETS: [f64; 10] =
    [0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 30.0, 120.0];

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
struct SeriesKey {
    labels: Vec<(String, String)>,
}

impl Hash for SeriesKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for (key, value) in &self.labels {
            key.hash(state);
            value.hash(state);
        }
    }
}

#[derive(Clone)]
struct HistogramData {
    buckets: Vec<u64>,
    count: u64,
    sum: f64,
}

impl HistogramData {
    fn new(bucket_count: usize) -> Self {
        Self {
            buckets: vec![0; bucket_count],
            count: 0,
            sum: 0.0,
        }
    }

    fn observe(&mut self, value: f64, bucket_bounds: &[f64]) {
        self.count = self.count.saturating_add(1);
        self.sum += value;
        for (idx, upper_bound) in bucket_bounds.iter().enumerate() {
            if value <= *upper_bound {
                self.buckets[idx] = self.buckets[idx].saturating_add(1);
            }
        }
    }
}

#[derive(Default)]
struct MetricStore {
    counters: Mutex<HashMap<SeriesKey, u64>>,
    histograms: Mutex<HashMap<SeriesKey, HistogramData>>,
    gauges: Mutex<HashMap<SeriesKey, f64>>,
}

impl MetricStore {
    fn increment_counter(&self, labels: Vec<(&str, String)>, value: u64) {
        let key = series_key(labels);
        let mut counters = self.counters.lock().expect("metrics counter lock");
        let entry = counters.entry(key).or_insert(0);
        *entry = entry.saturating_add(value);
    }

    fn observe_histogram(&self, labels: Vec<(&str, String)>, value: f64, bucket_bounds: &[f64]) {
        let key = series_key(labels);
        let mut histograms = self.histograms.lock().expect("metrics histogram lock");
        let entry = histograms
            .entry(key)
            .or_insert_with(|| HistogramData::new(bucket_bounds.len()));
        entry.observe(value, bucket_bounds);
    }

    fn set_gauge(&self, labels: Vec<(&str, String)>, value: f64) {
        let key = series_key(labels);
        let mut gauges = self.gauges.lock().expect("metrics gauge lock");
        gauges.insert(key, value);
    }
}

static HTTP_REQUESTS_TOTAL: Lazy<MetricStore> = Lazy::new(MetricStore::default);
static HTTP_REQUEST_DURATION_SECONDS: Lazy<MetricStore> = Lazy::new(MetricStore::default);
static LLM_CALLS_TOTAL: Lazy<MetricStore> = Lazy::new(MetricStore::default);
static LLM_CALL_DURATION_SECONDS: Lazy<MetricStore> = Lazy::new(MetricStore::default);
static LLM_TOKENS_TOTAL: Lazy<MetricStore> = Lazy::new(MetricStore::default);
static CONTAINER_RUNS_TOTAL: Lazy<MetricStore> = Lazy::new(MetricStore::default);
static CONTAINER_LIFECYCLE_DURATION_SECONDS: Lazy<MetricStore> = Lazy::new(MetricStore::default);
static CONTAINER_SWEEPER_RUNS_TOTAL: Lazy<MetricStore> = Lazy::new(MetricStore::default);
static CONTAINER_SWEEPER_REMOVED_TOTAL: Lazy<MetricStore> = Lazy::new(MetricStore::default);
static ACTIVE_CONTAINERS_GAUGE: Lazy<MetricStore> = Lazy::new(MetricStore::default);
static BACKGROUND_TASK_PANICS_TOTAL: Lazy<MetricStore> = Lazy::new(MetricStore::default);

pub fn observe_http_request(method: &str, path: &str, status: u16, duration: Duration) {
    let labels = vec![
        ("method", method.to_string()),
        ("path", path.to_string()),
        ("status", status.to_string()),
    ];
    HTTP_REQUESTS_TOTAL.increment_counter(labels.clone(), 1);
    HTTP_REQUEST_DURATION_SECONDS.observe_histogram(
        labels,
        duration.as_secs_f64(),
        &HTTP_DURATION_BUCKETS,
    );
}

pub fn observe_llm_call(
    provider: &str,
    model: &str,
    status: &str,
    duration: Duration,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
) {
    let labels = vec![
        ("provider", provider.to_string()),
        ("model", model.to_string()),
        ("status", status.to_string()),
    ];
    LLM_CALLS_TOTAL.increment_counter(labels.clone(), 1);
    LLM_CALL_DURATION_SECONDS.observe_histogram(
        labels,
        duration.as_secs_f64(),
        &LLM_DURATION_BUCKETS,
    );
    if let Some(tokens) = prompt_tokens {
        LLM_TOKENS_TOTAL.increment_counter(
            vec![
                ("provider", provider.to_string()),
                ("model", model.to_string()),
                ("direction", "prompt".to_string()),
            ],
            tokens,
        );
    }
    if let Some(tokens) = completion_tokens {
        LLM_TOKENS_TOTAL.increment_counter(
            vec![
                ("provider", provider.to_string()),
                ("model", model.to_string()),
                ("direction", "completion".to_string()),
            ],
            tokens,
        );
    }
}

pub fn observe_container_run(action: &str, isolation: &str, network_access: bool, status: &str) {
    CONTAINER_RUNS_TOTAL.increment_counter(
        vec![
            ("action", action.to_string()),
            ("isolation", isolation.to_string()),
            (
                "network_access",
                if network_access {
                    "enabled"
                } else {
                    "disabled"
                }
                .to_string(),
            ),
            ("status", status.to_string()),
        ],
        1,
    );
}

pub fn observe_container_lifecycle(
    action: &str,
    phase: &str,
    isolation: &str,
    network_access: bool,
    status: &str,
    duration: Duration,
) {
    CONTAINER_LIFECYCLE_DURATION_SECONDS.observe_histogram(
        vec![
            ("action", action.to_string()),
            ("phase", phase.to_string()),
            ("isolation", isolation.to_string()),
            (
                "network_access",
                if network_access {
                    "enabled"
                } else {
                    "disabled"
                }
                .to_string(),
            ),
            ("status", status.to_string()),
        ],
        duration.as_secs_f64(),
        &CONTAINER_DURATION_BUCKETS,
    );
}

pub fn record_container_sweeper_run(status: &str, removed: u64) {
    CONTAINER_SWEEPER_RUNS_TOTAL.increment_counter(vec![("status", status.to_string())], 1);
    if removed > 0 {
        CONTAINER_SWEEPER_REMOVED_TOTAL.increment_counter(Vec::new(), removed);
    }
}

pub fn set_active_containers(count: usize) {
    ACTIVE_CONTAINERS_GAUGE.set_gauge(Vec::new(), count as f64);
}

pub fn record_background_task_panic(task: &str) {
    BACKGROUND_TASK_PANICS_TOTAL.increment_counter(vec![("task", task.to_string())], 1);
}

pub fn render_prometheus(extra_metrics: &[String]) -> String {
    let mut out = String::new();
    render_counter_family(
        &mut out,
        "agentark_http_requests_total",
        &format!(
            "Total HTTP requests handled by {}.",
            crate::branding::PRODUCT_NAME
        ),
        &HTTP_REQUESTS_TOTAL,
    );
    render_histogram_family(
        &mut out,
        "agentark_http_request_duration_seconds",
        "HTTP request latency in seconds.",
        &HTTP_REQUEST_DURATION_SECONDS,
        &HTTP_DURATION_BUCKETS,
    );
    render_counter_family(
        &mut out,
        "agentark_llm_calls_total",
        &format!(
            "Total LLM calls handled by {}.",
            crate::branding::PRODUCT_NAME
        ),
        &LLM_CALLS_TOTAL,
    );
    render_histogram_family(
        &mut out,
        "agentark_llm_call_duration_seconds",
        "LLM call latency in seconds.",
        &LLM_CALL_DURATION_SECONDS,
        &LLM_DURATION_BUCKETS,
    );
    render_counter_family(
        &mut out,
        "agentark_llm_tokens_total",
        "LLM token usage tracked by provider, model, and direction.",
        &LLM_TOKENS_TOTAL,
    );
    render_counter_family(
        &mut out,
        "agentark_container_runs_total",
        "Total sandbox container executions.",
        &CONTAINER_RUNS_TOTAL,
    );
    render_histogram_family(
        &mut out,
        "agentark_container_lifecycle_duration_seconds",
        "Sandbox container lifecycle phase duration in seconds.",
        &CONTAINER_LIFECYCLE_DURATION_SECONDS,
        &CONTAINER_DURATION_BUCKETS,
    );
    render_counter_family(
        &mut out,
        "agentark_container_sweeper_runs_total",
        "Total orphan container sweeper runs.",
        &CONTAINER_SWEEPER_RUNS_TOTAL,
    );
    render_counter_family(
        &mut out,
        "agentark_container_sweeper_removed_total",
        "Total orphan sandbox containers removed by the sweeper.",
        &CONTAINER_SWEEPER_REMOVED_TOTAL,
    );
    render_gauge_family(
        &mut out,
        "agentark_active_containers",
        "Currently tracked active sandbox containers.",
        &ACTIVE_CONTAINERS_GAUGE,
    );
    render_counter_family(
        &mut out,
        "agentark_background_task_panics_total",
        "Total caught panics in background tasks.",
        &BACKGROUND_TASK_PANICS_TOTAL,
    );
    for line in extra_metrics {
        out.push_str(line);
        if !line.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

fn series_key(labels: Vec<(&str, String)>) -> SeriesKey {
    let mut labels = labels
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect::<Vec<_>>();
    labels.sort_by(|left, right| left.0.cmp(&right.0));
    SeriesKey { labels }
}

fn render_counter_family(output: &mut String, name: &str, help: &str, store: &MetricStore) {
    output.push_str(&format!("# HELP {} {}\n", name, help));
    output.push_str(&format!("# TYPE {} counter\n", name));
    let counters = store.counters.lock().expect("metrics counter render lock");
    let mut rows = counters.iter().collect::<Vec<_>>();
    rows.sort_by(|left, right| left.0.cmp(right.0));
    for (key, value) in rows {
        output.push_str(&format!(
            "{}{} {}\n",
            name,
            format_labels(&key.labels),
            value
        ));
    }
}

fn render_gauge_family(output: &mut String, name: &str, help: &str, store: &MetricStore) {
    output.push_str(&format!("# HELP {} {}\n", name, help));
    output.push_str(&format!("# TYPE {} gauge\n", name));
    let gauges = store.gauges.lock().expect("metrics gauge render lock");
    let mut rows = gauges.iter().collect::<Vec<_>>();
    rows.sort_by(|left, right| left.0.cmp(right.0));
    for (key, value) in rows {
        output.push_str(&format!(
            "{}{} {:.6}\n",
            name,
            format_labels(&key.labels),
            value
        ));
    }
}

fn render_histogram_family(
    output: &mut String,
    name: &str,
    help: &str,
    store: &MetricStore,
    bucket_bounds: &[f64],
) {
    output.push_str(&format!("# HELP {} {}\n", name, help));
    output.push_str(&format!("# TYPE {} histogram\n", name));
    let histograms = store
        .histograms
        .lock()
        .expect("metrics histogram render lock");
    let mut rows = histograms.iter().collect::<Vec<_>>();
    rows.sort_by(|left, right| left.0.cmp(right.0));
    for (key, histogram) in rows {
        for (idx, upper_bound) in bucket_bounds.iter().enumerate() {
            let mut labels = key.labels.clone();
            labels.push(("le".to_string(), format_bucket(*upper_bound)));
            labels.sort_by(|left, right| left.0.cmp(&right.0));
            output.push_str(&format!(
                "{}_bucket{} {}\n",
                name,
                format_labels(&labels),
                histogram.buckets[idx]
            ));
        }
        let mut inf_labels = key.labels.clone();
        inf_labels.push(("le".to_string(), "+Inf".to_string()));
        inf_labels.sort_by(|left, right| left.0.cmp(&right.0));
        output.push_str(&format!(
            "{}_bucket{} {}\n",
            name,
            format_labels(&inf_labels),
            histogram.count
        ));
        output.push_str(&format!(
            "{}_sum{} {:.6}\n",
            name,
            format_labels(&key.labels),
            histogram.sum
        ));
        output.push_str(&format!(
            "{}_count{} {}\n",
            name,
            format_labels(&key.labels),
            histogram.count
        ));
    }
}

fn format_labels(labels: &[(String, String)]) -> String {
    if labels.is_empty() {
        return String::new();
    }
    let body = labels
        .iter()
        .map(|(key, value)| format!(r#"{}="{}""#, key, escape_label_value(value)))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{}}}", body)
}

fn escape_label_value(value: &str) -> String {
    value
        .replace('\\', r"\\")
        .replace('\n', r"\n")
        .replace('"', r#"\""#)
}

fn format_bucket(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{:.0}", value)
    } else {
        format!("{:.3}", value)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        observe_container_run, observe_http_request, record_background_task_panic,
        render_prometheus,
    };

    #[test]
    fn render_prometheus_includes_observed_metric_families() {
        observe_http_request(
            "TEST_METRICS_METHOD",
            "/test-metrics-render",
            204,
            std::time::Duration::from_millis(12),
        );
        observe_container_run("test_metrics_action", "standard", false, "ok");
        record_background_task_panic("test_metrics_background_task");

        let rendered = render_prometheus(&["agentark_test_metric 1".to_string()]);

        assert!(rendered.contains("# HELP agentark_http_requests_total"));
        assert!(rendered.contains("TEST_METRICS_METHOD"));
        assert!(rendered.contains("/test-metrics-render"));
        assert!(rendered.contains("# HELP agentark_container_runs_total"));
        assert!(rendered.contains("test_metrics_action"));
        assert!(rendered.contains("# HELP agentark_background_task_panics_total"));
        assert!(rendered.contains("test_metrics_background_task"));
        assert!(rendered.contains("agentark_test_metric 1"));
    }
}
