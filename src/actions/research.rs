//! Research Action - Deep web research and information synthesis
//!
//! Provides comprehensive research capabilities by:
//! 1. Searching multiple sources
//! 2. Fetching and extracting content from URLs
//! 3. Synthesizing information into a coherent report

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use super::search::{SearchBackend, SearchClient, SearchConfig, SearchResult};

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

/// Research client
pub struct ResearchClient {
    search_config: SearchConfig,
    http_client: reqwest::Client,
}

impl ResearchClient {
    pub fn new(search_config: SearchConfig) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .expect("Failed to create HTTP client");

        Self {
            search_config,
            http_client,
        }
    }

    /// Perform research on a topic
    pub async fn research(&self, args: &ResearchArgs) -> Result<ResearchResult> {
        match args.depth {
            ResearchDepth::Quick => self.quick_research(args).await,
            ResearchDepth::Standard => self.standard_research(args).await,
            ResearchDepth::Deep => self.deep_research(args).await,
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

    /// Quick research - just search results
    async fn quick_research(&self, args: &ResearchArgs) -> Result<ResearchResult> {
        let max_sources = self.effective_max_sources(args);
        let search_results = self
            .search(&args.query, max_sources, &args.backend)
            .await?;

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
            .map(|r| Source {
                title: r.title.clone(),
                url: r.url.clone(),
                description: r.snippet.clone(),
                reliability: self.estimate_reliability(&r.url),
            })
            .collect();

        let summary = self.generate_summary(&args.query, &findings);

        Ok(ResearchResult {
            query: args.query.clone(),
            summary,
            key_findings: findings.clone(),
            findings,
            open_questions: Vec::new(),
            contradictions: Vec::new(),
            sources,
            related_topics: self.extract_related_topics(&search_results),
        })
    }

    /// Standard research - search + fetch content
    async fn standard_research(&self, args: &ResearchArgs) -> Result<ResearchResult> {
        let max_sources = self.effective_max_sources(args);
        let search_results = self
            .search(&args.query, max_sources, &args.backend)
            .await?;

        let mut sources: Vec<Source> = Vec::new();
        let mut findings: Vec<Finding> = Vec::new();

        for (i, result) in search_results.iter().enumerate().take(max_sources) {
            // Add source
            sources.push(Source {
                title: result.title.clone(),
                url: result.url.clone(),
                description: result.snippet.clone(),
                reliability: self.estimate_reliability(&result.url),
            });

            // Try to fetch and extract content
            match self.fetch_content(&result.url).await {
                Ok(content) => {
                    // Extract key points from the content
                    let key_points = self.extract_key_points(&content, &args.query);
                    for point in key_points {
                        findings.push(Finding {
                            content: point,
                            confidence: 0.8,
                            source_index: i,
                            supporting_source_indices: vec![i],
                        });
                    }
                }
                Err(_) => {
                    // Fall back to snippet
                    findings.push(Finding {
                        content: result.snippet.clone(),
                        confidence: 0.6,
                        source_index: i,
                        supporting_source_indices: vec![i],
                    });
                }
            }
        }

        let summary = self.generate_summary(&args.query, &findings);

        Ok(ResearchResult {
            query: args.query.clone(),
            summary,
            key_findings: findings.clone(),
            findings,
            open_questions: Vec::new(),
            contradictions: Vec::new(),
            sources,
            related_topics: self.extract_related_topics(&search_results),
        })
    }

    /// Deep research - multiple searches + comprehensive analysis
    async fn deep_research(&self, args: &ResearchArgs) -> Result<ResearchResult> {
        let max_sources = self.effective_max_sources(args);
        let queries = self.generate_research_queries(&args.query);
        let prefers_official_sources = self.prefers_official_sources(&args.query);
        let mut results_by_url: HashMap<String, RankedResearchResult> = HashMap::new();

        for query in &queries {
            if let Ok(results) = self.search(&query.text, 4, &args.backend).await {
                for result in results {
                    let normalized_url = result.url.trim().to_lowercase();
                    if normalized_url.is_empty() {
                        continue;
                    }
                    let score = self.research_rank_score(
                        &result,
                        &args.query,
                        prefers_official_sources,
                        query.category,
                    );
                    results_by_url
                        .entry(normalized_url)
                        .and_modify(|existing| {
                            existing.categories.insert(query.category);
                            if score > existing.score {
                                existing.result = result.clone();
                                existing.score = score;
                            }
                        })
                        .or_insert_with(|| RankedResearchResult {
                            result,
                            categories: HashSet::from([query.category]),
                            score,
                        });
                }
            }
        }

        let ranked_results = self.select_diverse_results(
            results_by_url.into_values().collect(),
            max_sources,
            prefers_official_sources,
        );
        let all_results = ranked_results
            .into_iter()
            .map(|entry| entry.result)
            .collect::<Vec<_>>();

        let mut sources: Vec<Source> = Vec::new();
        let mut findings: Vec<Finding> = Vec::new();

        for (i, result) in all_results.iter().enumerate() {
            let reliability = self.estimate_reliability(&result.url);
            sources.push(Source {
                title: result.title.clone(),
                url: result.url.clone(),
                description: result.snippet.clone(),
                reliability,
            });

            match self.fetch_content(&result.url).await {
                Ok(content) => {
                    let key_points = self.extract_key_points(&content, &args.query);
                    if key_points.is_empty() {
                        findings.push(Finding {
                            content: result.snippet.clone(),
                            confidence: (0.58 + reliability * 0.25).min(0.92),
                            source_index: i,
                            supporting_source_indices: vec![i],
                        });
                    } else {
                        for point in key_points {
                            findings.push(Finding {
                                content: point,
                                confidence: (0.62 + reliability * 0.28).min(0.96),
                                source_index: i,
                                supporting_source_indices: vec![i],
                            });
                        }
                    }
                }
                Err(_) => {
                    findings.push(Finding {
                        content: result.snippet.clone(),
                        confidence: (0.44 + reliability * 0.2).min(0.8),
                        source_index: i,
                        supporting_source_indices: vec![i],
                    });
                }
            }
        }

        findings = self.deduplicate_findings(findings);
        let key_findings = self.cluster_findings(&findings);
        let contradictions = self.detect_contradictions(&findings, &sources);
        let open_questions =
            self.derive_open_questions(&args.query, &sources, &findings, &contradictions);
        let summary = self.generate_comprehensive_summary(
            &args.query,
            &key_findings,
            &sources,
            &open_questions,
            &contradictions,
        );

        Ok(ResearchResult {
            query: args.query.clone(),
            summary,
            findings,
            key_findings,
            open_questions,
            contradictions,
            sources,
            related_topics: self.extract_related_topics(&all_results),
        })
    }

    /// Search using configured backend with fallback chain
    async fn search(
        &self,
        query: &str,
        num_results: usize,
        backend_preference: &Option<String>,
    ) -> Result<Vec<SearchResult>> {
        if let Some(explicit) = backend_preference.as_deref() {
            let backend = match explicit {
                "serper" => self
                    .search_config
                    .serper
                    .clone()
                    .ok_or_else(|| anyhow!("Serper not configured"))?,
                "brave" | "brave_api" => self
                    .search_config
                    .brave
                    .clone()
                    .ok_or_else(|| anyhow!("Brave not configured"))?,
                "playwright" => self
                    .search_config
                    .playwright
                    .clone()
                    .ok_or_else(|| anyhow!("Playwright not configured"))?,
                "duckduckgo" => SearchBackend::DuckDuckGo,
                "lightpanda" => SearchBackend::Lightpanda,
                other => return Err(anyhow!("Unknown search backend: {}", other)),
            };
            let client = SearchClient::new(backend);
            let response = client.search(query, num_results).await?;
            return Ok(response.results);
        }

        let chain = self.search_config.ordered_backend_names();
        let mut last_err = None;
        for name in &chain {
            if let Some(backend) = self.search_config.resolve_backend(name) {
                let client = SearchClient::new(backend);
                match client.search(query, num_results).await {
                    Ok(response) if !response.results.is_empty() => {
                        return Ok(response.results);
                    }
                    Ok(_) => {
                        tracing::warn!(
                            "Research: search backend '{}' returned 0 results, trying next",
                            name
                        );
                        last_err = Some(anyhow!("Backend '{}' returned 0 results", name));
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Research: search backend '{}' failed: {}, trying next",
                            name,
                            e
                        );
                        last_err = Some(e);
                    }
                }
            } else {
                tracing::debug!(
                    "Research: search backend '{}' not configured, trying next",
                    name
                );
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow!("All search backends failed")))
    }

    /// Fetch content from a URL
    async fn fetch_content(&self, url: &str) -> Result<String> {
        // Fast-path: Lightpanda returns clean markdown (no HTML stripping needed)
        match crate::integrations::lightpanda::fetch_markdown(url).await {
            Ok(markdown) => return Ok(markdown),
            Err(e) => {
                tracing::debug!(
                    "Lightpanda unavailable for research fetch, falling back to reqwest: {}",
                    e
                );
            }
        }

        // Fallback: raw HTTP + HTML text extraction
        let response = self.http_client.get(url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!("Failed to fetch URL: {}", response.status()));
        }

        let html = response.text().await?;

        // Extract text content from HTML
        Ok(self.extract_text_from_html(&html))
    }

    /// Extract text content from HTML
    fn extract_text_from_html(&self, html: &str) -> String {
        // Remove script and style tags
        let mut text = html.to_string();

        // Simple regex-free HTML stripping
        // Remove script tags
        while let Some(start) = text.find("<script") {
            if let Some(end) = text[start..].find("</script>") {
                text = format!("{}{}", &text[..start], &text[start + end + 9..]);
            } else {
                break;
            }
        }

        // Remove style tags
        while let Some(start) = text.find("<style") {
            if let Some(end) = text[start..].find("</style>") {
                text = format!("{}{}", &text[..start], &text[start + end + 8..]);
            } else {
                break;
            }
        }

        // Remove all remaining HTML tags
        let mut result = String::new();
        let mut in_tag = false;

        for c in text.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => result.push(c),
                _ => {}
            }
        }

        // Clean up whitespace
        result
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Extract key points from content relevant to the query
    fn extract_key_points(&self, content: &str, query: &str) -> Vec<String> {
        let query_lower = query.to_lowercase();
        let query_words: Vec<String> = query_lower
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        let mut candidates: Vec<(String, f32)> = Vec::new();

        for sentence in content.split(['.', '!', '?', '\n']) {
            let sentence = sentence.trim();
            if sentence.len() < 30 || sentence.len() > 480 {
                continue;
            }

            let sentence_lower = sentence.to_lowercase();
            let relevance: usize = query_words
                .iter()
                .filter(|w| sentence_lower.contains(w.as_str()))
                .count();
            if relevance == 0 {
                continue;
            }

            let mut score = relevance as f32;
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

        candidates.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

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

    fn prefers_official_sources(&self, query: &str) -> bool {
        let query_lower = query.to_lowercase();
        [
            "api",
            "sdk",
            "library",
            "framework",
            "package",
            "crate",
            "npm",
            "docs",
            "documentation",
            "standard",
            "spec",
            "specification",
            "protocol",
            "implementation",
            "config",
            "configuration",
            "release notes",
            "changelog",
        ]
        .iter()
        .any(|keyword| query_lower.contains(keyword))
    }

    fn looks_like_primary_source(&self, url: &str) -> bool {
        let lower = url.to_lowercase();
        lower.contains(".gov")
            || lower.contains(".edu")
            || lower.contains("docs.")
            || lower.contains("/docs")
            || lower.contains("developer.")
            || lower.contains("github.com")
            || lower.contains("ietf.org")
            || lower.contains("w3.org")
    }

    fn research_rank_score(
        &self,
        result: &SearchResult,
        query: &str,
        prefers_official_sources: bool,
        category: ResearchQueryCategory,
    ) -> f32 {
        let mut score = self.estimate_reliability(&result.url);
        let haystack = format!("{} {}", result.title.to_lowercase(), result.snippet.to_lowercase());
        let query_terms: Vec<&str> = query
            .split_whitespace()
            .map(str::trim)
            .filter(|term| term.len() > 2)
            .collect();
        let match_count = query_terms
            .iter()
            .filter(|term| haystack.contains(&term.to_lowercase()))
            .count();
        score += (match_count.min(5) as f32) * 0.04;
        if prefers_official_sources && self.looks_like_primary_source(&result.url) {
            score += 0.18;
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
        prefers_official_sources: bool,
    ) -> Vec<RankedResearchResult> {
        ranked.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut selected = Vec::new();
        let mut used_urls = HashSet::new();
        let mut quotas = vec![
            (
                ResearchQueryCategory::Recent,
                1usize,
            ),
            (
                ResearchQueryCategory::Comparison,
                1usize,
            ),
            (
                ResearchQueryCategory::Risks,
                1usize,
            ),
        ];

        if prefers_official_sources {
            quotas.insert(0, (ResearchQueryCategory::Primary, 2usize));
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
                let representative = cluster
                    .iter()
                    .cloned()
                    .max_by(|a, b| {
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
                let average_confidence =
                    cluster.iter().map(|finding| finding.confidence).sum::<f32>()
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
                    confidence: (average_confidence + (support_count.saturating_sub(1) as f32 * 0.06))
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

        if sources.iter().filter(|source| source.reliability >= 0.85).count() < 2 {
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
            format!("{}...", compact.chars().take(max_len.saturating_sub(3)).collect::<String>())
        }
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

        let top_findings: Vec<String> = key_findings
            .iter()
            .take(4)
            .enumerate()
            .map(|(i, f)| format!("{}. {}", i + 1, f.content))
            .collect();

        let contradiction_line = if contradictions.is_empty() {
            "No major source contradictions surfaced in the top findings.".to_string()
        } else {
            format!("{} contradiction(s) still need judgment.", contradictions.len())
        };
        let open_question_line = if open_questions.is_empty() {
            "Most major questions were answered by the collected sources.".to_string()
        } else {
            format!("{} open question(s) remain for follow-up.", open_questions.len())
        };

        format!(
            "# Research Summary: {}\n\n\
            **Sources analyzed:** {}\n\
            **Average confidence:** {:.0}%\n\
            **Average source reliability:** {:.0}%\n\
            **Contradictions:** {}\n\
            **Open questions:** {}\n\n\
            ## Key Findings\n\
            {}",
            query,
            sources.len(),
            avg_confidence * 100.0,
            avg_reliability * 100.0,
            contradiction_line,
            open_question_line,
            top_findings.join("\n")
        )
    }

    /// Generate sub-queries for deep research
    fn generate_research_queries(&self, query: &str) -> Vec<ResearchQuery> {
        let prefers_official = self.prefers_official_sources(query);
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

        if prefers_official {
            queries.push(ResearchQuery {
                category: ResearchQueryCategory::Primary,
                text: format!("{} official documentation", query),
            });
            queries.push(ResearchQuery {
                category: ResearchQueryCategory::Primary,
                text: format!("{} implementation guide", query),
            });
            queries.push(ResearchQuery {
                category: ResearchQueryCategory::Primary,
                text: format!("{} specification standard", query),
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
    let client = ResearchClient::new(config.clone());
    let result = client.research(args).await?;

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
            output.push_str(&format!(
                "{}. [{}]({}) - reliability: {:.0}%\n",
                i + 1,
                source.title,
                source.url,
                source.reliability * 100.0
            ));
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
        };

        assert_eq!(client.effective_max_sources(&args), 12);
    }

    #[test]
    fn deep_research_queries_cover_verification_phases() {
        let client = test_client();
        let queries = client.generate_research_queries("rust agent framework");

        assert!(queries.iter().any(|query| query.text.contains("primary sources")));
        assert!(queries.iter().any(|query| query.text.contains("recent coverage")));
        assert!(queries.iter().any(|query| query.text.contains("comparison alternatives")));
        assert!(queries
            .iter()
            .any(|query| query.text.contains("risks limitations open questions")));
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
            },
            Source {
                title: "Recent review".to_string(),
                url: "https://example.com/review".to_string(),
                description: String::new(),
                reliability: 0.71,
            },
        ];

        let contradictions = client.detect_contradictions(&findings, &sources);
        assert!(!contradictions.is_empty());
    }
}
