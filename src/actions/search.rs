//! Web Search Actions
//!
//! Supports multiple search backends:
//! - SearXNG (self-hosted, reliable)
//! - Serper API (Google results)
//! - Brave Search API
//! - DuckDuckGo (scraping, no API key needed)

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// Search result from any backend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub source: String,
}

/// Search response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub backend: String,
}

/// Search backend configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SearchBackend {
    /// SearXNG self-hosted instance
    SearXNG { base_url: String },
    /// Serper API (Google results)
    Serper { api_key: String },
    /// Brave Search API
    Brave { api_key: String },
    /// DuckDuckGo (no API key, uses HTML scraping)
    DuckDuckGo,
    /// Playwright browser automation (headless Chromium via bridge sidecar)
    Playwright { bridge_url: String },
    /// Lightpanda fast headless browser (CLI-based, no sidecar needed)
    Lightpanda,
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
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .expect("Failed to create HTTP client");

        Self { backend, client }
    }

    /// Perform a web search
    pub async fn search(&self, query: &str, num_results: usize) -> Result<SearchResponse> {
        match &self.backend {
            SearchBackend::SearXNG { base_url } => {
                self.search_searxng(base_url, query, num_results).await
            }
            SearchBackend::Serper { api_key } => {
                self.search_serper(api_key, query, num_results).await
            }
            SearchBackend::Brave { api_key } => {
                self.search_brave(api_key, query, num_results).await
            }
            SearchBackend::DuckDuckGo => self.search_duckduckgo(query, num_results).await,
            SearchBackend::Playwright { bridge_url } => {
                self.search_playwright(bridge_url, query, num_results).await
            }
            SearchBackend::Lightpanda => self.search_lightpanda(query, num_results).await,
        }
    }

    /// Search using SearXNG instance
    async fn search_searxng(
        &self,
        base_url: &str,
        query: &str,
        num_results: usize,
    ) -> Result<SearchResponse> {
        #[derive(Deserialize)]
        struct SearXNGResponse {
            results: Vec<SearXNGResult>,
        }

        #[derive(Deserialize)]
        struct SearXNGResult {
            title: String,
            url: String,
            content: Option<String>,
        }

        let url = format!(
            "{}/search?q={}&format=json&categories=general",
            base_url.trim_end_matches('/'),
            urlencoding::encode(query)
        );

        let response: SearXNGResponse = self.client.get(&url).send().await?.json().await?;

        let results = response
            .results
            .into_iter()
            .take(num_results)
            .map(|r| SearchResult {
                title: r.title,
                url: r.url,
                snippet: r.content.unwrap_or_default(),
                source: "searxng".to_string(),
            })
            .collect();

        Ok(SearchResponse {
            query: query.to_string(),
            results,
            backend: "searxng".to_string(),
        })
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
        }

        let request = SerperRequest {
            q: query.to_string(),
            num: num_results,
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
            .map(|r| SearchResult {
                title: r.title,
                url: r.link,
                snippet: r.snippet.unwrap_or_default(),
                source: "serper".to_string(),
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
        }

        let url = format!(
            "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
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
            .map(|r| SearchResult {
                title: r.title,
                url: r.url,
                snippet: r.description.unwrap_or_default(),
                source: "brave".to_string(),
            })
            .collect();

        Ok(SearchResponse {
            query: query.to_string(),
            results,
            backend: "brave".to_string(),
        })
    }

    /// Search using DuckDuckGo (HTML scraping - no API key needed)
    async fn search_duckduckgo(&self, query: &str, num_results: usize) -> Result<SearchResponse> {
        // DuckDuckGo HTML search
        let url = format!(
            "https://html.duckduckgo.com/html/?q={}",
            urlencoding::encode(query)
        );

        let html = self.client.get(&url).send().await?.text().await?;

        // Simple HTML parsing for results
        let mut results = Vec::new();

        // Look for result divs - basic regex-style parsing
        // In production, use a proper HTML parser like scraper
        let mut remaining = html.as_str();

        while results.len() < num_results {
            // Find result link
            let Some(link_start) = remaining.find("class=\"result__a\"") else {
                break;
            };
            remaining = &remaining[link_start..];

            let Some(href_start) = remaining.find("href=\"") else {
                break;
            };
            remaining = &remaining[href_start + 6..];

            let Some(href_end) = remaining.find('"') else {
                break;
            };
            let url = &remaining[..href_end];
            remaining = &remaining[href_end..];

            // Get title
            let Some(title_start) = remaining.find('>') else {
                break;
            };
            remaining = &remaining[title_start + 1..];

            let Some(title_end) = remaining.find("</a>") else {
                break;
            };
            let title = html_decode(&remaining[..title_end]);
            remaining = &remaining[title_end..];

            // Get snippet
            let snippet = if let Some(snippet_start) = remaining.find("class=\"result__snippet\"") {
                let temp = &remaining[snippet_start..];
                if let Some(s_start) = temp.find('>') {
                    let temp = &temp[s_start + 1..];
                    if let Some(s_end) = temp.find("</a>").or_else(|| temp.find("</span>")) {
                        html_decode(&temp[..s_end])
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            // Decode DuckDuckGo redirect URL
            let actual_url = if url.starts_with("//duckduckgo.com/l/") {
                // Extract actual URL from redirect
                if let Some(uddg_start) = url.find("uddg=") {
                    let encoded = &url[uddg_start + 5..];
                    if let Some(end) = encoded.find('&') {
                        urlencoding::decode(&encoded[..end])
                            .map(|s| s.to_string())
                            .unwrap_or_else(|_| url.to_string())
                    } else {
                        urlencoding::decode(encoded)
                            .map(|s| s.to_string())
                            .unwrap_or_else(|_| url.to_string())
                    }
                } else {
                    url.to_string()
                }
            } else {
                url.to_string()
            };

            results.push(SearchResult {
                title,
                url: actual_url,
                snippet,
                source: "duckduckgo".to_string(),
            });
        }

        Ok(SearchResponse {
            query: query.to_string(),
            results,
            backend: "duckduckgo".to_string(),
        })
    }

    /// Search using Playwright browser automation (headless Chromium via bridge sidecar)
    async fn search_playwright(
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
            "https://search.brave.com/search?q={}&count={}",
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
            let snippet = Self::extract_snippet_near(&body_text, &title);

            results.push(SearchResult {
                title,
                url: el.href.clone(),
                snippet,
                source: "playwright".to_string(),
            });

            if results.len() >= num_results {
                break;
            }
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
            &title[..title
                .char_indices()
                .nth(20)
                .map(|(i, _)| i)
                .unwrap_or(title.len())]
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
                let max_len = snippet_text.len().min(200);
                let chunk = &snippet_text[..max_len];
                let end = chunk.rfind(". ").map(|p| p + 1).unwrap_or(max_len);
                return chunk[..end].trim().to_string();
            }
        }
        String::new()
    }

    /// Search using Lightpanda + DuckDuckGo HTML
    async fn search_lightpanda(&self, query: &str, num_results: usize) -> Result<SearchResponse> {
        let search_url = format!(
            "https://html.duckduckgo.com/html/?q={}",
            urlencoding::encode(query)
        );

        let html = crate::integrations::lightpanda::fetch_html(&search_url).await?;

        let mut results = Vec::new();
        for chunk in html.split("class=\"result__a\"") {
            if results.len() >= num_results {
                break;
            }
            let href = chunk
                .split("href=\"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .unwrap_or("")
                .to_string();
            if href.is_empty() || href.starts_with('#') || href.contains("duckduckgo.com") {
                continue;
            }
            let url = if href.contains("uddg=") {
                urlencoding::decode(
                    href.split("uddg=")
                        .nth(1)
                        .unwrap_or(&href)
                        .split('&')
                        .next()
                        .unwrap_or(&href),
                )
                .unwrap_or_default()
                .to_string()
            } else {
                href
            };
            if url.is_empty() || !url.starts_with("http") {
                continue;
            }
            let title = chunk
                .split('>')
                .nth(1)
                .and_then(|s| s.split("</").next())
                .unwrap_or("")
                .trim()
                .to_string();
            let snippet = chunk
                .split("result__snippet")
                .nth(1)
                .and_then(|s| s.split('>').nth(1))
                .and_then(|s| s.split("</").next())
                .unwrap_or("")
                .trim()
                .to_string();

            results.push(SearchResult {
                title: strip_html_tags(&title),
                url,
                snippet: strip_html_tags(&snippet),
                source: "lightpanda".to_string(),
            });
        }

        Ok(SearchResponse {
            query: query.to_string(),
            results,
            backend: "lightpanda".to_string(),
        })
    }
}

fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        if ch == '<' {
            in_tag = true;
            continue;
        }
        if ch == '>' {
            in_tag = false;
            continue;
        }
        if !in_tag {
            out.push(ch);
        }
    }
    out
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

/// Search action arguments
#[derive(Debug, Deserialize)]
pub struct SearchArgs {
    pub query: String,
    #[serde(default = "default_num_results")]
    pub num_results: usize,
    #[serde(default)]
    pub backend: Option<String>,
}

fn default_num_results() -> usize {
    5
}

const DEFAULT_SEARCH_PRIMARY: &str = "lightpanda";
const DEFAULT_SEARCH_FALLBACK1: &str = "duckduckgo";
const DEFAULT_SEARCH_FALLBACK2: &str = "none";

/// Execute a web search
pub async fn execute_search(args: &SearchArgs, config: &SearchConfig) -> Result<String> {
    // When an explicit backend is requested, use it directly (no fallback)
    if let Some(explicit) = args.backend.as_deref() {
        let backend = match explicit {
            "searxng" => config
                .searxng
                .clone()
                .ok_or_else(|| anyhow!("SearXNG not configured"))?,
            "serper" => config
                .serper
                .clone()
                .ok_or_else(|| anyhow!("Serper not configured"))?,
            "brave" | "brave_api" => config
                .brave
                .clone()
                .ok_or_else(|| anyhow!("Brave not configured"))?,
            "playwright" => config
                .playwright
                .clone()
                .ok_or_else(|| anyhow!("Playwright not configured"))?,
            "duckduckgo" => SearchBackend::DuckDuckGo,
            "lightpanda" => SearchBackend::Lightpanda,
            other => return Err(anyhow!("Unknown search backend: {}", other)),
        };
        let client = SearchClient::new(backend);
        let response = client.search(&args.query, args.num_results).await?;
        return Ok(format_search_results(&response));
    }

    // Build fallback chain from config (primary → fallback1 → fallback2)
    let chain = config.ordered_backend_names();

    let mut last_err = None;
    for name in &chain {
        if let Some(backend) = config.resolve_backend(name) {
                let client = SearchClient::new(backend);
                match client.search(&args.query, args.num_results).await {
                    Ok(response) if !response.results.is_empty() => {
                        return Ok(format_search_results(&response));
                    }
                    Ok(_) => {
                        tracing::warn!("Search backend '{}' returned 0 results, trying next", name);
                        last_err = Some(anyhow!("Backend '{}' returned 0 results", name));
                    }
                    Err(e) => {
                        tracing::warn!("Search backend '{}' failed: {}, trying next", name, e);
                        last_err = Some(e);
                    }
                }
        } else {
            tracing::debug!("Search backend '{}' not configured, trying next", name);
        }

    // No chain configured — legacy default: prefer Playwright, fall back to DuckDuckGo
    }

    Err(last_err.unwrap_or_else(|| anyhow!("All search backends failed")))
}

/// Format search results into a human-readable string
fn format_search_results(response: &SearchResponse) -> String {
    let mut output = format!("Search results for: {}\n\n", response.query);
    for (i, result) in response.results.iter().enumerate() {
        output.push_str(&format!(
            "{}. {}\n   {}\n   {}\n\n",
            i + 1,
            result.title,
            result.url,
            result.snippet
        ));
    }
    output
}

/// Search configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    pub searxng: Option<SearchBackend>,
    pub serper: Option<SearchBackend>,
    pub brave: Option<SearchBackend>,
    pub playwright: Option<SearchBackend>,
    /// Preferred primary backend name (e.g. "lightpanda", "serper", "duckduckgo")
    #[serde(default)]
    pub primary: Option<String>,
    /// First fallback backend name
    #[serde(default)]
    pub fallback1: Option<String>,
    /// Second fallback backend name
    #[serde(default)]
    pub fallback2: Option<String>,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            searxng: None,
            serper: None,
            brave: None,
            playwright: None,
            primary: Some(DEFAULT_SEARCH_PRIMARY.to_string()),
            fallback1: Some(DEFAULT_SEARCH_FALLBACK1.to_string()),
            fallback2: Some(DEFAULT_SEARCH_FALLBACK2.to_string()),
        }
    }
}

impl SearchConfig {
    /// Resolve a backend name to a configured SearchBackend instance
    pub fn resolve_backend(&self, name: &str) -> Option<SearchBackend> {
        match name {
            "playwright" => self.playwright.clone(),
            "serper" => self.serper.clone(),
            "searxng" => self.searxng.clone(),
            "brave_api" | "brave" => self.brave.clone(),
            "duckduckgo" => Some(SearchBackend::DuckDuckGo),
            "lightpanda" => Some(SearchBackend::Lightpanda),
            _ => None,
        }
    }

    pub fn ordered_backend_names(&self) -> Vec<&str> {
        let chain: Vec<&str> = [
            self.primary.as_deref(),
            self.fallback1.as_deref(),
            self.fallback2.as_deref(),
        ]
        .iter()
        .filter_map(|value| *value)
        .filter(|value| !value.trim().is_empty() && *value != "none")
        .collect();

        if chain.is_empty() {
            vec![DEFAULT_SEARCH_PRIMARY, DEFAULT_SEARCH_FALLBACK1]
        } else {
            chain
        }
    }

    pub fn ensure_default_chain(&mut self) {
        let has_explicit_chain = [
            self.primary.as_deref(),
            self.fallback1.as_deref(),
            self.fallback2.as_deref(),
        ]
        .iter()
        .filter_map(|value| *value)
        .any(|value| !value.trim().is_empty() && value != "none");

        if !has_explicit_chain {
            self.primary = Some(DEFAULT_SEARCH_PRIMARY.to_string());
            self.fallback1 = Some(DEFAULT_SEARCH_FALLBACK1.to_string());
            self.fallback2 = Some(DEFAULT_SEARCH_FALLBACK2.to_string());
        }
    }
}
