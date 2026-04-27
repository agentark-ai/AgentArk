//! Research Action - Deep web research and information synthesis
//! Provides comprehensive research capabilities by:
//! 1. Searching multiple sources
//! 2. Fetching and extracting content from URLs
//! 3. Synthesizing information into a coherent report

use anyhow::{Result, anyhow};
use chrono::{DateTime, Datelike, Duration as ChronoDuration, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;

use super::search::{SearchConfig, SearchResult};

/// Research request parameters
#[derive(Debug, Clone, Deserialize)]
pub struct ResearchArgs {
    /// The topic or question to research
    pub query: String,
    /// Maximum number of sources to examine
    #[serde(default = "default_max_sources")]
    pub max_sources: usize,
    /// Whether to include source URLs in output
    #[serde(default = "default_include_sources", rename = "include_sources")]
    pub _include_sources: bool,
    /// Preferred search backend
    pub backend: Option<String>,
    /// Depth of research (quick, standard, deep)
    #[serde(default)]
    pub depth: ResearchDepth,
    /// Minimum number of primary-source-like results to include when available
    #[serde(default = "default_min_primary_sources")]
    pub min_primary_sources: usize,
    /// Optional source freshness window in days for ranking and diversity
    #[serde(default)]
    pub freshness_window_days: Option<u32>,
    /// Extra follow-up search rounds for unresolved gaps or contradictions
    #[serde(default = "default_followup_rounds")]
    pub followup_rounds: usize,
}

fn default_max_sources() -> usize {
    5
}

fn default_deep_max_sources() -> usize {
    12
}

fn default_include_sources() -> bool {
    true
}

fn default_min_primary_sources() -> usize {
    0
}

fn default_followup_rounds() -> usize {
    0
}

const MAX_RESEARCH_FETCH_REDIRECTS: usize = 3;
const MAX_RESEARCH_FETCH_BYTES: usize = 2 * 1024 * 1024;

/// Research depth level
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ResearchDepth {
    /// Quick search - just search results
    Quick,
    /// Standard - search + fetch top results
    #[default]
    Standard,
    /// Deep - multiple searches + comprehensive fetching
    Deep,
}

/// Research result
#[derive(Debug, Clone, Serialize)]
pub struct ResearchResult {
    /// The original query
    pub query: String,
    /// Executive summary
    pub summary: String,
    /// Key findings
    pub findings: Vec<Finding>,
    /// Clustered findings prioritized for final synthesis
    pub key_findings: Vec<Finding>,
    /// Gaps or unresolved questions that still need confirmation
    pub open_questions: Vec<String>,
    /// Explicit contradictions or unresolved source disagreements
    pub contradictions: Vec<String>,
    /// Sources used
    pub sources: Vec<Source>,
    /// Related topics for further research
    pub related_topics: Vec<String>,
}

/// A single finding from research
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// The finding content
    pub content: String,
    /// Confidence score (0.0-1.0)
    pub confidence: f32,
    /// Source index
    pub source_index: usize,
    /// Supporting source indices for corroborated claims
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub supporting_source_indices: Vec<usize>,
}

/// A source used in research
#[derive(Debug, Clone, Serialize)]
pub struct Source {
    /// Source title
    pub title: String,
    /// Source URL
    pub url: String,
    /// Brief description/snippet
    pub description: String,
    /// Reliability score (0.0-1.0)
    pub reliability: f32,
    /// Published date when the search backend could infer one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_date: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ResearchQueryCategory {
    Primary,
    Recent,
    Comparison,
    Risks,
    General,
}

#[derive(Debug, Clone)]
struct ResearchQuery {
    category: ResearchQueryCategory,
    text: String,
}

#[derive(Debug, Clone)]
struct RankedResearchResult {
    result: SearchResult,
    categories: HashSet<ResearchQueryCategory>,
    score: f32,
}

#[derive(Debug, Clone, Copy, Default)]
struct ResearchCoverage {
    primary_sources: usize,
    recent_sources: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResearchProgressUpdate {
    pub phase: String,
    pub label: String,
    pub detail: String,
    pub status: String,
    pub elapsed_secs: u64,
    pub stream_key: String,
}

#[derive(Clone)]
pub struct ResearchProgressReporter {
    tx: UnboundedSender<ResearchProgressUpdate>,
    started_at: Instant,
}

impl ResearchProgressReporter {
    pub fn emit(
        &self,
        phase: &str,
        label: &str,
        detail: impl Into<String>,
        status: &str,
        stream_key: &str,
    ) {
        let detail = detail.into().trim().to_string();
        if detail.is_empty() {
            return;
        }
        let _ = self.tx.send(ResearchProgressUpdate {
            phase: phase.trim().to_string(),
            label: label.trim().to_string(),
            detail,
            status: status.trim().to_string(),
            elapsed_secs: self.started_at.elapsed().as_secs(),
            stream_key: stream_key.trim().to_string(),
        });
    }
}

fn summarize_progress_text(value: &str, max_chars: usize) -> String {
    let normalized = value
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    let mut shortened = normalized
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    shortened.push_str("...");
    shortened
}

fn backend_display_name(name: &str) -> &'static str {
    match name.trim().to_ascii_lowercase().as_str() {
        "serper" => "Serper",
        "brave" | "brave_api" => "Brave API",
        "exa" => "Exa",
        "tavily" => "Tavily",
        "perplexity" => "Perplexity",
        "firecrawl" => "Firecrawl",
        "searxng" => "SearXNG",
        "duckduckgo" => "DuckDuckGo",
        "bing_rss" => "Bing RSS",
        "playwright" => "Playwright",
        "lightpanda" => "Lightpanda",
        _ => "search backend",
    }
}

fn source_label(result: &SearchResult) -> String {
    let title = summarize_progress_text(&result.title, 72);
    if !title.is_empty() {
        return title;
    }
    summarize_progress_text(&result.url, 72)
}

fn source_host_label(url: &str) -> String {
    let trimmed = url.trim();
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let host = without_scheme
        .split('/')
        .next()
        .unwrap_or(without_scheme)
        .trim();
    summarize_progress_text(host, 48)
}

fn summarize_error(err: &anyhow::Error) -> String {
    summarize_progress_text(&err.to_string(), 120)
}

fn research_result_failure_reason(result: &ResearchResult) -> Option<String> {
    let has_sources = result.sources.iter().any(|source| {
        !source.url.trim().is_empty()
            || !source.title.trim().is_empty()
            || !source.description.trim().is_empty()
    });
    let primary_findings = if result.key_findings.is_empty() {
        &result.findings
    } else {
        &result.key_findings
    };
    let has_findings = primary_findings
        .iter()
        .any(|finding| !finding.content.trim().is_empty());
    let summary_lower = result.summary.trim().to_ascii_lowercase();
    let placeholder_summary = summary_lower.starts_with("no relevant information found for:");

    if !has_sources {
        Some("No usable sources were found across the available search backends.".to_string())
    } else if !has_findings || placeholder_summary {
        Some(
            "The search returned pages, but none produced enough usable evidence to draft a research report."
                .to_string(),
        )
    } else {
        None
    }
}

fn ensure_research_result_has_evidence(
    result: &ResearchResult,
    progress: Option<&ResearchProgressReporter>,
) -> Result<()> {
    let Some(reason) = research_result_failure_reason(result) else {
        return Ok(());
    };
    if let Some(progress) = progress {
        progress.emit(
            "synthesis",
            "Research failed",
            reason.clone(),
            "failed",
            "phase-status:research:synthesis",
        );
    }
    Err(anyhow!(
        "Unable to complete research because {}",
        reason.trim_end_matches('.').to_ascii_lowercase()
    ))
}

/// Research client
pub struct ResearchClient {
    search_config: SearchConfig,
    http_client: reqwest::Client,
}

impl ResearchClient {
    pub fn new(search_config: SearchConfig) -> Self {
        let http_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .expect("Failed to create HTTP client");

        Self {
            search_config,
            http_client,
        }
    }

    pub async fn research_with_progress(
        &self,
        args: &ResearchArgs,
        progress: Option<&ResearchProgressReporter>,
    ) -> Result<ResearchResult> {
        match args.depth {
            ResearchDepth::Quick => self.quick_research(args, progress).await,
            ResearchDepth::Standard => self.standard_research(args, progress).await,
            ResearchDepth::Deep => self.deep_research(args, progress).await,
        }
    }

    fn effective_max_sources(&self, args: &ResearchArgs) -> usize {
        match args.depth {
            ResearchDepth::Deep if args.max_sources == default_max_sources() => {
                default_deep_max_sources()
            }
            _ => args.max_sources.max(1),
        }
    }

    fn effective_min_primary_sources(&self, args: &ResearchArgs) -> usize {
        if args.min_primary_sources > 0 {
            return args.min_primary_sources;
        }
        if matches!(args.depth, ResearchDepth::Deep) {
            return self.effective_max_sources(args).min(2);
        }
        0
    }

    fn effective_followup_rounds(&self, args: &ResearchArgs) -> usize {
        if args.followup_rounds > 0 {
            return args.followup_rounds;
        }
        if matches!(args.depth, ResearchDepth::Deep) {
            return 2;
        }
        0
    }

    fn effective_freshness_window_days(&self, args: &ResearchArgs, query: &str) -> Option<i64> {
        if let Some(days) = args.freshness_window_days {
            return Some(days.max(1) as i64);
        }
        if matches!(args.depth, ResearchDepth::Deep) && !self.query_targets_historical_period(query)
        {
            return Some(365);
        }
        None
    }

    fn query_targets_historical_period(&self, query: &str) -> bool {
        let current_year = Utc::now().year();
        let years = query
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .filter_map(|token| {
                let token = token.trim();
                if token.len() == 4 {
                    token.parse::<i32>().ok()
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        !years.is_empty() && years.iter().all(|year| *year <= current_year - 2)
    }

    fn build_source(&self, result: &SearchResult) -> Source {
        Source {
            title: result.title.clone(),
            url: result.url.clone(),
            description: result.snippet.clone(),
            reliability: self.estimate_reliability(&result.url),
            published_date: result.published_date.clone(),
        }
    }

    /// Quick research - just search results
    async fn quick_research(
        &self,
        args: &ResearchArgs,
        progress: Option<&ResearchProgressReporter>,
    ) -> Result<ResearchResult> {
        let max_sources = self.effective_max_sources(args);
        if let Some(progress) = progress {
            progress.emit(
                "planning",
                "Preparing research",
                format!(
                    "Running a quick source scan for {}.",
                    summarize_progress_text(&args.query, 96)
                ),
                "running",
                "phase-status:research:planning",
            );
            progress.emit(
                "planning",
                "Preparing research",
                "Quick research scope confirmed.",
                "completed",
                "phase-status:research:planning",
            );
            progress.emit(
                "searching",
                "Searching sources",
                format!(
                    "Looking for up to {} source{} on {}.",
                    max_sources,
                    if max_sources == 1 { "" } else { "s" },
                    summarize_progress_text(&args.query, 96)
                ),
                "running",
                "phase-status:research:searching",
            );
        }
        let search_results = self
            .search(
                &args.query,
                max_sources,
                &args.backend,
                progress,
                "phase-status:research:searching",
            )
            .await?;
        if let Some(progress) = progress {
            progress.emit(
                "searching",
                "Searching sources",
                format!(
                    "Found {} candidate source{}.",
                    search_results.len(),
                    if search_results.len() == 1 { "" } else { "s" }
                ),
                "completed",
                "phase-status:research:searching",
            );
        }

        let findings: Vec<Finding> = search_results
            .iter()
            .enumerate()
            .map(|(i, r)| Finding {
                content: r.snippet.clone(),
                confidence: 0.7,
                source_index: i,
                supporting_source_indices: vec![i],
            })
            .collect();

        let sources: Vec<Source> = search_results
            .iter()
            .map(|r| self.build_source(r))
            .collect();

        let summary = self.generate_summary(&args.query, &findings);
        let result = ResearchResult {
            query: args.query.clone(),
            summary,
            key_findings: findings.clone(),
            findings,
            open_questions: Vec::new(),
            contradictions: Vec::new(),
            sources,
            related_topics: self.extract_related_topics(&search_results),
        };
        ensure_research_result_has_evidence(&result, progress)?;
        if let Some(progress) = progress {
            progress.emit(
                "synthesis",
                "Research complete",
                format!(
                    "Quick research finished with {} source{} and {} finding{}.",
                    result.sources.len(),
                    if result.sources.len() == 1 { "" } else { "s" },
                    result.findings.len(),
                    if result.findings.len() == 1 { "" } else { "s" }
                ),
                "completed",
                "phase-status:research:synthesis",
            );
        }

        Ok(result)
    }

    /// Standard research - search + fetch content
    async fn standard_research(
        &self,
        args: &ResearchArgs,
        progress: Option<&ResearchProgressReporter>,
    ) -> Result<ResearchResult> {
        let query_terms = self.normalized_query_terms(&args.query);
        let max_sources = self.effective_max_sources(args);
        if let Some(progress) = progress {
            progress.emit(
                "planning",
                "Preparing research",
                format!(
                    "Setting up a standard research pass for {}.",
                    summarize_progress_text(&args.query, 96)
                ),
                "running",
                "phase-status:research:planning",
            );
            progress.emit(
                "planning",
                "Preparing research",
                "Research scope is ready.",
                "completed",
                "phase-status:research:planning",
            );
            progress.emit(
                "searching",
                "Searching sources",
                format!(
                    "Looking for up to {} source{}.",
                    max_sources,
                    if max_sources == 1 { "" } else { "s" }
                ),
                "running",
                "phase-status:research:searching",
            );
        }
        let search_results = self
            .search(
                &args.query,
                max_sources,
                &args.backend,
                progress,
                "phase-status:research:searching",
            )
            .await?;
        if let Some(progress) = progress {
            progress.emit(
                "searching",
                "Searching sources",
                format!(
                    "Selected {} candidate source{} for reading.",
                    search_results.len(),
                    if search_results.len() == 1 { "" } else { "s" }
                ),
                "completed",
                "phase-status:research:searching",
            );
            progress.emit(
                "reading",
                "Reading sources",
                "Opening the strongest sources and extracting evidence.",
                "running",
                "phase-status:research:reading",
            );
        }

        let mut sources: Vec<Source> = Vec::new();
        let mut findings: Vec<Finding> = Vec::new();
        let selected_results = search_results
            .iter()
            .take(max_sources)
            .cloned()
            .collect::<Vec<_>>();

        self.read_search_results_into_evidence(
            &selected_results,
            &args.query,
            &query_terms,
            &mut sources,
            &mut findings,
            progress,
            "phase-status:research:reading",
        )
        .await;

        let summary = self.generate_summary(&args.query, &findings);
        let result = ResearchResult {
            query: args.query.clone(),
            summary,
            key_findings: findings.clone(),
            findings,
            open_questions: Vec::new(),
            contradictions: Vec::new(),
            sources,
            related_topics: self.extract_related_topics(&search_results),
        };
        ensure_research_result_has_evidence(&result, progress)?;
        if let Some(progress) = progress {
            progress.emit(
                "reading",
                "Reading sources",
                format!(
                    "Read {} source{} and captured {} finding{}.",
                    result.sources.len(),
                    if result.sources.len() == 1 { "" } else { "s" },
                    result.findings.len(),
                    if result.findings.len() == 1 { "" } else { "s" }
                ),
                "completed",
                "phase-status:research:reading",
            );
            progress.emit(
                "synthesis",
                "Synthesizing report",
                format!(
                    "Writing the standard research summary from {} finding{}.",
                    result.findings.len(),
                    if result.findings.len() == 1 { "" } else { "s" }
                ),
                "running",
                "phase-status:research:synthesis",
            );
            progress.emit(
                "synthesis",
                "Research complete",
                format!(
                    "Standard research finished with {} source{}.",
                    result.sources.len(),
                    if result.sources.len() == 1 { "" } else { "s" }
                ),
                "completed",
                "phase-status:research:synthesis",
            );
        }

        Ok(result)
    }

    /// Deep research - multiple searches + comprehensive analysis
    async fn deep_research(
        &self,
        args: &ResearchArgs,
        progress: Option<&ResearchProgressReporter>,
    ) -> Result<ResearchResult> {
        let query_terms = self.normalized_query_terms(&args.query);
        let max_sources = self.effective_max_sources(args);
        let min_primary_sources = self.effective_min_primary_sources(args);
        let followup_rounds = self.effective_followup_rounds(args);
        let freshness_window_days = self.effective_freshness_window_days(args, &args.query);
        let prefer_primary_sources = min_primary_sources > 0;
        if let Some(progress) = progress {
            progress.emit(
                "planning",
                "Preparing research",
                format!(
                    "Breaking {} into a deeper evidence-gathering plan.",
                    summarize_progress_text(&args.query, 96)
                ),
                "running",
                "phase-status:research:planning",
            );
        }
        let queries = self.generate_research_queries(&args.query, prefer_primary_sources);
        if let Some(progress) = progress {
            progress.emit(
                "planning",
                "Preparing research",
                format!(
                    "Prepared {} search angle{} for this run.",
                    queries.len(),
                    if queries.len() == 1 { "" } else { "s" }
                ),
                "completed",
                "phase-status:research:planning",
            );
            progress.emit(
                "searching",
                "Searching sources",
                "Scanning primary, recent, comparison, and risk angles.",
                "running",
                "phase-status:research:searching",
            );
        }
        let mut used_urls = HashSet::new();
        let results_by_url = self
            .collect_ranked_results(
                &args.query,
                &queries,
                4,
                &args.backend,
                progress,
                "phase-status:research:searching",
                prefer_primary_sources,
                freshness_window_days,
                &used_urls,
                true,
            )
            .await?;
        if let Some(progress) = progress {
            progress.emit(
                "searching",
                "Searching sources",
                format!(
                    "Collected {} unique candidate source{} across {} angle{}.",
                    results_by_url.len(),
                    if results_by_url.len() == 1 { "" } else { "s" },
                    queries.len(),
                    if queries.len() == 1 { "" } else { "s" }
                ),
                "completed",
                "phase-status:research:searching",
            );
            progress.emit(
                "ranking",
                "Selecting sources",
                "Scoring diverse sources and keeping the strongest set.",
                "running",
                "phase-status:research:ranking",
            );
        }

        let ranked_results = self.select_diverse_results(
            results_by_url.into_values().collect(),
            max_sources,
            prefer_primary_sources,
            min_primary_sources,
            freshness_window_days,
        );
        if let Some(progress) = progress {
            progress.emit(
                "ranking",
                "Selecting sources",
                format!(
                    "Selected {} source{} for closer reading.",
                    ranked_results.len(),
                    if ranked_results.len() == 1 { "" } else { "s" }
                ),
                "completed",
                "phase-status:research:ranking",
            );
            progress.emit(
                "reading",
                "Reading sources",
                "Opening the selected sources and extracting evidence.",
                "running",
                "phase-status:research:reading",
            );
        }
        let mut all_results = ranked_results
            .into_iter()
            .map(|entry| entry.result)
            .collect::<Vec<_>>();
        for result in &all_results {
            used_urls.insert(result.url.trim().to_lowercase());
        }

        let mut sources: Vec<Source> = Vec::new();
        let mut findings: Vec<Finding> = Vec::new();

        self.read_search_results_into_evidence(
            &all_results,
            &args.query,
            &query_terms,
            &mut sources,
            &mut findings,
            progress,
            "phase-status:research:reading",
        )
        .await;

        let mut findings = self.deduplicate_findings(findings);
        let mut key_findings = self.cluster_findings(&findings);
        let mut contradictions = self.detect_contradictions(&findings, &sources);
        let mut open_questions =
            self.derive_open_questions(&args.query, &sources, &findings, &contradictions);

        for round in 0..followup_rounds {
            let remaining_slots = max_sources.saturating_sub(sources.len());
            let coverage = self.research_coverage(&sources, freshness_window_days);
            let primary_gap = coverage.primary_sources < min_primary_sources;
            let freshness_gap = freshness_window_days.is_some() && coverage.recent_sources == 0;
            let unresolved = !open_questions.is_empty() || !contradictions.is_empty();
            if remaining_slots == 0 || (!primary_gap && !freshness_gap && !unresolved) {
                break;
            }

            let followup_queries = self.generate_followup_queries(
                &args.query,
                &open_questions,
                &contradictions,
                primary_gap,
                freshness_gap,
                round,
            );
            if followup_queries.is_empty() {
                break;
            }

            if let Some(progress) = progress {
                progress.emit(
                    "searching",
                    "Searching sources",
                    format!(
                        "Follow-up round {}/{}: resolving remaining evidence gaps.",
                        round + 1,
                        followup_rounds
                    ),
                    "running",
                    "phase-status:research:searching",
                );
            }

            let followup_results_by_url = self
                .collect_ranked_results(
                    &args.query,
                    &followup_queries,
                    3,
                    &args.backend,
                    progress,
                    "phase-status:research:searching",
                    prefer_primary_sources,
                    freshness_window_days,
                    &used_urls,
                    false,
                )
                .await?;
            if followup_results_by_url.is_empty() {
                break;
            }

            let followup_results = self.select_diverse_results(
                followup_results_by_url.into_values().collect(),
                remaining_slots.min(4),
                prefer_primary_sources,
                min_primary_sources.saturating_sub(coverage.primary_sources),
                if coverage.recent_sources > 0 {
                    None
                } else {
                    freshness_window_days
                },
            );
            if followup_results.is_empty() {
                break;
            }

            let new_results = followup_results
                .into_iter()
                .map(|entry| entry.result)
                .collect::<Vec<_>>();
            for result in &new_results {
                used_urls.insert(result.url.trim().to_lowercase());
            }
            all_results.extend(new_results.iter().cloned());

            self.read_search_results_into_evidence(
                &new_results,
                &args.query,
                &query_terms,
                &mut sources,
                &mut findings,
                progress,
                "phase-status:research:reading",
            )
            .await;

            findings = self.deduplicate_findings(findings);
            key_findings = self.cluster_findings(&findings);
            contradictions = self.detect_contradictions(&findings, &sources);
            open_questions =
                self.derive_open_questions(&args.query, &sources, &findings, &contradictions);
        }

        if let Some(progress) = progress {
            progress.emit(
                "reading",
                "Reading sources",
                format!(
                    "Read {} source{} and captured {} raw finding{}.",
                    sources.len(),
                    if sources.len() == 1 { "" } else { "s" },
                    findings.len(),
                    if findings.len() == 1 { "" } else { "s" }
                ),
                "completed",
                "phase-status:research:reading",
            );
            progress.emit(
                "synthesis",
                "Synthesizing report",
                format!(
                    "Weighing {} raw finding{} across {} source{}.",
                    findings.len(),
                    if findings.len() == 1 { "" } else { "s" },
                    sources.len(),
                    if sources.len() == 1 { "" } else { "s" }
                ),
                "running",
                "phase-status:research:synthesis",
            );
        }

        let summary = self.generate_comprehensive_summary(
            &args.query,
            &key_findings,
            &sources,
            &open_questions,
            &contradictions,
        );
        let result = ResearchResult {
            query: args.query.clone(),
            summary,
            findings,
            key_findings,
            open_questions,
            contradictions,
            sources,
            related_topics: self.extract_related_topics(&all_results),
        };
        ensure_research_result_has_evidence(&result, progress)?;
        if let Some(progress) = progress {
            progress.emit(
                "synthesis",
                "Research complete",
                format!(
                    "Finished with {} source{}, {} key finding{}, {} open question{}, and {} contradiction{} to verify.",
                    result.sources.len(),
                    if result.sources.len() == 1 { "" } else { "s" },
                    result.key_findings.len(),
                    if result.key_findings.len() == 1 { "" } else { "s" },
                    result.open_questions.len(),
                    if result.open_questions.len() == 1 { "" } else { "s" },
                    result.contradictions.len(),
                    if result.contradictions.len() == 1 { "" } else { "s" }
                ),
                "completed",
                "phase-status:research:synthesis",
            );
        }

        Ok(result)
    }

    /// Search using configured backend with fallback chain
    async fn search(
        &self,
        query: &str,
        num_results: usize,
        backend_preference: &Option<String>,
        progress: Option<&ResearchProgressReporter>,
        stream_key: &str,
    ) -> Result<Vec<SearchResult>> {
        if let Some(progress) = progress {
            let label = backend_preference
                .as_deref()
                .map(backend_display_name)
                .unwrap_or("search providers");
            progress.emit(
                "searching",
                "Searching sources",
                format!(
                    "Trying {} for {}.",
                    label,
                    summarize_progress_text(query, 96)
                ),
                "running",
                stream_key,
            );
        }
        let response = super::search::search_with_config(
            query,
            num_results,
            backend_preference.as_deref(),
            &self.search_config,
        )
        .await;
        match response {
            Ok(response) => {
                let filtered_results =
                    self.filter_search_results_for_query(query, response.results);
                if let Some(progress) = progress {
                    progress.emit(
                        "searching",
                        "Searching sources",
                        format!(
                            "{} returned {} result{} for {}.",
                            backend_display_name(&response.backend),
                            filtered_results.len(),
                            if filtered_results.len() == 1 { "" } else { "s" },
                            summarize_progress_text(query, 96)
                        ),
                        "running",
                        stream_key,
                    );
                }
                return Ok(filtered_results);
            }
            Err(error) => {
                if let Some(progress) = progress {
                    progress.emit(
                        "searching",
                        "Searching sources",
                        format!(
                            "Search failed for {} ({}).",
                            summarize_progress_text(query, 96),
                            summarize_error(&error)
                        ),
                        "running",
                        stream_key,
                    );
                }
                return Err(error);
            }
        }
    }

    async fn collect_ranked_results(
        &self,
        query: &str,
        queries: &[ResearchQuery],
        results_per_query: usize,
        backend_preference: &Option<String>,
        progress: Option<&ResearchProgressReporter>,
        stream_key: &str,
        prefer_primary_sources: bool,
        freshness_window_days: Option<i64>,
        already_used_urls: &HashSet<String>,
        fail_on_total_failure: bool,
    ) -> Result<HashMap<String, RankedResearchResult>> {
        let mut results_by_url: HashMap<String, RankedResearchResult> = HashMap::new();
        let mut search_errors = Vec::new();

        for (query_index, search_query) in queries.iter().enumerate() {
            if let Some(progress) = progress {
                progress.emit(
                    "searching",
                    "Searching sources",
                    format!(
                        "Angle {}/{}: {}.",
                        query_index + 1,
                        queries.len(),
                        summarize_progress_text(&search_query.text, 104)
                    ),
                    "running",
                    stream_key,
                );
            }
            match self
                .search(
                    &search_query.text,
                    results_per_query,
                    backend_preference,
                    progress,
                    stream_key,
                )
                .await
            {
                Ok(results) => {
                    for result in results {
                        let normalized_url = result.url.trim().to_lowercase();
                        if normalized_url.is_empty() || already_used_urls.contains(&normalized_url)
                        {
                            continue;
                        }
                        let score = self.research_rank_score(
                            &result,
                            query,
                            prefer_primary_sources,
                            search_query.category,
                            freshness_window_days,
                        );
                        if score <= 0.0 {
                            continue;
                        }
                        results_by_url
                            .entry(normalized_url)
                            .and_modify(|existing| {
                                existing.categories.insert(search_query.category);
                                if score > existing.score {
                                    existing.result = result.clone();
                                    existing.score = score;
                                }
                            })
                            .or_insert_with(|| RankedResearchResult {
                                result,
                                categories: HashSet::from([search_query.category]),
                                score,
                            });
                    }
                }
                Err(error) => search_errors.push(error.to_string()),
            }
        }

        if results_by_url.is_empty() && fail_on_total_failure && !search_errors.is_empty() {
            if let Some(provider_setup_error) = search_errors
                .iter()
                .find(|error| error.contains(super::search::SEARCH_PROVIDER_SETUP_REQUIRED_MESSAGE))
            {
                return Err(anyhow!(provider_setup_error.clone()));
            }
            return Err(anyhow!(
                "Unable to complete research because all search angles failed: {}",
                search_errors.join(" | ")
            ));
        }

        Ok(results_by_url)
    }

    async fn read_search_results_into_evidence(
        &self,
        results: &[SearchResult],
        query: &str,
        query_terms: &[String],
        sources: &mut Vec<Source>,
        findings: &mut Vec<Finding>,
        progress: Option<&ResearchProgressReporter>,
        stream_key: &str,
    ) {
        let total_after_batch = sources.len() + results.len();
        for result in results {
            let source_index = sources.len();
            let source = self.build_source(result);
            let reliability = source.reliability;
            sources.push(source);

            if let Some(progress) = progress {
                progress.emit(
                    "reading",
                    "Reading sources",
                    format!(
                        "Source {}/{}: reading {}.",
                        source_index + 1,
                        total_after_batch,
                        source_label(result)
                    ),
                    "running",
                    stream_key,
                );
            }
            match self.fetch_content(&result.url, progress, stream_key).await {
                Ok(content) => {
                    let key_points = self.extract_key_points(&content, query);
                    if key_points.is_empty() {
                        if let Some(finding) = self.build_snippet_finding(
                            &result.snippet,
                            query_terms,
                            source_index,
                            (0.58 + reliability * 0.25 + self.search_result_recency_bonus(result))
                                .min(0.92),
                        ) {
                            findings.push(finding);
                        }
                    } else {
                        for point in key_points {
                            findings.push(Finding {
                                content: point,
                                confidence: (0.62
                                    + reliability * 0.28
                                    + self.search_result_recency_bonus(result))
                                .min(0.96),
                                source_index,
                                supporting_source_indices: vec![source_index],
                            });
                        }
                    }
                }
                Err(err) => {
                    if let Some(progress) = progress {
                        progress.emit(
                            "reading",
                            "Reading sources",
                            format!(
                                "Source {}/{}: using the search snippet after the page read failed ({}).",
                                source_index + 1,
                                total_after_batch,
                                summarize_error(&err)
                            ),
                            "running",
                            stream_key,
                        );
                    }
                    if let Some(finding) = self.build_snippet_finding(
                        &result.snippet,
                        query_terms,
                        source_index,
                        (0.44 + reliability * 0.2 + self.search_result_recency_bonus(result))
                            .min(0.8),
                    ) {
                        findings.push(finding);
                    }
                }
            }
        }
    }

    /// Fetch content from a URL
    async fn fetch_content(
        &self,
        url: &str,
        progress: Option<&ResearchProgressReporter>,
        stream_key: &str,
    ) -> Result<String> {
        let validated_url = crate::core::net::validate_public_https_url(url).await?;
        if let Some(progress) = progress {
            progress.emit(
                "reading",
                "Reading sources",
                format!("Opening {}.", source_host_label(validated_url.as_str())),
                "running",
                stream_key,
            );
        }
        let (final_url, html) = self
            .fetch_html_with_safe_redirects(validated_url.clone())
            .await?;
        // Fast-path: Lightpanda returns clean markdown. Still wrap it in the
        // untrusted envelope: the remote server is attacker-controllable, and
        // clean markdown can still carry embedded instructions.
        let origin_label = source_host_label(final_url.as_str());
        match crate::integrations::lightpanda::fetch_markdown(final_url.as_str()).await {
            Ok(markdown) => {
                return Ok(crate::security::sanitize_untrusted_output(
                    &format!("web_page:{}", origin_label),
                    &markdown,
                ));
            }
            Err(e) => {
                tracing::debug!(
                    "Lightpanda unavailable for research fetch, falling back to reqwest: {}",
                    e
                );
                if let Some(progress) = progress {
                    progress.emit(
                        "reading",
                        "Reading sources",
                        format!(
                            "Using direct fetch for {} after Lightpanda fallback ({}).",
                            origin_label,
                            summarize_error(&e)
                        ),
                        "running",
                        stream_key,
                    );
                }
            }
        }

        // HTML fallback: run structural neutralization (strip script/style/
        // hidden elements/comments) and wrap.
        Ok(crate::security::sanitize_untrusted_html(
            &format!("web_page:{}", origin_label),
            &html,
        ))
    }

    async fn fetch_html_with_safe_redirects(
        &self,
        initial: reqwest::Url,
    ) -> Result<(reqwest::Url, String)> {
        let mut current = initial;
        for _ in 0..=MAX_RESEARCH_FETCH_REDIRECTS {
            let response = self.http_client.get(current.clone()).send().await?;
            if response.status().is_success() {
                if let Some(length) = response.content_length() {
                    if length > MAX_RESEARCH_FETCH_BYTES as u64 {
                        return Err(anyhow!("Fetched content is too large"));
                    }
                }
                let bytes = response.bytes().await?;
                if bytes.len() > MAX_RESEARCH_FETCH_BYTES {
                    return Err(anyhow!("Fetched content is too large"));
                }
                return Ok((current, String::from_utf8_lossy(&bytes).into_owned()));
            }
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| anyhow!("Redirect missing Location header"))?;
                let next = current
                    .join(location)
                    .map_err(|error| anyhow!("Invalid redirect URL: {}", error))?;
                current = crate::core::net::validate_public_https_url(next.as_str()).await?;
                continue;
            }
            return Err(anyhow!("Failed to fetch URL: {}", response.status()));
        }
        Err(anyhow!(
            "Too many redirects while fetching research content"
        ))
    }

    fn filter_search_results_for_query(
        &self,
        query: &str,
        results: Vec<SearchResult>,
    ) -> Vec<SearchResult> {
        let query_terms = self.normalized_query_terms(query);
        results
            .into_iter()
            .filter_map(|result| self.normalize_search_result(result))
            .filter(|result| {
                query_terms.is_empty()
                    || self.search_result_matches_query_terms(result, &query_terms)
            })
            .collect()
    }

    fn normalize_search_result(&self, mut result: SearchResult) -> Option<SearchResult> {
        let normalized_url = self.normalize_source_url(&result.url)?;
        result.url = normalized_url;
        result.title = result.title.trim().to_string();
        result.snippet = result.snippet.trim().to_string();
        Some(result)
    }

    fn normalize_source_url(&self, url: &str) -> Option<String> {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            return Some(trimmed.to_string());
        }
        if trimmed.starts_with("//") {
            return Some(format!("https:{}", trimmed));
        }
        if trimmed.starts_with('/') {
            return None;
        }

        let host = trimmed.split(['/', '?', '#']).next().unwrap_or("").trim();
        let host_is_domain_like = host.contains('.')
            && !host.contains(' ')
            && host
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '.');
        if host_is_domain_like {
            Some(format!("https://{}", trimmed))
        } else {
            None
        }
    }

    fn parse_published_date(&self, value: Option<&str>) -> Option<DateTime<Utc>> {
        let value = value?.trim();
        if value.is_empty() {
            return None;
        }
        if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
            return Some(parsed.with_timezone(&Utc));
        }
        if let Ok(parsed) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
            return parsed
                .and_hms_opt(0, 0, 0)
                .map(|dt| Utc.from_utc_datetime(&dt));
        }
        for fmt in [
            "%B %d, %Y",
            "%b %d, %Y",
            "%m/%d/%Y",
            "%m/%d/%y",
            "%d/%m/%Y",
            "%d/%m/%y",
        ] {
            if let Ok(parsed) = NaiveDate::parse_from_str(value, fmt) {
                if let Some(dt) = parsed.and_hms_opt(0, 0, 0) {
                    return Some(Utc.from_utc_datetime(&dt));
                }
            }
        }

        let lower = value.to_ascii_lowercase();
        let mut parts = lower.split_whitespace();
        let amount = parts.next().and_then(|token| token.parse::<i64>().ok());
        let unit = parts.next();
        let ago = parts.next();
        if let (Some(amount), Some(unit), Some("ago")) = (amount, unit, ago) {
            let duration = match unit {
                "minute" | "minutes" => Some(ChronoDuration::minutes(amount)),
                "hour" | "hours" => Some(ChronoDuration::hours(amount)),
                "day" | "days" => Some(ChronoDuration::days(amount)),
                "week" | "weeks" => Some(ChronoDuration::weeks(amount)),
                "month" | "months" => Some(ChronoDuration::days(amount * 30)),
                "year" | "years" => Some(ChronoDuration::days(amount * 365)),
                _ => None,
            };
            if let Some(duration) = duration {
                return Some(Utc::now() - duration);
            }
        }

        None
    }

    fn search_result_is_recent(
        &self,
        result: &SearchResult,
        freshness_window_days: Option<i64>,
    ) -> bool {
        let Some(window_days) = freshness_window_days else {
            return false;
        };
        let Some(published_at) = self.parse_published_date(result.published_date.as_deref()) else {
            return false;
        };
        published_at >= Utc::now() - ChronoDuration::days(window_days.max(1))
    }

    fn source_is_recent(&self, source: &Source, freshness_window_days: Option<i64>) -> bool {
        let Some(window_days) = freshness_window_days else {
            return false;
        };
        let Some(published_at) = self.parse_published_date(source.published_date.as_deref()) else {
            return false;
        };
        published_at >= Utc::now() - ChronoDuration::days(window_days.max(1))
    }

    fn search_result_recency_bonus(&self, result: &SearchResult) -> f32 {
        let Some(published_at) = self.parse_published_date(result.published_date.as_deref()) else {
            return 0.0;
        };
        let age_days = (Utc::now() - published_at).num_days().max(0);
        match age_days {
            0..=30 => 0.06,
            31..=90 => 0.04,
            91..=365 => 0.02,
            _ => 0.0,
        }
    }

    fn research_coverage(
        &self,
        sources: &[Source],
        freshness_window_days: Option<i64>,
    ) -> ResearchCoverage {
        ResearchCoverage {
            primary_sources: sources
                .iter()
                .filter(|source| self.looks_like_primary_source(&source.url))
                .count(),
            recent_sources: sources
                .iter()
                .filter(|source| self.source_is_recent(source, freshness_window_days))
                .count(),
        }
    }

    fn normalized_query_terms(&self, query: &str) -> Vec<String> {
        let mut seen = HashSet::new();
        query
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .map(|term| term.trim().to_ascii_lowercase())
            .filter(|term| !term.is_empty())
            .filter(|term| term.len() >= 2)
            .filter(|term| {
                !matches!(
                    term.as_str(),
                    "a" | "an"
                        | "and"
                        | "are"
                        | "as"
                        | "at"
                        | "be"
                        | "between"
                        | "by"
                        | "for"
                        | "from"
                        | "how"
                        | "in"
                        | "into"
                        | "is"
                        | "latest"
                        | "of"
                        | "on"
                        | "or"
                        | "recent"
                        | "report"
                        | "research"
                        | "should"
                        | "source"
                        | "sources"
                        | "that"
                        | "the"
                        | "their"
                        | "these"
                        | "this"
                        | "those"
                        | "to"
                        | "what"
                        | "when"
                        | "where"
                        | "which"
                        | "who"
                        | "why"
                        | "with"
                )
            })
            .filter(|term| seen.insert(term.clone()))
            .collect()
    }

    fn search_result_matches_query_terms(
        &self,
        result: &SearchResult,
        query_terms: &[String],
    ) -> bool {
        let haystack = format!("{} {}", result.title, result.snippet);
        self.text_relevance_score(&haystack, query_terms).is_some()
    }

    fn text_relevance_score(&self, text: &str, query_terms: &[String]) -> Option<f32> {
        if query_terms.is_empty() {
            return Some(0.0);
        }

        let text_lower = text.to_ascii_lowercase();
        let total_matches = query_terms
            .iter()
            .filter(|term| text_lower.contains(term.as_str()))
            .count();
        let specific_matches = query_terms
            .iter()
            .filter(|term| self.is_specific_query_term(term))
            .filter(|term| text_lower.contains(term.as_str()))
            .count();
        let minimum_matches = self.minimum_query_matches(query_terms.len());

        if total_matches < minimum_matches {
            return None;
        }
        if query_terms.len() >= 4 && specific_matches == 0 {
            return None;
        }

        Some(total_matches as f32 + (specific_matches as f32 * 0.35))
    }

    fn minimum_query_matches(&self, query_term_count: usize) -> usize {
        if query_term_count >= 8 {
            3
        } else if query_term_count >= 4 {
            2
        } else {
            1
        }
    }

    fn is_specific_query_term(&self, term: &str) -> bool {
        term.len() >= 6 || (term.len() == 4 && term.chars().all(|ch| ch.is_ascii_digit()))
    }

    fn build_snippet_finding(
        &self,
        snippet: &str,
        query_terms: &[String],
        source_index: usize,
        confidence: f32,
    ) -> Option<Finding> {
        let content = snippet.trim();
        if content.is_empty() {
            return None;
        }
        if self.text_relevance_score(content, query_terms).is_none() {
            return None;
        }
        Some(Finding {
            content: content.to_string(),
            confidence,
            source_index,
            supporting_source_indices: vec![source_index],
        })
    }

    /// Extract key points from content relevant to the query
    fn extract_key_points(&self, content: &str, query: &str) -> Vec<String> {
        let query_terms = self.normalized_query_terms(query);
        let mut candidates: Vec<(String, f32)> = Vec::new();

        for sentence in content.split(['.', '!', '?', '\n']) {
            let sentence = sentence.trim();
            if sentence.len() < 30 || sentence.len() > 480 {
                continue;
            }

            let sentence_lower = sentence.to_lowercase();
            let Some(mut score) = self.text_relevance_score(sentence, &query_terms) else {
                continue;
            };
            if sentence.chars().any(|ch| ch.is_ascii_digit()) {
                score += 0.35;
            }
            if self.has_negation_or_uncertainty(sentence) {
                score += 0.15;
            }
            if sentence_lower.contains("according to")
                || sentence_lower.contains("announced")
                || sentence_lower.contains("released")
                || sentence_lower.contains("supports")
                || sentence_lower.contains("does not")
            {
                score += 0.2;
            }

            candidates.push((sentence.to_string(), score));
        }

        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut key_points: Vec<String> = Vec::new();
        for (candidate, _) in candidates {
            if key_points
                .iter()
                .any(|existing| self.similarity(existing.as_str(), candidate.as_str()) > 0.72)
            {
                continue;
            }
            key_points.push(candidate);
            if key_points.len() >= 5 {
                break;
            }
        }
        key_points
    }

    /// Estimate reliability score based on URL
    fn estimate_reliability(&self, url: &str) -> f32 {
        let url_lower = url.to_lowercase();

        // Academic and government sources are highly reliable
        if url_lower.contains(".edu") || url_lower.contains(".gov") {
            return 0.95;
        }

        // Known reliable domains
        let reliable_domains = [
            "wikipedia.org",
            "arxiv.org",
            "nature.com",
            "science.org",
            "bbc.com",
            "reuters.com",
            "apnews.com",
            "nytimes.com",
            "sec.gov",
            "who.int",
            "imf.org",
            "worldbank.org",
            "oecd.org",
            "europa.eu",
            "github.com",
            "stackoverflow.com",
            "docs.rs",
            "crates.io",
        ];

        for domain in &reliable_domains {
            if url_lower.contains(domain) {
                return 0.85;
            }
        }

        // Default reliability
        0.6
    }

    fn looks_like_primary_source(&self, url: &str) -> bool {
        let lower = url.to_lowercase();
        lower.contains(".gov")
            || lower.contains(".edu")
            || lower.contains("docs.")
            || lower.contains("/docs")
            || lower.contains("developer.")
            || lower.contains("github.com")
            || lower.contains("arxiv.org")
            || lower.contains("sec.gov")
            || lower.contains("who.int")
            || lower.contains("worldbank.org")
            || lower.contains("oecd.org")
            || lower.contains("europa.eu")
            || lower.contains("ietf.org")
            || lower.contains("w3.org")
    }

    fn research_rank_score(
        &self,
        result: &SearchResult,
        query: &str,
        prefer_primary_sources: bool,
        category: ResearchQueryCategory,
        freshness_window_days: Option<i64>,
    ) -> f32 {
        let query_terms = self.normalized_query_terms(query);
        let mut score = self.estimate_reliability(&result.url);
        let haystack = format!(
            "{} {}",
            result.title.to_lowercase(),
            result.snippet.to_lowercase()
        );
        let Some(relevance_score) = self.text_relevance_score(&haystack, &query_terms) else {
            return 0.0;
        };
        score += relevance_score.min(6.0) * 0.04;
        if prefer_primary_sources && self.looks_like_primary_source(&result.url) {
            score += 0.18;
        }
        score += self.search_result_recency_bonus(result);
        if freshness_window_days.is_some()
            && self.search_result_is_recent(result, freshness_window_days)
        {
            score += 0.08;
        }
        score += match category {
            ResearchQueryCategory::Primary => 0.16,
            ResearchQueryCategory::Recent => 0.08,
            ResearchQueryCategory::Comparison => 0.06,
            ResearchQueryCategory::Risks => 0.05,
            ResearchQueryCategory::General => 0.03,
        };
        score
    }

    fn select_diverse_results(
        &self,
        mut ranked: Vec<RankedResearchResult>,
        max_sources: usize,
        prefer_primary_sources: bool,
        min_primary_sources: usize,
        freshness_window_days: Option<i64>,
    ) -> Vec<RankedResearchResult> {
        ranked.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut selected = Vec::new();
        let mut used_urls = HashSet::new();

        if min_primary_sources > 0 {
            self.push_matching_ranked_results(
                &ranked,
                max_sources,
                &mut selected,
                &mut used_urls,
                min_primary_sources,
                |candidate| self.looks_like_primary_source(&candidate.result.url),
            );
        }
        if freshness_window_days.is_some() {
            self.push_matching_ranked_results(
                &ranked,
                max_sources,
                &mut selected,
                &mut used_urls,
                1,
                |candidate| self.search_result_is_recent(&candidate.result, freshness_window_days),
            );
        }

        let mut quotas = vec![
            (ResearchQueryCategory::Recent, 1usize),
            (ResearchQueryCategory::Comparison, 1usize),
            (ResearchQueryCategory::Risks, 1usize),
        ];

        if prefer_primary_sources {
            quotas.insert(0, (ResearchQueryCategory::Primary, 1usize));
        } else {
            quotas.insert(0, (ResearchQueryCategory::General, 1usize));
        }

        for (category, target) in quotas {
            let mut added = 0usize;
            for candidate in &ranked {
                let url_key = candidate.result.url.to_lowercase();
                if used_urls.contains(&url_key) || !candidate.categories.contains(&category) {
                    continue;
                }
                used_urls.insert(url_key);
                selected.push(candidate.clone());
                added += 1;
                if selected.len() >= max_sources || added >= target {
                    break;
                }
            }
            if selected.len() >= max_sources {
                return selected;
            }
        }

        for candidate in ranked {
            let url_key = candidate.result.url.to_lowercase();
            if used_urls.contains(&url_key) {
                continue;
            }
            used_urls.insert(url_key);
            selected.push(candidate);
            if selected.len() >= max_sources {
                break;
            }
        }

        selected
    }

    fn push_matching_ranked_results<F>(
        &self,
        ranked: &[RankedResearchResult],
        max_sources: usize,
        selected: &mut Vec<RankedResearchResult>,
        used_urls: &mut HashSet<String>,
        target: usize,
        predicate: F,
    ) where
        F: Fn(&RankedResearchResult) -> bool,
    {
        let mut added = 0usize;
        for candidate in ranked {
            let url_key = candidate.result.url.to_lowercase();
            if used_urls.contains(&url_key) || !predicate(candidate) {
                continue;
            }
            used_urls.insert(url_key);
            selected.push(candidate.clone());
            added += 1;
            if selected.len() >= max_sources || added >= target {
                break;
            }
        }
    }

    fn cluster_findings(&self, findings: &[Finding]) -> Vec<Finding> {
        let mut clusters: Vec<Vec<Finding>> = Vec::new();

        'outer: for finding in findings.iter().cloned() {
            for cluster in &mut clusters {
                if cluster
                    .iter()
                    .any(|existing| self.similarity(&existing.content, &finding.content) > 0.52)
                {
                    cluster.push(finding);
                    continue 'outer;
                }
            }
            clusters.push(vec![finding]);
        }

        let mut clustered_findings = clusters
            .into_iter()
            .filter_map(|cluster| {
                let representative = cluster.iter().cloned().max_by(|a, b| {
                    a.confidence
                        .partial_cmp(&b.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })?;
                let support_count = cluster
                    .iter()
                    .flat_map(|finding| {
                        finding
                            .supporting_source_indices
                            .iter()
                            .copied()
                            .chain(std::iter::once(finding.source_index))
                    })
                    .collect::<HashSet<_>>()
                    .len();
                let mut support_indices = cluster
                    .iter()
                    .flat_map(|finding| {
                        finding
                            .supporting_source_indices
                            .iter()
                            .copied()
                            .chain(std::iter::once(finding.source_index))
                    })
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                support_indices.sort_unstable();
                let average_confidence = cluster
                    .iter()
                    .map(|finding| finding.confidence)
                    .sum::<f32>()
                    / cluster.len().max(1) as f32;
                Some(Finding {
                    content: if support_count > 1 {
                        format!(
                            "{} (corroborated across {} sources)",
                            representative.content, support_count
                        )
                    } else {
                        representative.content
                    },
                    confidence: (average_confidence
                        + (support_count.saturating_sub(1) as f32 * 0.06))
                        .min(0.98),
                    source_index: representative.source_index,
                    supporting_source_indices: support_indices,
                })
            })
            .collect::<Vec<_>>();

        clustered_findings.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        clustered_findings.truncate(6);
        clustered_findings
    }

    fn detect_contradictions(&self, findings: &[Finding], sources: &[Source]) -> Vec<String> {
        let mut contradictions = Vec::new();
        let mut seen = HashSet::new();

        for (idx, a) in findings.iter().enumerate() {
            for b in findings.iter().skip(idx + 1) {
                if a.source_index == b.source_index {
                    continue;
                }
                let overlap = self.similarity(&a.content, &b.content);
                if overlap < 0.16 {
                    continue;
                }
                let a_negated = self.has_negation_or_uncertainty(&a.content);
                let b_negated = self.has_negation_or_uncertainty(&b.content);
                if a_negated == b_negated {
                    continue;
                }
                let left = self.compact_sentence(&a.content, 120);
                let right = self.compact_sentence(&b.content, 120);
                let summary = format!(
                    "{} [{}] conflicts with {} [{}].",
                    left,
                    sources
                        .get(a.source_index)
                        .map(|source| source.title.as_str())
                        .unwrap_or("source"),
                    right,
                    sources
                        .get(b.source_index)
                        .map(|source| source.title.as_str())
                        .unwrap_or("source")
                );
                let dedupe_key = summary.to_lowercase();
                if seen.insert(dedupe_key) {
                    contradictions.push(summary);
                }
                if contradictions.len() >= 4 {
                    return contradictions;
                }
            }
        }

        contradictions
    }

    fn derive_open_questions(
        &self,
        query: &str,
        sources: &[Source],
        findings: &[Finding],
        contradictions: &[String],
    ) -> Vec<String> {
        let mut questions = Vec::new();

        if sources
            .iter()
            .filter(|source| source.reliability >= 0.85)
            .count()
            < 2
        {
            questions.push(
                "Official or primary-source coverage is limited, so the latest documentation should still be checked directly."
                    .to_string(),
            );
        }

        for finding in findings {
            if self.has_negation_or_uncertainty(&finding.content) {
                questions.push(self.compact_sentence(&finding.content, 150));
            }
            if questions.len() >= 4 {
                break;
            }
        }

        if contradictions.is_empty() && findings.len() < 3 {
            questions.push(format!(
                "More source coverage may still be needed for edge cases, recent changes, or implementation-specific details related to {}.",
                query
            ));
        }

        questions
            .into_iter()
            .filter(|question| !question.trim().is_empty())
            .fold(Vec::<String>::new(), |mut acc, question| {
                if !acc
                    .iter()
                    .any(|existing| self.similarity(existing, &question) > 0.72)
                {
                    acc.push(question);
                }
                acc
            })
            .into_iter()
            .take(5)
            .collect()
    }

    fn has_negation_or_uncertainty(&self, content: &str) -> bool {
        let lower = content.to_lowercase();
        [
            "not ",
            "no ",
            "unclear",
            "unknown",
            "however",
            "but ",
            "depends",
            "not yet",
            "not publicly",
            "coming soon",
            "to be announced",
            "tbd",
            "may",
            "might",
            "risk",
            "limitation",
        ]
        .iter()
        .any(|cue| lower.contains(cue))
    }

    fn compact_sentence(&self, content: &str, max_len: usize) -> String {
        let compact = content.split_whitespace().collect::<Vec<_>>().join(" ");
        if compact.len() <= max_len {
            compact
        } else {
            format!(
                "{}...",
                compact
                    .chars()
                    .take(max_len.saturating_sub(3))
                    .collect::<String>()
            )
        }
    }

    fn normalize_search_tail(&self, content: &str, max_len: usize) -> String {
        self.compact_sentence(content, max_len)
            .chars()
            .map(|ch| match ch {
                '[' | ']' | '(' | ')' | ':' | ';' | '.' | ',' | '"' | '\'' => ' ',
                _ => ch,
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Generate summary from findings
    fn generate_summary(&self, query: &str, findings: &[Finding]) -> String {
        if findings.is_empty() {
            return format!("No relevant information found for: {}", query);
        }

        let top_findings: Vec<&str> = findings
            .iter()
            .take(3)
            .map(|f| f.content.as_str())
            .collect();

        format!(
            "Research summary for \"{}\": {}",
            query,
            top_findings.join(" | ")
        )
    }

    /// Generate comprehensive summary for deep research
    fn generate_comprehensive_summary(
        &self,
        query: &str,
        key_findings: &[Finding],
        sources: &[Source],
        open_questions: &[String],
        contradictions: &[String],
    ) -> String {
        if key_findings.is_empty() {
            return format!("No relevant information found for: {}", query);
        }

        let avg_confidence: f32 =
            key_findings.iter().map(|f| f.confidence).sum::<f32>() / key_findings.len() as f32;

        let avg_reliability: f32 =
            sources.iter().map(|s| s.reliability).sum::<f32>() / sources.len().max(1) as f32;
        let coverage = self.research_coverage(sources, Some(365));

        let contradiction_line = if contradictions.is_empty() {
            "No major source contradictions surfaced in the top findings.".to_string()
        } else {
            format!(
                "{} contradiction(s) still need judgment.",
                contradictions.len()
            )
        };
        let open_question_line = if open_questions.is_empty() {
            "Most major questions were answered by the collected sources.".to_string()
        } else {
            format!(
                "{} open question(s) remain for follow-up.",
                open_questions.len()
            )
        };

        format!(
            "**Sources analyzed:** {}\n\
            **Primary-like sources:** {}\n\
            **Recent sources with dates:** {}\n\
            **Average confidence:** {:.0}%\n\
            **Average source reliability:** {:.0}%\n\
            **Contradictions:** {}\n\
            **Open questions:** {}",
            sources.len(),
            coverage.primary_sources,
            coverage.recent_sources,
            avg_confidence * 100.0,
            avg_reliability * 100.0,
            contradiction_line,
            open_question_line
        )
    }

    /// Generate sub-queries for deep research
    fn generate_research_queries(
        &self,
        query: &str,
        prefer_primary_sources: bool,
    ) -> Vec<ResearchQuery> {
        let mut queries = vec![
            ResearchQuery {
                category: ResearchQueryCategory::General,
                text: query.to_string(),
            },
            ResearchQuery {
                category: ResearchQueryCategory::Primary,
                text: format!("{} primary sources", query),
            },
            ResearchQuery {
                category: ResearchQueryCategory::Recent,
                text: format!("{} recent coverage", query),
            },
            ResearchQuery {
                category: ResearchQueryCategory::Comparison,
                text: format!("{} comparison alternatives", query),
            },
            ResearchQuery {
                category: ResearchQueryCategory::Risks,
                text: format!("{} risks limitations open questions", query),
            },
        ];

        if prefer_primary_sources {
            queries.push(ResearchQuery {
                category: ResearchQueryCategory::Primary,
                text: format!("{} official source", query),
            });
            queries.push(ResearchQuery {
                category: ResearchQueryCategory::Primary,
                text: format!("{} original report data", query),
            });
            queries.push(ResearchQuery {
                category: ResearchQueryCategory::Primary,
                text: format!("{} source material", query),
            });
        } else {
            queries.push(ResearchQuery {
                category: ResearchQueryCategory::General,
                text: format!("{} overview", query),
            });
            queries.push(ResearchQuery {
                category: ResearchQueryCategory::Comparison,
                text: format!("{} expert analysis", query),
            });
            queries.push(ResearchQuery {
                category: ResearchQueryCategory::Recent,
                text: format!("{} case study", query),
            });
        }

        let mut seen = HashSet::new();
        queries
            .into_iter()
            .filter(|candidate| seen.insert(candidate.text.to_lowercase()))
            .collect()
    }

    fn generate_followup_queries(
        &self,
        query: &str,
        open_questions: &[String],
        contradictions: &[String],
        primary_gap: bool,
        freshness_gap: bool,
        round: usize,
    ) -> Vec<ResearchQuery> {
        let mut queries = Vec::new();
        if primary_gap {
            queries.push(ResearchQuery {
                category: ResearchQueryCategory::Primary,
                text: format!("{} official source", query),
            });
            queries.push(ResearchQuery {
                category: ResearchQueryCategory::Primary,
                text: format!("{} original report data", query),
            });
        }
        if freshness_gap {
            queries.push(ResearchQuery {
                category: ResearchQueryCategory::Recent,
                text: format!("{} {}", query, Utc::now().year()),
            });
        }
        for question in open_questions.iter().take(2) {
            queries.push(ResearchQuery {
                category: ResearchQueryCategory::General,
                text: format!("{} {}", query, self.normalize_search_tail(question, 90)),
            });
        }
        if let Some(contradiction) =
            contradictions.get(round.min(contradictions.len().saturating_sub(1)))
        {
            queries.push(ResearchQuery {
                category: ResearchQueryCategory::Risks,
                text: format!(
                    "{} {}",
                    query,
                    self.normalize_search_tail(contradiction, 90)
                ),
            });
        }

        let mut seen = HashSet::new();
        queries
            .into_iter()
            .map(|mut candidate| {
                candidate.text = candidate
                    .text
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                candidate
            })
            .filter(|candidate| !candidate.text.trim().is_empty())
            .filter(|candidate| seen.insert(candidate.text.to_lowercase()))
            .collect()
    }

    /// Extract related topics from search results
    fn extract_related_topics(&self, results: &[SearchResult]) -> Vec<String> {
        let mut topics: std::collections::HashSet<String> = std::collections::HashSet::new();

        for result in results {
            // Extract potential topics from titles
            let words: Vec<&str> = result.title.split_whitespace().collect();
            for window in words.windows(2) {
                if window.len() == 2 {
                    let potential_topic = format!("{} {}", window[0], window[1]);
                    if potential_topic.len() > 5 && potential_topic.len() < 30 {
                        topics.insert(potential_topic);
                    }
                }
            }
        }

        topics.into_iter().take(5).collect()
    }

    /// Deduplicate similar findings
    fn deduplicate_findings(&self, findings: Vec<Finding>) -> Vec<Finding> {
        let mut unique: Vec<Finding> = Vec::new();

        for finding in findings {
            if let Some(existing) = unique
                .iter_mut()
                .find(|existing| self.similarity(&existing.content, &finding.content) > 0.7)
            {
                existing.confidence = existing.confidence.max(finding.confidence);
                for idx in finding
                    .supporting_source_indices
                    .iter()
                    .copied()
                    .chain(std::iter::once(finding.source_index))
                {
                    if !existing.supporting_source_indices.contains(&idx) {
                        existing.supporting_source_indices.push(idx);
                    }
                }
            } else {
                unique.push(finding);
            }
        }

        unique
    }

    /// Simple similarity measure (Jaccard similarity on words)
    fn similarity(&self, a: &str, b: &str) -> f32 {
        let a_lower = a.to_lowercase();
        let b_lower = b.to_lowercase();

        let words_a: std::collections::HashSet<&str> = a_lower.split_whitespace().collect();
        let words_b: std::collections::HashSet<&str> = b_lower.split_whitespace().collect();

        let intersection = words_a.intersection(&words_b).count();
        let union = words_a.union(&words_b).count();

        if union == 0 {
            0.0
        } else {
            intersection as f32 / union as f32
        }
    }
}

/// Execute a research request
pub async fn execute_research(args: &ResearchArgs, config: &SearchConfig) -> Result<String> {
    execute_research_with_progress(args, config, None).await
}

pub async fn execute_research_with_progress(
    args: &ResearchArgs,
    config: &SearchConfig,
    progress: Option<&ResearchProgressReporter>,
) -> Result<String> {
    let client = ResearchClient::new(config.clone());
    let result = client.research_with_progress(args, progress).await?;

    // Format output
    let mut output = format!("# Research: {}\n\n", result.query);
    output.push_str(&format!("{}\n\n", result.summary));

    let primary_findings = if result.key_findings.is_empty() {
        &result.findings
    } else {
        &result.key_findings
    };

    if !primary_findings.is_empty() {
        output.push_str("## Key Findings\n\n");
        for (i, finding) in primary_findings.iter().enumerate() {
            let citations = format_finding_citations(finding);
            output.push_str(&format!(
                "{}. {} (confidence: {:.0}%, sources: {})\n\n",
                i + 1,
                finding.content,
                finding.confidence * 100.0,
                citations
            ));
        }
    }

    if !result.open_questions.is_empty() {
        output.push_str("## Open Questions\n\n");
        for question in &result.open_questions {
            output.push_str(&format!("- {}\n", question));
        }
        output.push('\n');
    }

    if !result.contradictions.is_empty() {
        output.push_str("## Contradictions To Verify\n\n");
        for contradiction in &result.contradictions {
            output.push_str(&format!("- {}\n", contradiction));
        }
        output.push('\n');
    }

    if !result.sources.is_empty() {
        output.push_str("## Sources\n\n");
        for (i, source) in result.sources.iter().enumerate() {
            let date_suffix = source
                .published_date
                .as_deref()
                .map(|date| format!(" | date: {}", date))
                .unwrap_or_default();
            if let Some(url) = client.normalize_source_url(&source.url) {
                output.push_str(&format!(
                    "{}. [{}]({}) - reliability: {:.0}%{}\n",
                    i + 1,
                    source.title,
                    url,
                    source.reliability * 100.0,
                    date_suffix
                ));
            } else {
                output.push_str(&format!(
                    "{}. {} - reliability: {:.0}%{}\n",
                    i + 1,
                    source.title,
                    source.reliability * 100.0,
                    date_suffix
                ));
            }
        }
    }

    if !result.related_topics.is_empty() {
        output.push_str("\n## Related Topics\n\n");
        for topic in &result.related_topics {
            output.push_str(&format!("- {}\n", topic));
        }
    }

    Ok(output)
}

fn format_finding_citations(finding: &Finding) -> String {
    let mut citations = finding.supporting_source_indices.clone();
    if citations.is_empty() {
        citations.push(finding.source_index);
    } else if !citations.contains(&finding.source_index) {
        citations.push(finding.source_index);
    }
    citations.sort_unstable();
    citations.dedup();
    citations
        .into_iter()
        .map(|idx| (idx + 1).to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_client() -> ResearchClient {
        ResearchClient::new(SearchConfig::default())
    }

    #[test]
    fn deep_research_defaults_to_twelve_sources() {
        let client = test_client();
        let args = ResearchArgs {
            query: "open source ai agent release strategy".to_string(),
            max_sources: default_max_sources(),
            _include_sources: true,
            backend: None,
            depth: ResearchDepth::Deep,
            min_primary_sources: 0,
            freshness_window_days: None,
            followup_rounds: 0,
        };

        assert_eq!(client.effective_max_sources(&args), 12);
    }

    #[test]
    fn deep_research_queries_cover_verification_phases() {
        let client = test_client();
        let queries = client.generate_research_queries("rust agent framework", true);

        assert!(
            queries
                .iter()
                .any(|query| query.text.contains("primary sources"))
        );
        assert!(
            queries
                .iter()
                .any(|query| query.text.contains("recent coverage"))
        );
        assert!(
            queries
                .iter()
                .any(|query| query.text.contains("comparison alternatives"))
        );
        assert!(
            queries
                .iter()
                .any(|query| query.text.contains("risks limitations open questions"))
        );
    }

    #[test]
    fn deep_research_defaults_to_primary_coverage_followups_and_recent_bias() {
        let client = test_client();
        let args = ResearchArgs {
            query: "distributed agent runtime".to_string(),
            max_sources: default_max_sources(),
            _include_sources: true,
            backend: None,
            depth: ResearchDepth::Deep,
            min_primary_sources: 0,
            freshness_window_days: None,
            followup_rounds: 0,
        };

        assert_eq!(client.effective_min_primary_sources(&args), 2);
        assert_eq!(client.effective_followup_rounds(&args), 2);
        assert_eq!(
            client.effective_freshness_window_days(&args, &args.query),
            Some(365)
        );
    }

    #[test]
    fn contradiction_detection_surfaces_conflicting_claims() {
        let client = test_client();
        let findings = vec![
            Finding {
                content: "The project supports offline mode for local execution.".to_string(),
                confidence: 0.86,
                source_index: 0,
                supporting_source_indices: vec![0],
            },
            Finding {
                content: "The project does not yet support offline mode for local execution."
                    .to_string(),
                confidence: 0.82,
                source_index: 1,
                supporting_source_indices: vec![1],
            },
        ];
        let sources = vec![
            Source {
                title: "Official docs".to_string(),
                url: "https://docs.example.com/offline".to_string(),
                description: String::new(),
                reliability: 0.92,
                published_date: Some("2026-04-01".to_string()),
            },
            Source {
                title: "Recent review".to_string(),
                url: "https://example.com/review".to_string(),
                description: String::new(),
                reliability: 0.71,
                published_date: Some("2026-04-02".to_string()),
            },
        ];

        let contradictions = client.detect_contradictions(&findings, &sources);
        assert!(!contradictions.is_empty());
    }

    #[test]
    fn extract_key_points_filters_unrelated_sentences_for_long_queries() {
        let client = test_client();
        let content = "Report workplace sexual harassment complaints on SHe-Box portal, the Government of India's online complaint system.\nPrototype Fast Breeder Reactor at Kalpakkam, Tamil Nadu attains First Criticality.\nIndia's AI talent pipeline remains concentrated in a few universities, while startup formation is strongest around Bengaluru and Hyderabad.\n";

        let key_points = client.extract_key_points(
            content,
            "India AI research capacity 2025-2026 universities labs talent pipeline publications startups",
        );

        assert_eq!(key_points.len(), 1);
        assert!(key_points[0].contains("talent pipeline"));
    }

    #[test]
    fn normalize_source_url_adds_https_to_scheme_less_domains() {
        let client = test_client();
        assert_eq!(
            client.normalize_source_url("pib.gov.in/PressReleasePage.aspx?PRID=1"),
            Some("https://pib.gov.in/PressReleasePage.aspx?PRID=1".to_string())
        );
        assert_eq!(client.normalize_source_url("/local/path"), None);
    }

    #[test]
    fn generate_comprehensive_summary_omits_duplicate_key_findings_block() {
        let client = test_client();
        let summary = client.generate_comprehensive_summary(
            "India AI research capacity",
            &[Finding {
                content: "AI research output is concentrated in a small number of institutes."
                    .to_string(),
                confidence: 0.91,
                source_index: 0,
                supporting_source_indices: vec![0],
            }],
            &[Source {
                title: "Example".to_string(),
                url: "https://example.com".to_string(),
                description: String::new(),
                reliability: 0.8,
                published_date: Some("2026-04-01".to_string()),
            }],
            &[],
            &[],
        );

        assert!(summary.contains("**Sources analyzed:** 1"));
        assert!(summary.contains("**Primary-like sources:**"));
        assert!(!summary.contains("# Research Summary:"));
        assert!(!summary.contains("## Key Findings"));
    }
}
