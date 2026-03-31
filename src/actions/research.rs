//! Research Action - Deep web research and information synthesis
//!
//! Provides comprehensive research capabilities by:
//! 1. Searching multiple sources
//! 2. Fetching and extracting content from URLs
//! 3. Synthesizing information into a coherent report

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

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

    /// Quick research - just search results
    async fn quick_research(&self, args: &ResearchArgs) -> Result<ResearchResult> {
        let search_results = self
            .search(&args.query, args.max_sources, &args.backend)
            .await?;

        let findings: Vec<Finding> = search_results
            .iter()
            .enumerate()
            .map(|(i, r)| Finding {
                content: r.snippet.clone(),
                confidence: 0.7,
                source_index: i,
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
            findings,
            sources,
            related_topics: self.extract_related_topics(&search_results),
        })
    }

    /// Standard research - search + fetch content
    async fn standard_research(&self, args: &ResearchArgs) -> Result<ResearchResult> {
        let search_results = self
            .search(&args.query, args.max_sources, &args.backend)
            .await?;

        let mut sources: Vec<Source> = Vec::new();
        let mut findings: Vec<Finding> = Vec::new();

        for (i, result) in search_results.iter().enumerate().take(args.max_sources) {
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
                        });
                    }
                }
                Err(_) => {
                    // Fall back to snippet
                    findings.push(Finding {
                        content: result.snippet.clone(),
                        confidence: 0.6,
                        source_index: i,
                    });
                }
            }
        }

        let summary = self.generate_summary(&args.query, &findings);

        Ok(ResearchResult {
            query: args.query.clone(),
            summary,
            findings,
            sources,
            related_topics: self.extract_related_topics(&search_results),
        })
    }

    /// Deep research - multiple searches + comprehensive analysis
    async fn deep_research(&self, args: &ResearchArgs) -> Result<ResearchResult> {
        // Generate multiple search queries for different aspects
        let queries = self.generate_sub_queries(&args.query);

        let mut all_results: Vec<SearchResult> = Vec::new();
        let mut seen_urls: std::collections::HashSet<String> = std::collections::HashSet::new();

        // Search for each query
        for query in &queries {
            if let Ok(results) = self.search(query, 5, &args.backend).await {
                for result in results {
                    if !seen_urls.contains(&result.url) {
                        seen_urls.insert(result.url.clone());
                        all_results.push(result);
                    }
                }
            }
        }

        // Limit to max_sources
        all_results.truncate(args.max_sources);

        let mut sources: Vec<Source> = Vec::new();
        let mut findings: Vec<Finding> = Vec::new();

        for (i, result) in all_results.iter().enumerate() {
            sources.push(Source {
                title: result.title.clone(),
                url: result.url.clone(),
                description: result.snippet.clone(),
                reliability: self.estimate_reliability(&result.url),
            });

            match self.fetch_content(&result.url).await {
                Ok(content) => {
                    let key_points = self.extract_key_points(&content, &args.query);
                    for point in key_points {
                        findings.push(Finding {
                            content: point,
                            confidence: 0.85,
                            source_index: i,
                        });
                    }
                }
                Err(_) => {
                    findings.push(Finding {
                        content: result.snippet.clone(),
                        confidence: 0.5,
                        source_index: i,
                    });
                }
            }
        }

        // Deduplicate findings
        findings = self.deduplicate_findings(findings);

        let summary = self.generate_comprehensive_summary(&args.query, &findings, &sources);

        Ok(ResearchResult {
            query: args.query.clone(),
            summary,
            findings,
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
                "searxng" => self
                    .search_config
                    .searxng
                    .clone()
                    .ok_or_else(|| anyhow!("SearXNG not configured"))?,
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
        let mut key_points = Vec::new();

        // Split content into sentences
        for sentence in content.split(['.', '!', '?']) {
            let sentence = sentence.trim();
            if sentence.len() < 20 || sentence.len() > 500 {
                continue;
            }

            let sentence_lower = sentence.to_lowercase();

            // Check if sentence contains query terms
            let relevance: usize = query_words
                .iter()
                .filter(|w| sentence_lower.contains(w.as_str()))
                .count();

            if relevance >= (query_words.len() / 2).max(1) {
                key_points.push(sentence.to_string());
            }
        }

        // Limit to most relevant points
        key_points.truncate(5);
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
        findings: &[Finding],
        sources: &[Source],
    ) -> String {
        if findings.is_empty() {
            return format!("No relevant information found for: {}", query);
        }

        let avg_confidence: f32 =
            findings.iter().map(|f| f.confidence).sum::<f32>() / findings.len() as f32;

        let avg_reliability: f32 =
            sources.iter().map(|s| s.reliability).sum::<f32>() / sources.len().max(1) as f32;

        let key_findings: Vec<String> = findings
            .iter()
            .take(5)
            .enumerate()
            .map(|(i, f)| format!("{}. {}", i + 1, f.content))
            .collect();

        format!(
            "# Research Summary: {}\n\n\
            **Sources analyzed:** {}\n\
            **Average confidence:** {:.0}%\n\
            **Average source reliability:** {:.0}%\n\n\
            ## Key Findings\n\
            {}",
            query,
            sources.len(),
            avg_confidence * 100.0,
            avg_reliability * 100.0,
            key_findings.join("\n")
        )
    }

    /// Generate sub-queries for deep research
    fn generate_sub_queries(&self, query: &str) -> Vec<String> {
        let base_query = query.to_string();
        vec![
            base_query.clone(),
            format!("{} overview", query),
            format!("{} examples", query),
            format!("{} tutorial", query),
            format!("how does {} work", query),
        ]
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
            let is_duplicate = unique
                .iter()
                .any(|existing| self.similarity(&existing.content, &finding.content) > 0.7);

            if !is_duplicate {
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

    if !result.findings.is_empty() {
        output.push_str("## Detailed Findings\n\n");
        for (i, finding) in result.findings.iter().enumerate() {
            output.push_str(&format!(
                "{}. {} (confidence: {:.0}%, source: {})\n\n",
                i + 1,
                finding.content,
                finding.confidence * 100.0,
                finding.source_index + 1
            ));
        }
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
