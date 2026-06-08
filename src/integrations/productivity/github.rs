//! GitHub Integration
//!
//! Provides access to GitHub repositories, issues, pull requests, and search.
//! Authenticates via personal access token (GITHUB_TOKEN env var or secure config).

use super::{Capability, Integration, IntegrationStatus};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

/// GitHub API connector
pub struct GitHubConnector {
    http: reqwest::Client,
    config_dir: PathBuf,
}

impl GitHubConnector {
    const API_BASE: &'static str = "https://api.github.com";
    const MAX_RETRY_ATTEMPTS: usize = 3;

    pub fn new_with_config_dir(config_dir: PathBuf) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .connect_timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap_or_else(|_| crate::core::runtime::net::build_outgoing_http_client(15)),
            config_dir,
        }
    }

    pub fn new() -> Self {
        let config_dir = crate::branding::project_dirs()
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        Self::new_with_config_dir(config_dir)
    }

    /// Load token from environment variable or secure config storage
    pub fn load_token_from(config_dir: &Path) -> Option<String> {
        if let Ok(token) = std::env::var("GITHUB_TOKEN") {
            if !token.is_empty() {
                return Some(token);
            }
        }
        match crate::core::runtime::config::SecureConfigManager::new(config_dir) {
            Ok(manager) => manager.get_custom_secret("github_token").ok().flatten(),
            Err(_) => None,
        }
    }

    /// Build an authenticated request with standard GitHub headers
    fn authed_request(
        &self,
        method: reqwest::Method,
        url: &str,
    ) -> Result<reqwest::RequestBuilder> {
        let token = Self::load_token_from(&self.config_dir).ok_or_else(|| {
            anyhow!("GitHub token not configured. Set GITHUB_TOKEN or store via secure config.")
        })?;

        Ok(self
            .http
            .request(method, url)
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", crate::branding::versioned_user_agent())
            .header("Accept", "application/vnd.github+json"))
    }

    fn build_url(&self, path_segments: &[&str], query: &[(&str, String)]) -> Result<reqwest::Url> {
        let mut url = reqwest::Url::parse(Self::API_BASE)?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| anyhow!("Failed to build GitHub API URL"))?;
            for segment in path_segments {
                segments.push(segment);
            }
        }
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }
        Ok(url)
    }

    async fn send_with_retry(
        &self,
        request: reqwest::RequestBuilder,
        allow_retry: bool,
    ) -> Result<reqwest::Response> {
        let mut last_error = None;
        for attempt in 0..Self::MAX_RETRY_ATTEMPTS {
            let response = request
                .try_clone()
                .ok_or_else(|| anyhow!("Failed to clone GitHub request"))?
                .send()
                .await;
            match response {
                Ok(resp) => {
                    let status = resp.status();
                    if allow_retry
                        && attempt + 1 < Self::MAX_RETRY_ATTEMPTS
                        && (status.as_u16() == 429 || status.is_server_error())
                    {
                        let retry_after = resp
                            .headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.parse::<u64>().ok())
                            .unwrap_or(2)
                            .min(30);
                        tokio::time::sleep(std::time::Duration::from_secs(retry_after)).await;
                        continue;
                    }
                    return Ok(resp);
                }
                Err(err) => {
                    if allow_retry && attempt + 1 < Self::MAX_RETRY_ATTEMPTS {
                        last_error = Some(err);
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        continue;
                    }
                    return Err(err.into());
                }
            }
        }
        Err(last_error
            .map(anyhow::Error::from)
            .unwrap_or_else(|| anyhow!("GitHub request failed after retries")))
    }

    // === Action implementations ===

    /// List the authenticated user's repositories sorted by last update
    async fn list_repos(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let per_page = params
            .get("per_page")
            .and_then(|v| v.as_u64())
            .unwrap_or(30);

        let sort = params
            .get("sort")
            .and_then(|v| v.as_str())
            .unwrap_or("updated");

        let url = self.build_url(
            &["user", "repos"],
            &[
                ("per_page", per_page.to_string()),
                ("sort", sort.to_string()),
            ],
        )?;

        let response = self
            .send_with_retry(
                self.authed_request(reqwest::Method::GET, url.as_str())?,
                true,
            )
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            tracing::warn!("GitHub list_repos failed: {}", error);
            return Err(anyhow!("Failed to list repositories: {}", error));
        }

        let repos: Vec<serde_json::Value> = response.json().await?;

        let result: Vec<serde_json::Value> = repos
            .iter()
            .map(|repo| {
                serde_json::json!({
                    "name": repo.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                    "full_name": repo.get("full_name").and_then(|v| v.as_str()).unwrap_or(""),
                    "description": repo.get("description").and_then(|v| v.as_str()),
                    "html_url": repo.get("html_url").and_then(|v| v.as_str()).unwrap_or(""),
                    "language": repo.get("language").and_then(|v| v.as_str()),
                    "updated_at": repo.get("updated_at").and_then(|v| v.as_str()).unwrap_or(""),
                })
            })
            .collect();

        Ok(serde_json::json!({ "repositories": result }))
    }

    /// Create an issue in a repository
    async fn create_issue(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let owner = params
            .get("owner")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'owner' parameter"))?;
        let repo = params
            .get("repo")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'repo' parameter"))?;
        let title = params
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'title' parameter"))?;

        let body_text = params.get("body").and_then(|v| v.as_str()).unwrap_or("");

        let labels: Vec<String> = params
            .get("labels")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let url = self.build_url(&["repos", owner, repo, "issues"], &[])?;

        let request_body = serde_json::json!({
            "title": title,
            "body": body_text,
            "labels": labels,
        });

        let response = self
            .authed_request(reqwest::Method::POST, url.as_str())?
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            tracing::warn!(
                "GitHub create_issue failed for {}/{}: {}",
                owner,
                repo,
                error
            );
            return Err(anyhow!("Failed to create issue: {}", error));
        }

        let issue: serde_json::Value = response.json().await?;

        Ok(serde_json::json!({
            "number": issue.get("number").and_then(|v| v.as_u64()).unwrap_or(0),
            "html_url": issue.get("html_url").and_then(|v| v.as_str()).unwrap_or(""),
            "title": issue.get("title").and_then(|v| v.as_str()).unwrap_or(""),
        }))
    }

    /// List issues for a repository
    async fn list_issues(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let owner = params
            .get("owner")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'owner' parameter"))?;
        let repo = params
            .get("repo")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'repo' parameter"))?;
        let state = params
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("open");
        let per_page = params
            .get("per_page")
            .and_then(|v| v.as_u64())
            .unwrap_or(20);

        let url = self.build_url(
            &["repos", owner, repo, "issues"],
            &[
                ("state", state.to_string()),
                ("per_page", per_page.to_string()),
            ],
        )?;

        let response = self
            .send_with_retry(
                self.authed_request(reqwest::Method::GET, url.as_str())?,
                true,
            )
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            tracing::warn!(
                "GitHub list_issues failed for {}/{}: {}",
                owner,
                repo,
                error
            );
            return Err(anyhow!("Failed to list issues: {}", error));
        }

        let issues: Vec<serde_json::Value> = response.json().await?;

        let result: Vec<serde_json::Value> = issues.iter().map(|issue| {
            serde_json::json!({
                "number": issue.get("number").and_then(|v| v.as_u64()).unwrap_or(0),
                "title": issue.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                "state": issue.get("state").and_then(|v| v.as_str()).unwrap_or(""),
                "html_url": issue.get("html_url").and_then(|v| v.as_str()).unwrap_or(""),
                "user": issue.get("user").and_then(|u| u.get("login")).and_then(|v| v.as_str()).unwrap_or(""),
                "created_at": issue.get("created_at").and_then(|v| v.as_str()).unwrap_or(""),
                "updated_at": issue.get("updated_at").and_then(|v| v.as_str()).unwrap_or(""),
                "labels": issue.get("labels")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter()
                        .filter_map(|l| l.get("name").and_then(|n| n.as_str()))
                        .collect::<Vec<_>>())
                    .unwrap_or_default(),
            })
        }).collect();

        Ok(serde_json::json!({ "issues": result }))
    }

    /// List pull requests for a repository
    async fn list_prs(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let owner = params
            .get("owner")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'owner' parameter"))?;
        let repo = params
            .get("repo")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'repo' parameter"))?;
        let state = params
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("open");
        let per_page = params
            .get("per_page")
            .and_then(|v| v.as_u64())
            .unwrap_or(20);

        let url = self.build_url(
            &["repos", owner, repo, "pulls"],
            &[
                ("state", state.to_string()),
                ("per_page", per_page.to_string()),
            ],
        )?;

        let response = self
            .send_with_retry(
                self.authed_request(reqwest::Method::GET, url.as_str())?,
                true,
            )
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            tracing::warn!("GitHub list_prs failed for {}/{}: {}", owner, repo, error);
            return Err(anyhow!("Failed to list pull requests: {}", error));
        }

        let prs: Vec<serde_json::Value> = response.json().await?;

        let result: Vec<serde_json::Value> = prs.iter().map(|pr| {
            serde_json::json!({
                "number": pr.get("number").and_then(|v| v.as_u64()).unwrap_or(0),
                "title": pr.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                "state": pr.get("state").and_then(|v| v.as_str()).unwrap_or(""),
                "html_url": pr.get("html_url").and_then(|v| v.as_str()).unwrap_or(""),
                "user": pr.get("user").and_then(|u| u.get("login")).and_then(|v| v.as_str()).unwrap_or(""),
                "head": pr.get("head").and_then(|h| h.get("ref")).and_then(|v| v.as_str()).unwrap_or(""),
                "base": pr.get("base").and_then(|b| b.get("ref")).and_then(|v| v.as_str()).unwrap_or(""),
                "created_at": pr.get("created_at").and_then(|v| v.as_str()).unwrap_or(""),
                "updated_at": pr.get("updated_at").and_then(|v| v.as_str()).unwrap_or(""),
            })
        }).collect();

        Ok(serde_json::json!({ "pull_requests": result }))
    }

    /// Create a pull request
    async fn create_pr(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let owner = params
            .get("owner")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'owner' parameter"))?;
        let repo = params
            .get("repo")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'repo' parameter"))?;
        let title = params
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'title' parameter"))?;
        let head = params
            .get("head")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'head' parameter (source branch)"))?;
        let base = params
            .get("base")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'base' parameter (target branch)"))?;

        let body_text = params.get("body").and_then(|v| v.as_str()).unwrap_or("");

        let url = self.build_url(&["repos", owner, repo, "pulls"], &[])?;

        let request_body = serde_json::json!({
            "title": title,
            "body": body_text,
            "head": head,
            "base": base,
        });

        let response = self
            .authed_request(reqwest::Method::POST, url.as_str())?
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            tracing::warn!("GitHub create_pr failed for {}/{}: {}", owner, repo, error);
            return Err(anyhow!("Failed to create pull request: {}", error));
        }

        let pr: serde_json::Value = response.json().await?;

        Ok(serde_json::json!({
            "number": pr.get("number").and_then(|v| v.as_u64()).unwrap_or(0),
            "html_url": pr.get("html_url").and_then(|v| v.as_str()).unwrap_or(""),
            "title": pr.get("title").and_then(|v| v.as_str()).unwrap_or(""),
        }))
    }

    /// Search GitHub repositories, issues, or code
    async fn search(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'query' parameter"))?;
        let search_type = params
            .get("search_type")
            .and_then(|v| v.as_str())
            .unwrap_or("repos");

        let endpoint = match search_type {
            "repos" | "repositories" => "repositories",
            "issues" => "issues",
            "code" => "code",
            other => {
                return Err(anyhow!(
                    "Unsupported search_type '{}'. Use: repos, issues, code",
                    other
                ));
            }
        };

        let url = self.build_url(&["search", endpoint], &[("q", query.to_string())])?;

        let response = self
            .send_with_retry(
                self.authed_request(reqwest::Method::GET, url.as_str())?,
                true,
            )
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            tracing::warn!(
                "GitHub search failed for type={}, query={}: {}",
                search_type,
                query,
                error
            );
            return Err(anyhow!("Search failed: {}", error));
        }

        let body: serde_json::Value = response.json().await?;

        let total_count = body
            .get("total_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let items = body.get("items").and_then(|v| v.as_array());

        let results: Vec<serde_json::Value> = match items {
            Some(arr) => arr.iter().map(|item| {
                match search_type {
                    "repos" | "repositories" => serde_json::json!({
                        "full_name": item.get("full_name").and_then(|v| v.as_str()).unwrap_or(""),
                        "description": item.get("description").and_then(|v| v.as_str()),
                        "html_url": item.get("html_url").and_then(|v| v.as_str()).unwrap_or(""),
                        "language": item.get("language").and_then(|v| v.as_str()),
                        "stargazers_count": item.get("stargazers_count").and_then(|v| v.as_u64()).unwrap_or(0),
                        "updated_at": item.get("updated_at").and_then(|v| v.as_str()).unwrap_or(""),
                    }),
                    "issues" => serde_json::json!({
                        "title": item.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                        "html_url": item.get("html_url").and_then(|v| v.as_str()).unwrap_or(""),
                        "state": item.get("state").and_then(|v| v.as_str()).unwrap_or(""),
                        "repository_url": item.get("repository_url").and_then(|v| v.as_str()).unwrap_or(""),
                        "created_at": item.get("created_at").and_then(|v| v.as_str()).unwrap_or(""),
                    }),
                    "code" => serde_json::json!({
                        "name": item.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                        "path": item.get("path").and_then(|v| v.as_str()).unwrap_or(""),
                        "html_url": item.get("html_url").and_then(|v| v.as_str()).unwrap_or(""),
                        "repository": item.get("repository")
                            .and_then(|r| r.get("full_name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or(""),
                    }),
                    _ => item.clone(),
                }
            }).collect(),
            None => vec![],
        };

        Ok(serde_json::json!({
            "total_count": total_count,
            "search_type": search_type,
            "results": results,
        }))
    }
}

#[async_trait]
impl Integration for GitHubConnector {
    fn id(&self) -> &str {
        "github"
    }

    fn name(&self) -> &str {
        "GitHub"
    }

    fn description(&self) -> &str {
        "GitHub API connector for repositories, issues, pull requests, and search"
    }

    fn icon(&self) -> &str {
        "🐙"
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Read, Capability::Write, Capability::Search]
    }

    async fn status(&self) -> IntegrationStatus {
        // Verify the token works with a lightweight API call
        let url = match self.build_url(&["user"], &[]) {
            Ok(url) => url,
            Err(error) => {
                return IntegrationStatus::Error(format!("Invalid GitHub URL: {}", error));
            }
        };
        match self.authed_request(reqwest::Method::GET, url.as_str()) {
            Ok(req) => match self.send_with_retry(req, true).await {
                Ok(resp) if resp.status().is_success() => IntegrationStatus::Connected,
                Ok(resp) => {
                    let status = resp.status();
                    tracing::warn!("GitHub token validation returned status {}", status);
                    IntegrationStatus::Error(format!("API returned {}", status))
                }
                Err(e) => {
                    tracing::warn!("GitHub connectivity check failed: {}", e);
                    IntegrationStatus::Error(format!("Connection failed: {}", e))
                }
            },
            Err(_) => IntegrationStatus::NotConfigured,
        }
    }

    async fn execute(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value> {
        match action {
            "list_repos" => self.list_repos(params).await,
            "create_issue" => self.create_issue(params).await,
            "list_issues" => self.list_issues(params).await,
            "list_prs" => self.list_prs(params).await,
            "create_pr" => self.create_pr(params).await,
            "search" => self.search(params).await,
            _ => Err(anyhow!("Unknown action: {}", action)),
        }
    }
}

impl Default for GitHubConnector {
    fn default() -> Self {
        Self::new()
    }
}
