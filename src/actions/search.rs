//! Web Search Actions
//!
//! Supports multiple search backends:
//! - Serper API (Google results)
//! - Brave Search API
//! - DuckDuckGo (scraping, no API key needed)

use anyhow::{Result, anyhow};
use chrono::{DateTime, Datelike, Duration as ChronoDuration, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

static HTML_TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<[^>]+>").unwrap());
static HTML_ANCHOR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?is)<a\b(?P<attrs>[^>]*)>(?P<body>.*?)</a>").unwrap());
static HTML_GENERIC_NODE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?is)<(?:span|div|a|p)\b(?P<attrs>[^>]*)>(?P<body>.*?)</(?:span|div|a|p)>")
        .unwrap()
});
static LEADING_SNIPPET_DATE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?ix)
        ^\s*
        (?P<date>
            \d{4}-\d{2}-\d{2}
            |
            \d{1,2}/\d{1,2}/\d{2,4}
            |
            (?:jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|jun(?:e)?|jul(?:y)?|aug(?:ust)?|sep(?:t(?:ember)?)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)
            \s+\d{1,2},\s+\d{4}
            |
            \d+\s+(?:minute|minutes|hour|hours|day|days|week|weeks|month|months|year|years)\s+ago
        )
        \s*(?:[|:-]|[·•])\s*
        (?P<rest>.+)
        $
        ",
    )
    .unwrap()
});
static HTML_CLASS_ATTR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?is)\bclass\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).unwrap());
static HTML_HREF_ATTR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?is)\bhref\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).unwrap());
static QUERY_YEAR_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b(?:19|20)\d{2}\b").unwrap());
static RSS_ITEM_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?is)<item\b[^>]*>(?P<body>.*?)</item>").unwrap());

const SEARCH_HTTP_ATTEMPTS: usize = 3;
const SEARCH_HTTP_RETRY_BASE_MS: u64 = 400;
const SEARCH_HTML_ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";
const SEARCH_XML_ACCEPT: &str = "application/rss+xml,application/xml,text/xml;q=0.9,*/*;q=0.8";
const SEARCH_BACKEND_HEALTH_KEY: &str = "search_backend_health:v1";
const BUILTIN_BACKEND_COOLDOWN_HOURS: i64 = 24;

pub const SEARCH_PROVIDER_SETUP_REQUIRED_MESSAGE: &str = "No search backend is currently available in AgentArk right now. AgentArk includes free anonymous DuckDuckGo/browser search, but it is best-effort and not always reliable; anonymous HTML/browser search may be blocked or challenged. Configure a reachable SearXNG instance or an API-backed search provider like Serper, Brave, Exa, Tavily, Perplexity, or Firecrawl for reliable live search.";

fn char_prefix(value: &str, max_chars: usize) -> &str {
    if value.chars().count() <= max_chars {
        return value;
    }
    let end = value
        .char_indices()
        .nth(max_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(value.len());
    &value[..end]
}

/// Search result from any backend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_date: Option<String>,
}

/// Search response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub backend: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SearchBackendHealthSnapshot {
    #[serde(default)]
    backends: BTreeMap<String, SearchBackendHealthRecord>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SearchBackendHealthRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cooldown_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_failure_at: Option<String>,
}

#[derive(Clone, Default)]
pub struct SearchBackendHealthState {
    snapshot: Arc<RwLock<SearchBackendHealthSnapshot>>,
    storage: Option<crate::storage::Storage>,
}

impl std::fmt::Debug for SearchBackendHealthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchBackendHealthState")
            .field("has_storage", &self.storage.is_some())
            .finish()
    }
}

impl SearchBackendHealthState {
    pub async fn load(storage: Option<&crate::storage::Storage>) -> Self {
        let mut snapshot = if let Some(store) = storage {
            store
                .get(SEARCH_BACKEND_HEALTH_KEY)
                .await
                .ok()
                .flatten()
                .and_then(|bytes| {
                    serde_json::from_slice::<SearchBackendHealthSnapshot>(&bytes).ok()
                })
                .unwrap_or_default()
        } else {
            SearchBackendHealthSnapshot::default()
        };
        snapshot.prune_expired();
        Self {
            snapshot: Arc::new(RwLock::new(snapshot)),
            storage: storage.cloned(),
        }
    }

    pub fn is_in_cooldown(&self, backend: &str) -> bool {
        self.cooldown_until(backend)
            .map(|deadline| deadline > Utc::now())
            .unwrap_or(false)
    }

    pub fn cooldown_until(&self, backend: &str) -> Option<DateTime<Utc>> {
        let guard = self.snapshot.read().ok()?;
        guard
            .backends
            .get(backend)
            .and_then(|entry| entry.cooldown_until.as_deref())
            .and_then(parse_rfc3339_utc)
            .filter(|deadline| *deadline > Utc::now())
    }

    pub async fn mark_cooldown(&self, backend: &str, error: &str) -> Result<()> {
        let snapshot = {
            let mut guard = self
                .snapshot
                .write()
                .map_err(|_| anyhow!("search health state lock poisoned"))?;
            guard.backends.insert(
                backend.to_string(),
                SearchBackendHealthRecord {
                    cooldown_until: Some(
                        (Utc::now() + ChronoDuration::hours(BUILTIN_BACKEND_COOLDOWN_HOURS))
                            .to_rfc3339(),
                    ),
                    last_error: Some(error.trim().to_string()),
                    last_failure_at: Some(Utc::now().to_rfc3339()),
                },
            );
            guard.clone()
        };
        self.persist_snapshot(&snapshot).await
    }

    pub async fn clear(&self, backend: &str) -> Result<()> {
        let snapshot = {
            let mut guard = self
                .snapshot
                .write()
                .map_err(|_| anyhow!("search health state lock poisoned"))?;
            if guard.backends.remove(backend).is_none() {
                return Ok(());
            }
            guard.clone()
        };
        self.persist_snapshot(&snapshot).await
    }

    async fn persist_snapshot(&self, snapshot: &SearchBackendHealthSnapshot) -> Result<()> {
        let Some(storage) = self.storage.as_ref() else {
            return Ok(());
        };
        let bytes = serde_json::to_vec(snapshot)?;
        storage.set(SEARCH_BACKEND_HEALTH_KEY, &bytes).await
    }
}

impl SearchBackendHealthSnapshot {
    fn prune_expired(&mut self) {
        let now = Utc::now();
        self.backends.retain(|_, record| {
            record
                .cooldown_until
                .as_deref()
                .and_then(parse_rfc3339_utc)
                .map(|deadline| deadline > now)
                .unwrap_or(false)
        });
    }
}

fn parse_rfc3339_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
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
        "lightpanda" => "Lightpanda",
        "duckduckgo" => "DuckDuckGo",
        "bing_rss" => "Bing RSS",
        "playwright" => "Playwright",
        _ => "search backend",
    }
}

/// Search backend configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SearchBackend {
    /// Serper API (Google results)
    Serper { api_key: String },
    /// Brave Search API
    Brave { api_key: String },
    /// Exa Search API
    Exa { api_key: String },
    /// Tavily Search API
    Tavily { api_key: String },
    /// Perplexity Search API
    Perplexity { api_key: String },
    /// Firecrawl Search API
    Firecrawl { api_key: String },
    /// SearXNG instance
    Searxng { base_url: String },
    /// DuckDuckGo (no API key, uses HTML scraping)
    DuckDuckGo,
    /// Playwright browser automation (headless Chromium via bridge sidecar)
    Playwright { bridge_url: String },
    /// Lightpanda fast headless browser (CLI-based, no sidecar needed)
    Lightpanda,
    /// Bing RSS feed results (no API key needed)
    BingRss,
}

/// Web search client
pub struct SearchClient {
    backend: SearchBackend,
    client: reqwest::Client,
}

impl SearchClient {
    pub fn new(backend: SearchBackend) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .expect("Failed to create HTTP client");

        Self { backend, client }
    }

    /// Perform a web search
    pub async fn search(&self, query: &str, num_results: usize) -> Result<SearchResponse> {
        self.search_with_scope(query, num_results, None).await
    }

    /// Perform a web search with an optional semantic temporal scope supplied by the planner.
    pub async fn search_with_scope(
        &self,
        query: &str,
        num_results: usize,
        time_scope: Option<SearchTimeScope>,
    ) -> Result<SearchResponse> {
        let fresh_results = scope_requests_fresh_results(time_scope);
        match &self.backend {
            SearchBackend::Serper { api_key } => {
                self.search_serper(api_key, query, num_results).await
            }
            SearchBackend::Brave { api_key } => {
                self.search_brave(api_key, query, num_results).await
            }
            SearchBackend::Exa { api_key } => self.search_exa(api_key, query, num_results).await,
            SearchBackend::Tavily { api_key } => {
                self.search_tavily(api_key, query, num_results, fresh_results)
                    .await
            }
            SearchBackend::Perplexity { api_key } => {
                self.search_perplexity(api_key, query, num_results, fresh_results)
                    .await
            }
            SearchBackend::Firecrawl { api_key } => {
                self.search_firecrawl(api_key, query, num_results, fresh_results)
                    .await
            }
            SearchBackend::Searxng { base_url } => {
                self.search_searxng(base_url, query, num_results).await
            }
            SearchBackend::DuckDuckGo => self.search_duckduckgo(query, num_results).await,
            SearchBackend::Playwright { bridge_url } => {
                self.search_playwright(bridge_url, query, num_results).await
            }
            SearchBackend::Lightpanda => self.search_lightpanda(query, num_results).await,
            SearchBackend::BingRss => self.search_bing_rss(query, num_results).await,
        }
    }

    async fn get_text_with_retry(&self, url: &str, accept: &str) -> Result<String> {
        let mut last_err: Option<anyhow::Error> = None;

        for attempt in 0..SEARCH_HTTP_ATTEMPTS {
            let response = self
                .client
                .get(url)
                .header("Accept", accept)
                .header("Accept-Language", "en-US,en;q=0.9")
                .send()
                .await;

            match response {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        return Ok(response.text().await?);
                    }

                    let err = anyhow!("request to {} failed with HTTP {}", url, status);
                    if attempt + 1 < SEARCH_HTTP_ATTEMPTS
                        && (status.is_server_error() || status.as_u16() == 429)
                    {
                        last_err = Some(err);
                        tokio::time::sleep(Duration::from_millis(
                            SEARCH_HTTP_RETRY_BASE_MS * (attempt as u64 + 1),
                        ))
                        .await;
                        continue;
                    }

                    return Err(err);
                }
                Err(err) => {
                    let err = anyhow!(err);
                    if attempt + 1 < SEARCH_HTTP_ATTEMPTS
                        && should_retry_search_request(err.downcast_ref::<reqwest::Error>())
                    {
                        last_err = Some(err);
                        tokio::time::sleep(Duration::from_millis(
                            SEARCH_HTTP_RETRY_BASE_MS * (attempt as u64 + 1),
                        ))
                        .await;
                        continue;
                    }
                    return Err(err);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow!("request to {} failed", url)))
    }

    /// Search using Serper API (Google results)
    async fn search_serper(
        &self,
        api_key: &str,
        query: &str,
        num_results: usize,
    ) -> Result<SearchResponse> {
        #[derive(Serialize)]
        struct SerperRequest {
            q: String,
            num: usize,
            gl: String,
            hl: String,
        }

        #[derive(Deserialize)]
        struct SerperResponse {
            organic: Option<Vec<SerperResult>>,
        }

        #[derive(Deserialize)]
        struct SerperResult {
            title: String,
            link: String,
            snippet: Option<String>,
            #[serde(
                default,
                alias = "date",
                alias = "publishedDate",
                alias = "published_date"
            )]
            published_date: Option<String>,
        }

        let request = SerperRequest {
            q: query.to_string(),
            num: num_results,
            gl: "us".to_string(),
            hl: "en".to_string(),
        };

        let response: SerperResponse = self
            .client
            .post("https://google.serper.dev/search")
            .header("X-API-KEY", api_key)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?
            .json()
            .await?;

        let results = response
            .organic
            .unwrap_or_default()
            .into_iter()
            .map(|r| {
                let (snippet_date, snippet) =
                    split_snippet_leading_date(&r.snippet.unwrap_or_default());
                SearchResult {
                    title: r.title,
                    url: r.link,
                    snippet,
                    source: "serper".to_string(),
                    published_date: normalize_optional_date(r.published_date).or(snippet_date),
                }
            })
            .collect();

        Ok(SearchResponse {
            query: query.to_string(),
            results,
            backend: "serper".to_string(),
        })
    }

    /// Search using Brave Search API
    async fn search_brave(
        &self,
        api_key: &str,
        query: &str,
        num_results: usize,
    ) -> Result<SearchResponse> {
        #[derive(Deserialize)]
        struct BraveResponse {
            web: Option<BraveWebResults>,
        }

        #[derive(Deserialize)]
        struct BraveWebResults {
            results: Vec<BraveResult>,
        }

        #[derive(Deserialize)]
        struct BraveResult {
            title: String,
            url: String,
            description: Option<String>,
            #[serde(default, alias = "age", alias = "page_age", alias = "pageAge")]
            published_date: Option<String>,
        }

        let url = format!(
            "https://api.search.brave.com/res/v1/web/search?q={}&count={}&country=US&search_lang=en&ui_lang=en-US",
            urlencoding::encode(query),
            num_results
        );

        let response: BraveResponse = self
            .client
            .get(&url)
            .header("X-Subscription-Token", api_key)
            .header("Accept", "application/json")
            .send()
            .await?
            .json()
            .await?;

        let results = response
            .web
            .map(|w| w.results)
            .unwrap_or_default()
            .into_iter()
            .map(|r| {
                let (snippet_date, snippet) =
                    split_snippet_leading_date(&r.description.unwrap_or_default());
                SearchResult {
                    title: r.title,
                    url: r.url,
                    snippet,
                    source: "brave".to_string(),
                    published_date: normalize_optional_date(r.published_date).or(snippet_date),
                }
            })
            .collect();

        Ok(SearchResponse {
            query: query.to_string(),
            results,
            backend: "brave".to_string(),
        })
    }

    async fn search_exa(
        &self,
        api_key: &str,
        query: &str,
        num_results: usize,
    ) -> Result<SearchResponse> {
        #[derive(Serialize)]
        struct ExaRequest {
            query: String,
            #[serde(rename = "numResults")]
            num_results: usize,
            #[serde(rename = "type")]
            search_type: String,
            contents: serde_json::Value,
        }

        #[derive(Deserialize)]
        struct ExaResponse {
            results: Vec<ExaResult>,
        }

        #[derive(Deserialize)]
        struct ExaResult {
            title: Option<String>,
            url: String,
            text: Option<String>,
            summary: Option<String>,
            #[serde(default, rename = "publishedDate")]
            published_date: Option<String>,
            #[serde(default)]
            highlights: Vec<String>,
        }

        let request = ExaRequest {
            query: query.to_string(),
            num_results,
            search_type: "auto".to_string(),
            contents: serde_json::json!({
                "highlights": { "maxCharacters": 220 }
            }),
        };

        let response: ExaResponse = self
            .client
            .post("https://api.exa.ai/search")
            .header("x-api-key", api_key)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let results = response
            .results
            .into_iter()
            .map(|result| {
                let snippet = result
                    .highlights
                    .into_iter()
                    .find(|value| !value.trim().is_empty())
                    .or(result.summary)
                    .or(result.text)
                    .unwrap_or_default();
                let (snippet_date, snippet) = split_snippet_leading_date(&snippet);
                SearchResult {
                    title: result.title.unwrap_or_else(|| result.url.clone()),
                    url: result.url,
                    snippet,
                    source: "exa".to_string(),
                    published_date: normalize_optional_date(result.published_date).or(snippet_date),
                }
            })
            .collect();

        Ok(SearchResponse {
            query: query.to_string(),
            results,
            backend: "exa".to_string(),
        })
    }

    async fn search_tavily(
        &self,
        api_key: &str,
        query: &str,
        num_results: usize,
        fresh_results: bool,
    ) -> Result<SearchResponse> {
        #[derive(Serialize)]
        struct TavilyRequest {
            query: String,
            topic: String,
            search_depth: String,
            max_results: usize,
            include_answer: bool,
            include_raw_content: bool,
            include_images: bool,
            include_favicon: bool,
        }

        #[derive(Deserialize)]
        struct TavilyResponse {
            results: Vec<TavilyResult>,
        }

        #[derive(Deserialize)]
        struct TavilyResult {
            title: String,
            url: String,
            #[serde(default)]
            content: Option<String>,
            #[serde(default, alias = "published_date", alias = "date")]
            published_date: Option<String>,
        }

        let request = TavilyRequest {
            query: query.to_string(),
            topic: if fresh_results {
                "news".to_string()
            } else {
                "general".to_string()
            },
            search_depth: "basic".to_string(),
            max_results: num_results,
            include_answer: false,
            include_raw_content: false,
            include_images: false,
            include_favicon: false,
        };

        let response: TavilyResponse = self
            .client
            .post("https://api.tavily.com/search")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let results = response
            .results
            .into_iter()
            .map(|result| {
                let (snippet_date, snippet) =
                    split_snippet_leading_date(&result.content.unwrap_or_default());
                SearchResult {
                    title: result.title,
                    url: result.url,
                    snippet,
                    source: "tavily".to_string(),
                    published_date: normalize_optional_date(result.published_date).or(snippet_date),
                }
            })
            .collect();

        Ok(SearchResponse {
            query: query.to_string(),
            results,
            backend: "tavily".to_string(),
        })
    }

    async fn search_perplexity(
        &self,
        api_key: &str,
        query: &str,
        num_results: usize,
        fresh_results: bool,
    ) -> Result<SearchResponse> {
        #[derive(Serialize)]
        struct PerplexityRequest {
            query: String,
            max_results: usize,
            max_tokens: usize,
            max_tokens_per_page: usize,
            #[serde(skip_serializing_if = "Option::is_none")]
            search_recency_filter: Option<String>,
        }

        #[derive(Deserialize)]
        struct PerplexityResponse {
            results: Vec<PerplexityResult>,
        }

        #[derive(Deserialize)]
        struct PerplexityResult {
            title: String,
            url: String,
            #[serde(default)]
            snippet: Option<String>,
            #[serde(default)]
            date: Option<String>,
            #[serde(default)]
            last_updated: Option<String>,
        }

        let request = PerplexityRequest {
            query: query.to_string(),
            max_results: num_results.min(20),
            max_tokens: 10_000,
            max_tokens_per_page: 2048,
            search_recency_filter: fresh_results.then_some("month".to_string()),
        };

        let response: PerplexityResponse = self
            .client
            .post("https://api.perplexity.ai/search")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let results = response
            .results
            .into_iter()
            .map(|result| {
                let (snippet_date, snippet) =
                    split_snippet_leading_date(&result.snippet.unwrap_or_default());
                SearchResult {
                    title: result.title,
                    url: result.url,
                    snippet,
                    source: "perplexity".to_string(),
                    published_date: normalize_optional_date(result.date.or(result.last_updated))
                        .or(snippet_date),
                }
            })
            .collect();

        Ok(SearchResponse {
            query: query.to_string(),
            results,
            backend: "perplexity".to_string(),
        })
    }

    async fn search_firecrawl(
        &self,
        api_key: &str,
        query: &str,
        num_results: usize,
        fresh_results: bool,
    ) -> Result<SearchResponse> {
        #[derive(Serialize)]
        struct FirecrawlRequest {
            query: String,
            limit: usize,
            sources: Vec<String>,
        }

        #[derive(Deserialize)]
        struct FirecrawlResponse {
            success: bool,
            data: FirecrawlData,
        }

        #[derive(Deserialize, Default)]
        struct FirecrawlData {
            #[serde(default)]
            web: Vec<FirecrawlWebResult>,
            #[serde(default)]
            news: Vec<FirecrawlNewsResult>,
        }

        #[derive(Deserialize)]
        struct FirecrawlWebResult {
            title: Option<String>,
            url: String,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            markdown: Option<String>,
        }

        #[derive(Deserialize)]
        struct FirecrawlNewsResult {
            title: Option<String>,
            url: String,
            #[serde(default)]
            snippet: Option<String>,
            #[serde(default)]
            date: Option<String>,
        }

        let mut sources = vec!["web".to_string()];
        if fresh_results {
            sources.push("news".to_string());
        }
        let request = FirecrawlRequest {
            query: query.to_string(),
            limit: num_results,
            sources,
        };

        let response: FirecrawlResponse = self
            .client
            .post("https://api.firecrawl.dev/v2/search")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        if !response.success {
            return Err(anyhow!("Firecrawl search request was not successful"));
        }

        let mut results = Vec::new();
        for result in response.data.news.into_iter().take(num_results) {
            let snippet = result.snippet.unwrap_or_default();
            let (snippet_date, snippet) = split_snippet_leading_date(&snippet);
            results.push(SearchResult {
                title: result.title.unwrap_or_else(|| result.url.clone()),
                url: result.url,
                snippet,
                source: "firecrawl".to_string(),
                published_date: normalize_optional_date(result.date).or(snippet_date),
            });
        }
        if results.len() < num_results {
            for result in response
                .data
                .web
                .into_iter()
                .take(num_results.saturating_sub(results.len()))
            {
                let snippet = result.description.or(result.markdown).unwrap_or_default();
                let (snippet_date, snippet) = split_snippet_leading_date(&snippet);
                results.push(SearchResult {
                    title: result.title.unwrap_or_else(|| result.url.clone()),
                    url: result.url,
                    snippet,
                    source: "firecrawl".to_string(),
                    published_date: snippet_date,
                });
            }
        }

        Ok(SearchResponse {
            query: query.to_string(),
            results,
            backend: "firecrawl".to_string(),
        })
    }

    async fn search_searxng(
        &self,
        base_url: &str,
        query: &str,
        num_results: usize,
    ) -> Result<SearchResponse> {
        #[derive(Deserialize)]
        struct SearxngResponse {
            #[serde(default)]
            results: Vec<SearxngResult>,
        }

        #[derive(Deserialize)]
        struct SearxngResult {
            title: String,
            url: String,
            #[serde(default)]
            content: Option<String>,
            #[serde(default, rename = "publishedDate")]
            published_date_camel: Option<String>,
            #[serde(default)]
            published_date: Option<String>,
        }

        let url = format!(
            "{}/search?q={}&format=json&language=en-US",
            base_url.trim_end_matches('/'),
            urlencoding::encode(query)
        );
        let response: SearxngResponse = self
            .client
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let results = response
            .results
            .into_iter()
            .take(num_results)
            .map(|result| {
                let (snippet_date, snippet) =
                    split_snippet_leading_date(&result.content.unwrap_or_default());
                SearchResult {
                    title: result.title,
                    url: result.url,
                    snippet,
                    source: "searxng".to_string(),
                    published_date: normalize_optional_date(
                        result.published_date_camel.or(result.published_date),
                    )
                    .or(snippet_date),
                }
            })
            .collect();

        Ok(SearchResponse {
            query: query.to_string(),
            results,
            backend: "searxng".to_string(),
        })
    }

    async fn search_bing_rss(&self, query: &str, num_results: usize) -> Result<SearchResponse> {
        self.search_bing_rss_fallback(query, num_results, "bing_rss")
            .await
    }

    /// Search using DuckDuckGo (HTML scraping - no API key needed)
    async fn search_duckduckgo(&self, query: &str, num_results: usize) -> Result<SearchResponse> {
        let mut saw_no_results = false;
        let mut failures = Vec::new();

        for url in duckduckgo_search_urls(query) {
            let page = self.get_text_with_retry(&url, SEARCH_HTML_ACCEPT).await;
            match page {
                Ok(html) => {
                    let results = parse_duckduckgo_html_results(&html, num_results, "duckduckgo");
                    if !results.is_empty() {
                        return Ok(SearchResponse {
                            query: query.to_string(),
                            results,
                            backend: "duckduckgo".to_string(),
                        });
                    }
                    if let Some(error) = search_results_unavailable_reason("duckduckgo", &html) {
                        failures.push(error.to_string());
                    } else {
                        saw_no_results = true;
                    }
                }
                Err(err) => failures.push(err.to_string()),
            }
        }

        if saw_no_results && failures.is_empty() {
            return Ok(SearchResponse {
                query: query.to_string(),
                results: Vec::new(),
                backend: "duckduckgo".to_string(),
            });
        }

        Err(anyhow!(
            "DuckDuckGo backend failed: {}",
            failures.join("; ")
        ))
    }

    /// Search using Playwright browser automation (headless Chromium via bridge sidecar)
    async fn search_playwright(
        &self,
        bridge_url: &str,
        query: &str,
        num_results: usize,
    ) -> Result<SearchResponse> {
        self.search_playwright_browser(bridge_url, query, num_results)
            .await
    }

    async fn search_playwright_browser(
        &self,
        bridge_url: &str,
        query: &str,
        num_results: usize,
    ) -> Result<SearchResponse> {
        #[derive(Deserialize)]
        struct SessionResp {
            session_id: String,
        }

        #[derive(Deserialize)]
        struct ContentResp {
            title: Option<String>,
            url: Option<String>,
            body_text: Option<String>,
            elements: Option<Vec<ContentElement>>,
        }

        #[derive(Deserialize)]
        struct ContentElement {
            tag: String,
            #[serde(default)]
            text: String,
            #[serde(default)]
            href: String,
        }

        // Create a browser session
        let session: SessionResp = self
            .client
            .post(format!("{}/session", bridge_url))
            .send()
            .await?
            .error_for_status()
            .map_err(|e| anyhow!("Playwright bridge unavailable: {}", e))?
            .json()
            .await?;

        let sid = &session.session_id;

        // Navigate to Brave Search (reliable from VPS IPs, no captcha)
        let search_url = format!(
            "https://search.brave.com/search?q={}&count={}&country=us&search_lang=en&ui_lang=en-US",
            urlencoding::encode(query),
            num_results + 5
        );

        let nav_result = self
            .client
            .post(format!("{}/session/{}/navigate", bridge_url, sid))
            .json(&serde_json::json!({ "url": search_url }))
            .send()
            .await;

        if let Err(e) = nav_result {
            let _ = self
                .client
                .delete(format!("{}/session/{}", bridge_url, sid))
                .send()
                .await;
            return Err(anyhow!("Navigation failed: {}", e));
        }

        // Brief wait for page to settle
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // Get page content
        let content_result = self
            .client
            .get(format!("{}/session/{}/content", bridge_url, sid))
            .send()
            .await;

        // Close session regardless of result
        let _ = self
            .client
            .delete(format!("{}/session/{}", bridge_url, sid))
            .send()
            .await;

        let content: ContentResp = content_result?.error_for_status()?.json().await?;

        let title = content.title.unwrap_or_default();
        let rendered_url = content.url.unwrap_or_default();
        let body_text = content.body_text.unwrap_or_default();
        let elements = content.elements.unwrap_or_default();

        // Extract search results from page elements
        let mut results = Vec::new();

        // Parse links that look like search results (skip search engine internal links)
        for el in &elements {
            if el.tag != "a" || el.href.is_empty() || el.text.is_empty() {
                continue;
            }

            // Skip search engine internal links
            if el.href.contains("brave.com")
                || el.href.contains("google.com")
                || el.href.contains("bing.com")
                || el.href.starts_with("#")
                || el.href.starts_with("/")
            {
                continue;
            }

            // Must be an external http(s) link
            if !el.href.starts_with("http") {
                continue;
            }

            // Skip duplicates
            if results.iter().any(|r: &SearchResult| r.url == el.href) {
                continue;
            }

            let title = el.text.trim().to_string();
            if title.is_empty() || title.len() < 3 {
                continue;
            }

            // Try to find a snippet near this URL in the body text
            let (published_date, snippet) =
                split_snippet_leading_date(&Self::extract_snippet_near(&body_text, &title));

            results.push(SearchResult {
                title,
                url: el.href.clone(),
                snippet,
                source: "playwright".to_string(),
                published_date,
            });

            if results.len() >= num_results {
                break;
            }
        }

        if results.is_empty() {
            let page_summary = format!("{}\n{}\n{}", title, rendered_url, body_text);
            if let Some(error) = search_results_unavailable_reason("playwright", &page_summary) {
                return Err(error);
            }
            return Err(anyhow!(
                "Search backend 'playwright' rendered a page, but no recognizable result links were found"
            ));
        }

        Ok(SearchResponse {
            query: query.to_string(),
            results,
            backend: "playwright".to_string(),
        })
    }

    /// Extract a snippet from body text near a given title string
    fn extract_snippet_near(body_text: &str, title: &str) -> String {
        // Find the title (or a substring) in the body text
        let search_term = if title.chars().count() > 20 {
            char_prefix(title, 20)
        } else {
            title
        };
        if let Some(pos) = body_text.to_lowercase().find(&search_term.to_lowercase()) {
            // Take text after the title match, skip the title itself
            let after = &body_text[pos..];
            // Skip past the title
            let snippet_start = after.find('\n').map(|p| p + 1).unwrap_or(search_term.len());
            if snippet_start < after.len() {
                let snippet_text = &after[snippet_start..];
                // Take first ~200 chars, stop at sentence boundary
                let chunk = char_prefix(snippet_text, 200);
                let end = chunk.rfind(". ").map(|p| p + 1).unwrap_or(chunk.len());
                return chunk[..end].trim().to_string();
            }
        }
        String::new()
    }

    /// Search using Lightpanda + DuckDuckGo HTML
    async fn search_lightpanda(&self, query: &str, num_results: usize) -> Result<SearchResponse> {
        let mut saw_no_results = false;
        let mut failures = Vec::new();

        for url in duckduckgo_search_urls(query) {
            match crate::integrations::lightpanda::fetch_html(&url).await {
                Ok(html) => {
                    let results = parse_duckduckgo_html_results(&html, num_results, "lightpanda");
                    if !results.is_empty() {
                        return Ok(SearchResponse {
                            query: query.to_string(),
                            results,
                            backend: "lightpanda".to_string(),
                        });
                    }
                    if let Some(error) = search_results_unavailable_reason("lightpanda", &html) {
                        failures.push(error.to_string());
                    } else {
                        saw_no_results = true;
                    }
                }
                Err(err) => failures.push(err.to_string()),
            }
        }

        if saw_no_results && failures.is_empty() {
            return Ok(SearchResponse {
                query: query.to_string(),
                results: Vec::new(),
                backend: "lightpanda".to_string(),
            });
        }

        Err(anyhow!(
            "Lightpanda backend failed: {}",
            failures.join("; ")
        ))
    }

    async fn search_bing_rss_fallback(
        &self,
        query: &str,
        num_results: usize,
        backend: &str,
    ) -> Result<SearchResponse> {
        let url = format!(
            "https://www.bing.com/search?format=rss&cc=US&setlang=en-US&mkt=en-US&q={}",
            urlencoding::encode(query)
        );
        let xml = self.get_text_with_retry(&url, SEARCH_XML_ACCEPT).await?;
        let results = parse_bing_rss_results(&xml, num_results, &format!("{}_bing_rss", backend));
        if results.is_empty() {
            return Err(anyhow!(
                "Bing RSS fallback returned no recognizable results for '{}'",
                query
            ));
        }

        Ok(SearchResponse {
            query: query.to_string(),
            results,
            backend: backend.to_string(),
        })
    }
}

fn parse_duckduckgo_html_results(
    html: &str,
    num_results: usize,
    source: &str,
) -> Vec<SearchResult> {
    let matches = HTML_ANCHOR_RE.find_iter(html).collect::<Vec<_>>();
    let primary_results =
        collect_duckduckgo_anchor_results(html, &matches, num_results, source, true);
    if !primary_results.is_empty() {
        return primary_results;
    }
    collect_duckduckgo_anchor_results(html, &matches, num_results, source, false)
}

fn collect_duckduckgo_anchor_results(
    html: &str,
    matches: &[regex::Match<'_>],
    num_results: usize,
    source: &str,
    require_result_markers: bool,
) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut seen_urls = std::collections::HashSet::new();

    for (index, anchor_match) in matches.iter().enumerate() {
        if results.len() >= num_results {
            break;
        }

        let anchor_html = anchor_match.as_str();
        let Some(captures) = HTML_ANCHOR_RE.captures(anchor_html) else {
            continue;
        };
        let attrs = captures
            .name("attrs")
            .map(|value| value.as_str())
            .unwrap_or("");
        if require_result_markers && !anchor_attrs_look_like_duckduckgo_result(attrs) {
            continue;
        }

        let Some(raw_href) = extract_attr_value(attrs, &HTML_HREF_ATTR_RE) else {
            continue;
        };
        let Some(url) = decode_duckduckgo_result_url(&raw_href) else {
            continue;
        };
        let title = captures
            .name("body")
            .map(|value| strip_html_tags(&html_decode(value.as_str())))
            .unwrap_or_default();
        if title.trim().is_empty() {
            continue;
        }
        if !require_result_markers && !looks_like_generic_search_result_title(&title) {
            continue;
        }
        if !seen_urls.insert(url.clone()) {
            continue;
        }

        let next_anchor_start = matches
            .get(index + 1)
            .map(|candidate| candidate.start())
            .unwrap_or(html.len());
        let between_anchors = &html[anchor_match.end()..next_anchor_start];
        let snippet = {
            let extracted = extract_duckduckgo_snippet(between_anchors);
            if extracted.trim().is_empty() {
                extract_generic_result_snippet(between_anchors)
            } else {
                extracted
            }
        };
        let (published_date, snippet) = split_snippet_leading_date(&snippet);

        results.push(SearchResult {
            title,
            url,
            snippet,
            source: source.to_string(),
            published_date,
        });
    }

    results
}

fn looks_like_generic_search_result_title(title: &str) -> bool {
    let normalized = title.trim().to_ascii_lowercase();
    if normalized.len() < 8 {
        return false;
    }
    if [
        "all", "news", "videos", "images", "maps", "shopping", "more", "next", "previous",
        "feedback", "privacy", "help", "settings", "sign in",
    ]
    .contains(&normalized.as_str())
    {
        return false;
    }
    normalized.split_whitespace().count() >= 2 || normalized.len() >= 18
}

fn extract_duckduckgo_snippet(html_after_result: &str) -> String {
    let preview = char_prefix(html_after_result, 4_000);
    for captures in HTML_GENERIC_NODE_RE.captures_iter(preview) {
        let attrs = captures
            .name("attrs")
            .map(|value| value.as_str())
            .unwrap_or("");
        if !attrs_have_class_token(attrs, "result__snippet")
            && !attrs_have_class_token(attrs, "result-snippet")
        {
            continue;
        }
        let body = captures
            .name("body")
            .map(|value| value.as_str())
            .unwrap_or("");
        let snippet = strip_html_tags(&html_decode(body));
        if !snippet.trim().is_empty() {
            return snippet;
        }
    }

    String::new()
}

fn extract_generic_result_snippet(html_after_result: &str) -> String {
    let preview = char_prefix(html_after_result, 2_000);
    let plain = strip_html_tags(&html_decode(preview));
    let compact = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() < 24 {
        return String::new();
    }
    char_prefix(&compact, 220).trim().to_string()
}

fn decode_duckduckgo_result_url(raw_href: &str) -> Option<String> {
    let trimmed = raw_href.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }

    let normalized = if trimmed.starts_with("//") {
        format!("https:{}", trimmed)
    } else {
        trimmed.to_string()
    };

    if let Some(redirect_start) = normalized.find("uddg=") {
        let encoded = &normalized[redirect_start + 5..];
        let target = encoded.split('&').next().unwrap_or(encoded);
        if let Ok(decoded) = urlencoding::decode(target) {
            let decoded = decoded.trim();
            if decoded.starts_with("http://") || decoded.starts_with("https://") {
                return Some(decoded.to_string());
            }
        }
    }

    if normalized.starts_with("http://") || normalized.starts_with("https://") {
        if normalized.contains("duckduckgo.com/") {
            return None;
        }
        return Some(normalized);
    }

    None
}

fn search_results_unavailable_reason(backend: &str, html: &str) -> Option<anyhow::Error> {
    let lower = html.to_ascii_lowercase();
    if lower.contains("no results") || lower.contains("did not match any documents") {
        None
    } else if lower.contains("captcha")
        || lower.contains("unusual traffic")
        || lower.contains("automated requests")
        || lower.contains("anomaly")
        || lower.contains("verify you are human")
        || lower.contains("unfortunately, bots use duckduckgo too")
        || lower.contains("confirm this search was made by a human")
        || lower.contains("proof of work captcha")
        || lower.contains("confirm you’re a human being")
        || lower.contains("confirm you're a human being")
        || lower.contains("one last step")
        || lower.contains("please solve the challenge below to continue")
        || lower.contains("i'm not a robot")
    {
        Some(anyhow!(
            "Search backend '{}' received an anti-bot or challenge page instead of results",
            backend
        ))
    } else {
        Some(anyhow!(
            "Search backend '{}' returned HTML, but no recognizable result links were found",
            backend
        ))
    }
}

fn anchor_attrs_look_like_duckduckgo_result(attrs: &str) -> bool {
    attrs_have_class_token(attrs, "result__a")
        || attrs_have_class_token(attrs, "result-link")
        || attrs
            .to_ascii_lowercase()
            .contains("data-testid=\"result-title-a\"")
        || attrs
            .to_ascii_lowercase()
            .contains("data-testid='result-title-a'")
}

fn attrs_have_class_token(attrs: &str, token: &str) -> bool {
    extract_attr_value(attrs, &HTML_CLASS_ATTR_RE)
        .map(|value| {
            value
                .split_whitespace()
                .any(|candidate| candidate.eq_ignore_ascii_case(token))
        })
        .unwrap_or(false)
}

fn extract_attr_value(attrs: &str, attr_re: &Regex) -> Option<String> {
    let captures = attr_re.captures(attrs)?;
    captures
        .get(1)
        .or_else(|| captures.get(2))
        .or_else(|| captures.get(3))
        .map(|value| value.as_str().trim().to_string())
        .filter(|value| !value.is_empty())
}

fn strip_html_tags(s: &str) -> String {
    HTML_TAG_RE.replace_all(s, "").trim().to_string()
}

fn normalize_optional_date(value: Option<String>) -> Option<String> {
    value
        .map(|date| {
            date.trim()
                .trim_matches(|c| c == '-' || c == '|' || c == ':')
                .trim()
                .to_string()
        })
        .filter(|date| !date.is_empty())
}

fn split_snippet_leading_date(snippet: &str) -> (Option<String>, String) {
    let trimmed = snippet.trim();
    if trimmed.is_empty() {
        return (None, String::new());
    }
    if let Some(captures) = LEADING_SNIPPET_DATE_RE.captures(trimmed) {
        let date = captures
            .name("date")
            .map(|value| value.as_str().trim().to_string())
            .filter(|value| !value.is_empty());
        let rest = captures
            .name("rest")
            .map(|value| value.as_str().trim().to_string())
            .unwrap_or_else(|| trimmed.to_string());
        return (date, rest);
    }
    (None, trimmed.to_string())
}

/// Simple HTML entity decoder
fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("<b>", "")
        .replace("</b>", "")
        .replace("<span>", "")
        .replace("</span>", "")
        .trim()
        .to_string()
}

fn should_retry_search_request(err: Option<&reqwest::Error>) -> bool {
    err.map(|err| err.is_connect() || err.is_timeout() || err.is_request())
        .unwrap_or(false)
}

fn duckduckgo_search_urls(query: &str) -> Vec<String> {
    let encoded = urlencoding::encode(query);
    vec![
        format!("https://html.duckduckgo.com/html/?kl=us-en&q={}", encoded),
        format!("https://lite.duckduckgo.com/lite/?kl=us-en&q={}", encoded),
    ]
}

fn parse_bing_rss_results(xml: &str, num_results: usize, source: &str) -> Vec<SearchResult> {
    let mut results = Vec::new();

    for captures in RSS_ITEM_RE.captures_iter(xml) {
        if results.len() >= num_results {
            break;
        }
        let body = captures
            .name("body")
            .map(|value| value.as_str())
            .unwrap_or("");
        let title = extract_xml_tag(body, "title");
        let url = extract_xml_tag(body, "link");
        if title.trim().is_empty() || url.trim().is_empty() {
            continue;
        }
        let description = extract_xml_tag(body, "description");
        let published_date = normalize_optional_date(Some(extract_xml_tag(body, "pubDate")));
        results.push(SearchResult {
            title: html_decode(&strip_html_tags(&title)),
            url: html_decode(&url),
            snippet: html_decode(&strip_html_tags(&description)),
            source: source.to_string(),
            published_date,
        });
    }

    results
}

fn extract_xml_tag(xml: &str, tag: &str) -> String {
    let escaped_tag = regex::escape(tag.trim());
    let pattern = format!(
        r"(?is)<{tag}\b[^>]*>(?P<body>.*?)</{tag}>",
        tag = escaped_tag
    );

    Regex::new(&pattern)
        .ok()
        .and_then(|re| re.captures(xml))
        .and_then(|captures| captures.name("body"))
        .map(|value| value.as_str().trim().to_string())
        .unwrap_or_default()
}

/// Search action arguments
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchTimeScope {
    Current,
    Recent,
    Historical,
    Timeless,
}

fn scope_requests_fresh_results(time_scope: Option<SearchTimeScope>) -> bool {
    matches!(
        time_scope,
        Some(SearchTimeScope::Current | SearchTimeScope::Recent)
    )
}

#[derive(Debug, Deserialize)]
pub struct SearchArgs {
    pub query: String,
    #[serde(default = "default_num_results")]
    pub num_results: usize,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub time_scope: Option<SearchTimeScope>,
}

fn default_num_results() -> usize {
    5
}

fn normalize_freshness_query(raw_query: &str, time_scope: Option<SearchTimeScope>) -> String {
    let compact = raw_query.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() || !scope_requests_fresh_results(time_scope) {
        return compact;
    }

    let current_year = chrono::Utc::now().year();
    let distinct_years = QUERY_YEAR_RE
        .find_iter(&compact)
        .filter_map(|m| m.as_str().parse::<i32>().ok())
        .collect::<std::collections::BTreeSet<_>>();

    if !distinct_years.is_empty() {
        return compact;
    }

    format!("{} {}", compact, current_year)
}

const DEFAULT_CONFIGURED_PROVIDER_ORDER: &[&str] = &[
    "serper",
    "brave_api",
    "exa",
    "tavily",
    "perplexity",
    "firecrawl",
    "searxng",
];
const DEFAULT_FREE_BACKEND_ORDER: &[&str] = &["duckduckgo", "lightpanda", "playwright", "bing_rss"];

/// Execute a web search
pub async fn execute_search(args: &SearchArgs, config: &SearchConfig) -> Result<String> {
    let response = execute_search_response(args, config).await?;
    Ok(format_search_results(&response))
}

/// Execute a web search and return the structured response for callers that
/// need machine-readable fallback behavior.
pub async fn execute_search_response(
    args: &SearchArgs,
    config: &SearchConfig,
) -> Result<SearchResponse> {
    let response = search_with_config(
        &args.query,
        args.num_results.max(1),
        args.backend.as_deref(),
        config,
        args.time_scope,
    )
    .await?;
    Ok(response)
}

fn markdown_inline_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('*', "\\*")
        .replace('_', "\\_")
        .replace('`', "\\`")
}

fn markdown_link_target(value: &str) -> String {
    value.trim().replace(' ', "%20").replace(')', "%29")
}

fn search_result_display_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Format search results into a human-readable markdown list.
pub fn format_search_results(response: &SearchResponse) -> String {
    let mut output = format!(
        "Search results for: **{}**\n\n",
        markdown_inline_text(&search_result_display_text(&response.query))
    );
    for result in &response.results {
        let title = search_result_display_text(&result.title);
        let url = markdown_link_target(&result.url);
        let source = search_result_display_text(&result.source);
        let date = result
            .published_date
            .as_deref()
            .map(search_result_display_text)
            .filter(|value| !value.is_empty());
        let snippet = search_result_display_text(&result.snippet);
        let mut meta = Vec::new();
        if !source.is_empty() {
            meta.push(markdown_inline_text(&source));
        }
        if let Some(date) = date {
            meta.push(format!("Published: {}", markdown_inline_text(&date)));
        }

        if url.is_empty() {
            output.push_str(&format!("- **{}**", markdown_inline_text(&title)));
        } else {
            output.push_str(&format!(
                "- **[{}]({})**",
                markdown_inline_text(&title),
                url
            ));
        }
        if !meta.is_empty() {
            output.push_str(&format!("  \n  {}", meta.join(" . ")));
        }
        if !snippet.is_empty() {
            output.push_str(&format!("  \n  {}", markdown_inline_text(&snippet)));
        }
        output.push('\n');
    }
    output
}

fn default_lightpanda_available() -> bool {
    true
}

/// Search configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    pub serper: Option<SearchBackend>,
    pub brave: Option<SearchBackend>,
    pub exa: Option<SearchBackend>,
    pub tavily: Option<SearchBackend>,
    pub perplexity: Option<SearchBackend>,
    pub firecrawl: Option<SearchBackend>,
    pub searxng: Option<SearchBackend>,
    pub playwright: Option<SearchBackend>,
    #[serde(skip, default = "default_lightpanda_available")]
    pub lightpanda_available: bool,
    /// Preferred primary backend name (e.g. "lightpanda", "serper", "duckduckgo")
    #[serde(default)]
    pub primary: Option<String>,
    /// First fallback backend name
    #[serde(default)]
    pub fallback1: Option<String>,
    /// Second fallback backend name
    #[serde(default)]
    pub fallback2: Option<String>,
    #[serde(default)]
    pub provider_order: Vec<String>,
    #[serde(skip, default)]
    pub health: SearchBackendHealthState,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            serper: None,
            brave: None,
            exa: None,
            tavily: None,
            perplexity: None,
            firecrawl: None,
            searxng: None,
            playwright: None,
            lightpanda_available: default_lightpanda_available(),
            primary: None,
            fallback1: None,
            fallback2: None,
            provider_order: Vec::new(),
            health: SearchBackendHealthState::default(),
        }
    }
}

impl SearchConfig {
    pub fn with_health(mut self, health: SearchBackendHealthState) -> Self {
        self.health = health;
        self
    }

    /// Resolve a backend name to a configured SearchBackend instance
    pub fn resolve_backend(&self, name: &str) -> Option<SearchBackend> {
        match normalize_backend_name(name).as_str() {
            "playwright" => self.playwright.clone(),
            "serper" => self.serper.clone(),
            "brave_api" => self.brave.clone(),
            "exa" => self.exa.clone(),
            "tavily" => self.tavily.clone(),
            "perplexity" => self.perplexity.clone(),
            "firecrawl" => self.firecrawl.clone(),
            "searxng" => self.searxng.clone(),
            "bing_rss" => Some(SearchBackend::BingRss),
            "duckduckgo" => Some(SearchBackend::DuckDuckGo),
            "lightpanda" if self.lightpanda_available => Some(SearchBackend::Lightpanda),
            _ => None,
        }
    }

    fn push_unique_backend_name(ordered: &mut Vec<String>, name: &str) {
        let normalized = normalize_backend_name(name);
        if normalized.is_empty() || ordered.contains(&normalized) {
            return;
        }
        ordered.push(normalized);
    }

    fn preferred_configured_provider_names(&self) -> Vec<String> {
        let mut ordered = Vec::new();

        for name in &self.provider_order {
            let normalized = normalize_backend_name(name);
            if is_configurable_provider_backend(&normalized)
                && self.resolve_backend(&normalized).is_some()
            {
                Self::push_unique_backend_name(&mut ordered, &normalized);
            }
        }

        if ordered.is_empty() {
            for legacy_name in [
                self.primary.as_deref(),
                self.fallback1.as_deref(),
                self.fallback2.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                let normalized = normalize_backend_name(legacy_name);
                if is_configurable_provider_backend(&normalized)
                    && self.resolve_backend(&normalized).is_some()
                {
                    Self::push_unique_backend_name(&mut ordered, &normalized);
                }
            }
        }

        ordered
    }

    pub fn ordered_backend_names(&self) -> Vec<String> {
        let mut ordered = Vec::new();

        for name in self.preferred_configured_provider_names() {
            Self::push_unique_backend_name(&mut ordered, &name);
        }

        for name in DEFAULT_CONFIGURED_PROVIDER_ORDER {
            let normalized = normalize_backend_name(name);
            if self.resolve_backend(&normalized).is_some() {
                Self::push_unique_backend_name(&mut ordered, &normalized);
            }
        }

        for name in DEFAULT_FREE_BACKEND_ORDER {
            let normalized = normalize_backend_name(name);
            if self.resolve_backend(&normalized).is_some() {
                Self::push_unique_backend_name(&mut ordered, &normalized);
            }
        }

        ordered
    }

    pub fn ensure_default_chain(&mut self) {
        let mut normalized_order = Vec::new();
        for name in &self.provider_order {
            let normalized = normalize_backend_name(name);
            if is_configurable_provider_backend(&normalized) {
                Self::push_unique_backend_name(&mut normalized_order, &normalized);
            }
        }

        if normalized_order.is_empty() {
            for legacy_name in [
                self.primary.as_deref(),
                self.fallback1.as_deref(),
                self.fallback2.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                let normalized = normalize_backend_name(legacy_name);
                if is_configurable_provider_backend(&normalized) {
                    Self::push_unique_backend_name(&mut normalized_order, &normalized);
                }
            }
        }

        self.provider_order = normalized_order;
    }

    fn backend_attempt_chain(
        &self,
        explicit_backend: Option<&str>,
    ) -> (Vec<String>, Option<String>) {
        let explicit_backend_name = explicit_backend.map(normalize_backend_name);
        if let Some(backend_name) = explicit_backend_name.as_deref() {
            if self.resolve_backend(backend_name).is_some() {
                return (vec![backend_name.to_string()], None);
            }
            return (self.ordered_backend_names(), Some(backend_name.to_string()));
        }
        (self.ordered_backend_names(), None)
    }

    pub fn is_builtin_cooldown_backend(&self, name: &str) -> bool {
        matches!(
            normalize_backend_name(name).as_str(),
            "bing_rss" | "lightpanda" | "duckduckgo"
        )
    }
}

pub async fn search_with_config(
    raw_query: &str,
    num_results: usize,
    explicit_backend: Option<&str>,
    config: &SearchConfig,
    time_scope: Option<SearchTimeScope>,
) -> Result<SearchResponse> {
    let normalized_query = normalize_freshness_query(raw_query, time_scope);
    if normalized_query != raw_query {
        tracing::warn!(
            "Normalized freshness search query from '{}' to '{}'",
            raw_query,
            normalized_query
        );
    }

    let (chain, fallback_backend_name) = config.backend_attempt_chain(explicit_backend);
    let mut last_err = None;
    let mut attempts = Vec::new();
    let mut cooldowns = Vec::new();
    let mut search_available = false;

    if let Some(backend_name) = fallback_backend_name.as_deref() {
        if chain.is_empty() {
            return Err(anyhow!(
                "Search backend '{}' is not configured",
                backend_name
            ));
        }
        attempts.push(format!(
            "{}: not configured, falling back to automatic chain",
            backend_name
        ));
    }

    for name in &chain {
        let backend = config.resolve_backend(name);
        if config.is_builtin_cooldown_backend(name) && config.health.is_in_cooldown(name) {
            let cooldown_until = config
                .health
                .cooldown_until(name)
                .map(|value| value.to_rfc3339())
                .unwrap_or_else(|| "unknown".to_string());
            attempts.push(format!("{}: cooling down until {}", name, cooldown_until));
            cooldowns.push(format!(
                "{} until {}",
                backend_display_name(name),
                cooldown_until
            ));
            continue;
        }

        let Some(backend) = backend else {
            attempts.push(format!("{}: not configured", name));
            continue;
        };

        let client = SearchClient::new(backend);
        let search_result = if time_scope.is_some() {
            client
                .search_with_scope(&normalized_query, num_results, time_scope)
                .await
        } else {
            client.search(&normalized_query, num_results).await
        };
        match search_result {
            Ok(response) if !response.results.is_empty() => {
                if config.is_builtin_cooldown_backend(name) {
                    let _ = config.health.clear(name).await;
                }
                return Ok(response);
            }
            Ok(_) => {
                search_available = true;
                last_err = Some(anyhow!("Backend '{}' returned 0 results", name));
                attempts.push(format!("{}: returned 0 results", name));
            }
            Err(error) => {
                let error_text = error.to_string();
                let should_cooldown = config.is_builtin_cooldown_backend(name)
                    && search_error_is_cooldown_worthy(&error);
                if should_cooldown {
                    let _ = config.health.mark_cooldown(name, &error_text).await;
                    cooldowns.push(format!(
                        "{} until {}",
                        backend_display_name(name),
                        (Utc::now() + ChronoDuration::hours(BUILTIN_BACKEND_COOLDOWN_HOURS))
                            .to_rfc3339()
                    ));
                }
                attempts.push(if should_cooldown {
                    format!("{}: {} (cooldown started)", name, error_text)
                } else {
                    format!("{}: {}", name, error_text)
                });
                last_err = Some(error);
            }
        }
    }

    if !search_available {
        let mut detail = String::from(SEARCH_PROVIDER_SETUP_REQUIRED_MESSAGE);
        if !cooldowns.is_empty() {
            detail.push(' ');
            detail.push_str("Anonymous fallback cooldowns: ");
            detail.push_str(&cooldowns.join(", "));
            detail.push('.');
        }
        if !attempts.is_empty() {
            detail.push(' ');
            detail.push_str("Attempts: ");
            detail.push_str(&attempts.join("; "));
        }
        return Err(anyhow!(detail));
    }

    Err(if attempts.is_empty() {
        last_err.unwrap_or_else(|| anyhow!("All search backends failed"))
    } else {
        anyhow!("All search backends failed: {}", attempts.join("; "))
    })
}

fn normalize_backend_name(name: &str) -> String {
    match name.trim().to_ascii_lowercase().as_str() {
        "brave" => "brave_api".to_string(),
        other => other.to_string(),
    }
}

fn is_configurable_provider_backend(name: &str) -> bool {
    matches!(
        normalize_backend_name(name).as_str(),
        "serper" | "brave_api" | "exa" | "tavily" | "perplexity" | "firecrawl" | "searxng"
    )
}

fn search_error_is_cooldown_worthy(error: &anyhow::Error) -> bool {
    let lower = error.to_string().to_ascii_lowercase();
    lower.contains("anti-bot") || lower.contains("challenge page")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duckduckgo_results_decodes_redirect_links() {
        let html = r#"
            <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fstory%3Fid%3D42&rut=abc">Example result</a>
            <span class="result__snippet">A useful summary.</span>
        "#;

        let results = parse_duckduckgo_html_results(html, 5, "lightpanda");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Example result");
        assert_eq!(results[0].url, "https://example.com/story?id=42");
        assert_eq!(results[0].snippet, "A useful summary.");
        assert_eq!(results[0].source, "lightpanda");
    }

    #[test]
    fn parse_duckduckgo_results_skips_internal_duckduckgo_links() {
        let html = r#"
            <a class="result__a" href="https://duckduckgo.com/?q=agentark">Internal</a>
            <span class="result__snippet">Ignore me.</span>
            <a class="result__a" href="https://example.com/docs">Docs</a>
            <span class="result__snippet">Keep me.</span>
        "#;

        let results = parse_duckduckgo_html_results(html, 5, "duckduckgo");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Docs");
        assert_eq!(results[0].url, "https://example.com/docs");
        assert_eq!(results[0].snippet, "Keep me.");
    }

    #[test]
    fn parse_duckduckgo_results_handles_extra_classes_and_single_quotes() {
        let html = r#"
            <a class='result__a result-link' data-testid='result-title-a' href='https://example.com/report'>
                Example <b>report</b>
            </a>
            <div class='result__snippet result-snippet'>Strong <b>evidence</b> here.</div>
        "#;

        let results = parse_duckduckgo_html_results(html, 5, "lightpanda");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Example report");
        assert_eq!(results[0].url, "https://example.com/report");
        assert_eq!(results[0].snippet, "Strong evidence here.");
    }

    #[test]
    fn parse_duckduckgo_results_extracts_leading_publication_date() {
        let html = r#"
            <a class="result__a" href="https://example.com/report">Example report</a>
            <span class="result__snippet">April 5, 2026 - Strong evidence here.</span>
        "#;

        let results = parse_duckduckgo_html_results(html, 5, "duckduckgo");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].published_date.as_deref(), Some("April 5, 2026"));
        assert_eq!(results[0].snippet, "Strong evidence here.");
    }

    #[test]
    fn parse_duckduckgo_results_falls_back_to_generic_external_anchors() {
        let html = r#"
            <a href="https://example.com/policy/india-ai">India AI policy outlook 2026</a>
            <p>Government strategy, compute constraints, and university talent pipeline updates.</p>
        "#;

        let results = parse_duckduckgo_html_results(html, 5, "lightpanda");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "India AI policy outlook 2026");
        assert_eq!(results[0].url, "https://example.com/policy/india-ai");
        assert!(
            results[0]
                .snippet
                .contains("Government strategy, compute constraints")
        );
    }

    #[test]
    fn parse_bing_rss_results_extracts_items() {
        let xml = r#"
            <?xml version="1.0" encoding="utf-8" ?>
            <rss version="2.0">
              <channel>
                <item>
                  <title>Example report</title>
                  <link>https://example.com/report</link>
                  <description>Strong &amp; current evidence.</description>
                  <pubDate>Mon, 07 Apr 2026 10:00:00 GMT</pubDate>
                </item>
              </channel>
            </rss>
        "#;

        let results = parse_bing_rss_results(xml, 5, "duckduckgo_bing_rss");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Example report");
        assert_eq!(results[0].url, "https://example.com/report");
        assert_eq!(results[0].snippet, "Strong & current evidence.");
        assert_eq!(
            results[0].published_date.as_deref(),
            Some("Mon, 07 Apr 2026 10:00:00 GMT")
        );
    }

    #[test]
    fn extract_xml_tag_matches_case_insensitive_closing_tags_without_panicking() {
        let xml = r#"<TITLE>Example report</TITLE>"#;

        assert_eq!(extract_xml_tag(xml, "title"), "Example report");
    }

    #[test]
    fn extract_xml_tag_supports_namespaced_tags_without_backreference_regexes() {
        let xml = r#"<content:encoded>Encoded body</content:encoded>"#;

        assert_eq!(extract_xml_tag(xml, "content:encoded"), "Encoded body");
    }

    #[test]
    fn freshness_query_adds_current_year_when_user_did_not_scope_a_year() {
        let current_year = chrono::Utc::now().year();

        assert_eq!(
            normalize_freshness_query("search iran news", Some(SearchTimeScope::Current)),
            format!("search iran news {}", current_year)
        );
    }

    #[test]
    fn freshness_query_does_not_guess_scope_from_query_words() {
        assert_eq!(
            normalize_freshness_query("search iran news", None),
            "search iran news"
        );
    }

    #[test]
    fn freshness_query_preserves_explicit_historical_year_scope() {
        assert_eq!(
            normalize_freshness_query(
                "events on corona from march 2020",
                Some(SearchTimeScope::Historical)
            ),
            "events on corona from march 2020"
        );
        assert_eq!(
            normalize_freshness_query("iran news may 2025", Some(SearchTimeScope::Current)),
            "iran news may 2025"
        );
    }

    #[test]
    fn search_results_unavailable_reason_detects_duckduckgo_bot_challenge() {
        let html = "Unfortunately, bots use DuckDuckGo too. Please complete the following challenge to confirm this search was made by a human.";
        let reason = search_results_unavailable_reason("duckduckgo", html)
            .expect("duckduckgo challenge should be detected");
        assert!(reason.to_string().contains("anti-bot"));
    }

    #[test]
    fn search_results_unavailable_reason_detects_browser_challenge() {
        let html = "One last step. Please solve the challenge below to continue. Confirm you're a human being.";
        let reason = search_results_unavailable_reason("playwright", html)
            .expect("browser challenge should be detected");
        assert!(reason.to_string().contains("anti-bot"));
    }

    #[test]
    fn ordered_backend_names_prioritizes_configured_providers_then_free_fallbacks() {
        let config = SearchConfig {
            provider_order: vec!["tavily".to_string(), "serper".to_string()],
            serper: Some(SearchBackend::Serper {
                api_key: "test".to_string(),
            }),
            tavily: Some(SearchBackend::Tavily {
                api_key: "test".to_string(),
            }),
            ..SearchConfig::default()
        };

        let chain = config.ordered_backend_names();

        assert_eq!(
            chain,
            vec![
                "tavily".to_string(),
                "serper".to_string(),
                "duckduckgo".to_string(),
                "lightpanda".to_string(),
                "bing_rss".to_string()
            ]
        );
    }

    #[test]
    fn ordered_backend_names_uses_legacy_primary_and_fallbacks_when_provider_order_absent() {
        let config = SearchConfig {
            primary: Some("brave".to_string()),
            fallback1: Some("tavily".to_string()),
            brave: Some(SearchBackend::Brave {
                api_key: "test".to_string(),
            }),
            tavily: Some(SearchBackend::Tavily {
                api_key: "test".to_string(),
            }),
            ..SearchConfig::default()
        };

        let chain = config.ordered_backend_names();

        assert_eq!(chain[0], "brave_api");
        assert_eq!(chain[1], "tavily");
        assert!(chain.ends_with(&[
            "duckduckgo".to_string(),
            "lightpanda".to_string(),
            "bing_rss".to_string()
        ]));
    }

    #[test]
    fn ensure_default_chain_normalizes_provider_order_and_legacy_aliases() {
        let mut config = SearchConfig {
            primary: Some("brave".to_string()),
            fallback1: Some("serper".to_string()),
            provider_order: vec![
                " Brave ".to_string(),
                "serper".to_string(),
                "brave_api".to_string(),
                String::new(),
            ],
            ..SearchConfig::default()
        };

        config.ensure_default_chain();

        assert_eq!(
            config.provider_order,
            vec!["brave_api".to_string(), "serper".to_string()]
        );
    }

    #[test]
    fn backend_attempt_chain_falls_back_from_unconfigured_explicit_provider() {
        let config = SearchConfig::default();

        let (chain, fallback_backend_name) = config.backend_attempt_chain(Some("brave"));

        assert_eq!(fallback_backend_name.as_deref(), Some("brave_api"));
        assert_eq!(
            chain,
            vec![
                "duckduckgo".to_string(),
                "lightpanda".to_string(),
                "bing_rss".to_string()
            ]
        );
    }

    #[test]
    fn backend_attempt_chain_keeps_configured_explicit_provider() {
        let config = SearchConfig {
            serper: Some(SearchBackend::Serper {
                api_key: "test".to_string(),
            }),
            ..SearchConfig::default()
        };

        let (chain, fallback_backend_name) = config.backend_attempt_chain(Some("serper"));

        assert_eq!(chain, vec!["serper".to_string()]);
        assert!(fallback_backend_name.is_none());
    }

    #[test]
    fn ordered_backend_names_skips_lightpanda_when_runtime_marks_it_unavailable() {
        let config = SearchConfig {
            lightpanda_available: false,
            ..SearchConfig::default()
        };

        let chain = config.ordered_backend_names();

        assert_eq!(
            chain,
            vec!["duckduckgo".to_string(), "bing_rss".to_string()]
        );
    }

    #[test]
    fn format_search_results_includes_dates_when_available() {
        let rendered = format_search_results(&SearchResponse {
            query: "india israel news".to_string(),
            results: vec![SearchResult {
                title: "Example".to_string(),
                url: "https://example.com/story".to_string(),
                snippet: "Summary".to_string(),
                source: "duckduckgo".to_string(),
                published_date: Some("2026-04-06".to_string()),
            }],
            backend: "duckduckgo".to_string(),
        });

        assert!(rendered.contains("Published: 2026-04-06"));
        assert!(rendered.contains("Summary"));
        assert!(!rendered.contains("\n   Date:"));
        assert!(!rendered.contains("\n1."));
    }
}
