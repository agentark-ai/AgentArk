//! Notion Integration
//!
//! Provides access to Notion workspaces: search pages, create and update pages,
//! append content blocks. Authenticates via integration token (NOTION_TOKEN env var
//! or secure config).

use super::{Capability, Integration, IntegrationStatus};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

/// Notion API connector
pub struct NotionConnector {
    http: reqwest::Client,
    config_dir: PathBuf,
}

impl NotionConnector {
    const API_BASE: &'static str = "https://api.notion.com/v1";
    const NOTION_VERSION: &'static str = "2022-06-28";

    pub fn new_with_config_dir(config_dir: PathBuf) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .connect_timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
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
    fn load_token_from(config_dir: &Path) -> Option<String> {
        if let Ok(token) = std::env::var("NOTION_TOKEN") {
            if !token.is_empty() {
                return Some(token);
            }
        }
        match crate::core::config::SecureConfigManager::new(config_dir) {
            Ok(manager) => manager.get_custom_secret("notion_token").ok().flatten(),
            Err(_) => None,
        }
    }

    /// Build an authenticated request with standard Notion headers
    fn authed_request(
        &self,
        method: reqwest::Method,
        url: &str,
    ) -> Result<reqwest::RequestBuilder> {
        let token = Self::load_token_from(&self.config_dir).ok_or_else(|| {
            anyhow!("Notion token not configured. Set NOTION_TOKEN or store via secure config.")
        })?;

        Ok(self
            .http
            .request(method, url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Notion-Version", Self::NOTION_VERSION)
            .header("Content-Type", "application/json"))
    }

    // === Action implementations ===

    /// Search for pages in the Notion workspace
    async fn search(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");

        let url = format!("{}/search", Self::API_BASE);

        let request_body = serde_json::json!({
            "query": query,
            "filter": {
                "property": "object",
                "value": "page"
            },
            "sort": {
                "direction": "descending",
                "timestamp": "last_edited_time"
            }
        });

        let response = self
            .authed_request(reqwest::Method::POST, &url)?
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            tracing::warn!("Notion search failed: {}", error);
            return Err(anyhow!("Search failed: {}", error));
        }

        let body: serde_json::Value = response.json().await?;

        let results = body
            .get("results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let pages: Vec<serde_json::Value> = results.iter().map(|page| {
            let title = Self::extract_page_title(page);
            serde_json::json!({
                "id": page.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                "title": title,
                "url": page.get("url").and_then(|v| v.as_str()).unwrap_or(""),
                "last_edited_time": page.get("last_edited_time").and_then(|v| v.as_str()).unwrap_or(""),
            })
        }).collect();

        Ok(serde_json::json!({ "pages": pages }))
    }

    /// Create a new page in a Notion database or as a child of another page
    async fn create_page(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let parent_id = params
            .get("parent_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'parent_id' parameter (database or page ID)"))?;
        let title = params
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'title' parameter"))?;
        let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");

        let url = format!("{}/pages", Self::API_BASE);

        // Determine parent type: if it looks like a database ID (32 hex chars with dashes),
        // use database_id; otherwise use page_id
        let parent = if params.get("parent_type").and_then(|v| v.as_str()) == Some("page") {
            serde_json::json!({ "page_id": parent_id })
        } else {
            serde_json::json!({ "database_id": parent_id })
        };

        let children = markdown_to_blocks(content);

        let request_body = serde_json::json!({
            "parent": parent,
            "properties": {
                "title": {
                    "title": [{
                        "text": {
                            "content": title
                        }
                    }]
                }
            },
            "children": children,
        });

        let response = self
            .authed_request(reqwest::Method::POST, &url)?
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            tracing::warn!("Notion create_page failed: {}", error);
            return Err(anyhow!("Failed to create page: {}", error));
        }

        let page: serde_json::Value = response.json().await?;

        Ok(serde_json::json!({
            "id": page.get("id").and_then(|v| v.as_str()).unwrap_or(""),
            "url": page.get("url").and_then(|v| v.as_str()).unwrap_or(""),
            "title": title,
            "created_time": page.get("created_time").and_then(|v| v.as_str()).unwrap_or(""),
        }))
    }

    /// Update page properties
    async fn update_page(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let page_id = params
            .get("page_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'page_id' parameter"))?;

        let properties = params
            .get("properties")
            .ok_or_else(|| anyhow!("Missing 'properties' parameter (JSON object)"))?;

        let url = format!("{}/pages/{}", Self::API_BASE, page_id);

        let request_body = serde_json::json!({
            "properties": properties,
        });

        let response = self
            .authed_request(reqwest::Method::PATCH, &url)?
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            tracing::warn!("Notion update_page failed for {}: {}", page_id, error);
            return Err(anyhow!("Failed to update page: {}", error));
        }

        let page: serde_json::Value = response.json().await?;

        Ok(serde_json::json!({
            "id": page.get("id").and_then(|v| v.as_str()).unwrap_or(""),
            "url": page.get("url").and_then(|v| v.as_str()).unwrap_or(""),
            "last_edited_time": page.get("last_edited_time").and_then(|v| v.as_str()).unwrap_or(""),
        }))
    }

    /// Get a page by ID
    async fn get_page(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let page_id = params
            .get("page_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'page_id' parameter"))?;

        let url = format!("{}/pages/{}", Self::API_BASE, page_id);

        let response = self
            .authed_request(reqwest::Method::GET, &url)?
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            tracing::warn!("Notion get_page failed for {}: {}", page_id, error);
            return Err(anyhow!("Failed to get page: {}", error));
        }

        let page: serde_json::Value = response.json().await?;

        let title = Self::extract_page_title(&page);

        Ok(serde_json::json!({
            "id": page.get("id").and_then(|v| v.as_str()).unwrap_or(""),
            "title": title,
            "url": page.get("url").and_then(|v| v.as_str()).unwrap_or(""),
            "created_time": page.get("created_time").and_then(|v| v.as_str()).unwrap_or(""),
            "last_edited_time": page.get("last_edited_time").and_then(|v| v.as_str()).unwrap_or(""),
            "properties": page.get("properties").cloned().unwrap_or(serde_json::json!({})),
            "parent": page.get("parent").cloned().unwrap_or(serde_json::json!({})),
        }))
    }

    /// Append content blocks to a page or block
    async fn append_blocks(&self, params: &serde_json::Value) -> Result<serde_json::Value> {
        let block_id = params
            .get("block_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'block_id' parameter (page or block ID)"))?;
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'content' parameter (text to append)"))?;

        let blocks = markdown_to_blocks(content);

        let url = format!("{}/blocks/{}/children", Self::API_BASE, block_id);

        let request_body = serde_json::json!({
            "children": blocks,
        });

        let response = self
            .authed_request(reqwest::Method::PATCH, &url)?
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await.unwrap_or_default();
            tracing::warn!("Notion append_blocks failed for {}: {}", block_id, error);
            return Err(anyhow!("Failed to append blocks: {}", error));
        }

        let body: serde_json::Value = response.json().await?;

        let results = body
            .get("results")
            .and_then(|v| v.as_array())
            .map(|arr| arr.len())
            .unwrap_or(0);

        Ok(serde_json::json!({
            "block_id": block_id,
            "blocks_appended": results,
        }))
    }

    /// Extract the title from a Notion page object
    fn extract_page_title(page: &serde_json::Value) -> String {
        // Try the "title" property (common for database pages)
        if let Some(properties) = page.get("properties") {
            // Look through all properties for one with type "title"
            if let Some(obj) = properties.as_object() {
                for (_key, prop) in obj {
                    if prop.get("type").and_then(|v| v.as_str()) == Some("title") {
                        if let Some(title_arr) = prop.get("title").and_then(|v| v.as_array()) {
                            let title: String = title_arr
                                .iter()
                                .filter_map(|t| t.get("plain_text").and_then(|v| v.as_str()))
                                .collect();
                            if !title.is_empty() {
                                return title;
                            }
                        }
                    }
                }
            }
        }

        // Fallback: look for a "Name" property
        if let Some(name_prop) = page.get("properties").and_then(|p| p.get("Name")) {
            if let Some(title_arr) = name_prop.get("title").and_then(|v| v.as_array()) {
                let title: String = title_arr
                    .iter()
                    .filter_map(|t| t.get("plain_text").and_then(|v| v.as_str()))
                    .collect();
                if !title.is_empty() {
                    return title;
                }
            }
        }

        "(Untitled)".to_string()
    }
}

/// Convert plain text/markdown paragraphs into Notion block objects.
///
/// Splits the input on blank lines to produce separate paragraph blocks.
/// Lines starting with `# `, `## `, or `### ` are converted to heading blocks.
/// Lines starting with `- ` or `* ` are converted to bulleted list item blocks.
/// Lines starting with a digit and `. ` (e.g., `1. `) are converted to numbered list item blocks.
/// Lines starting with `> ` are converted to quote blocks.
/// Lines surrounded by triple backticks produce a code block.
/// All other non-empty text becomes a paragraph block.
fn markdown_to_blocks(text: &str) -> Vec<serde_json::Value> {
    let mut blocks: Vec<serde_json::Value> = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // Skip empty lines between blocks
        if line.trim().is_empty() {
            i += 1;
            continue;
        }

        // Fenced code block
        if line.trim_start().starts_with("```") {
            let language = line.trim_start().trim_start_matches('`').trim().to_string();
            let lang = if language.is_empty() {
                "plain text".to_string()
            } else {
                language
            };
            let mut code_lines: Vec<&str> = Vec::new();
            i += 1;
            while i < lines.len() {
                if lines[i].trim_start().starts_with("```") {
                    i += 1;
                    break;
                }
                code_lines.push(lines[i]);
                i += 1;
            }
            let code_content = code_lines.join("\n");
            blocks.push(serde_json::json!({
                "object": "block",
                "type": "code",
                "code": {
                    "rich_text": [{
                        "type": "text",
                        "text": { "content": code_content }
                    }],
                    "language": lang
                }
            }));
            continue;
        }

        // Heading 1
        if let Some(content) = line.strip_prefix("# ") {
            blocks.push(serde_json::json!({
                "object": "block",
                "type": "heading_1",
                "heading_1": {
                    "rich_text": [{
                        "type": "text",
                        "text": { "content": content }
                    }]
                }
            }));
            i += 1;
            continue;
        }

        // Heading 2
        if let Some(content) = line.strip_prefix("## ") {
            blocks.push(serde_json::json!({
                "object": "block",
                "type": "heading_2",
                "heading_2": {
                    "rich_text": [{
                        "type": "text",
                        "text": { "content": content }
                    }]
                }
            }));
            i += 1;
            continue;
        }

        // Heading 3
        if let Some(content) = line.strip_prefix("### ") {
            blocks.push(serde_json::json!({
                "object": "block",
                "type": "heading_3",
                "heading_3": {
                    "rich_text": [{
                        "type": "text",
                        "text": { "content": content }
                    }]
                }
            }));
            i += 1;
            continue;
        }

        // Bulleted list item
        if line.starts_with("- ") || line.starts_with("* ") {
            let content = &line[2..];
            blocks.push(serde_json::json!({
                "object": "block",
                "type": "bulleted_list_item",
                "bulleted_list_item": {
                    "rich_text": [{
                        "type": "text",
                        "text": { "content": content }
                    }]
                }
            }));
            i += 1;
            continue;
        }

        // Numbered list item (e.g., "1. item")
        if line.len() > 2 {
            let trimmed = line.trim_start();
            if let Some(dot_pos) = trimmed.find(". ") {
                let prefix = &trimmed[..dot_pos];
                if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()) {
                    let content = &trimmed[dot_pos + 2..];
                    blocks.push(serde_json::json!({
                        "object": "block",
                        "type": "numbered_list_item",
                        "numbered_list_item": {
                            "rich_text": [{
                                "type": "text",
                                "text": { "content": content }
                            }]
                        }
                    }));
                    i += 1;
                    continue;
                }
            }
        }

        // Block quote
        if let Some(content) = line.strip_prefix("> ") {
            blocks.push(serde_json::json!({
                "object": "block",
                "type": "quote",
                "quote": {
                    "rich_text": [{
                        "type": "text",
                        "text": { "content": content }
                    }]
                }
            }));
            i += 1;
            continue;
        }

        // Regular paragraph: collect consecutive non-empty, non-special lines
        let mut para_lines: Vec<&str> = vec![line];
        i += 1;
        while i < lines.len() {
            let next = lines[i];
            if next.trim().is_empty()
                || next.starts_with("# ")
                || next.starts_with("## ")
                || next.starts_with("### ")
                || next.starts_with("- ")
                || next.starts_with("* ")
                || next.starts_with("> ")
                || next.trim_start().starts_with("```")
            {
                break;
            }
            // Check for numbered list
            let trimmed = next.trim_start();
            if let Some(dot_pos) = trimmed.find(". ") {
                let prefix = &trimmed[..dot_pos];
                if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()) {
                    break;
                }
            }
            para_lines.push(next);
            i += 1;
        }

        let paragraph_text = para_lines.join("\n");

        // Notion rich_text content has a 2000 char limit per text object
        // Split long paragraphs into multiple rich_text segments
        let mut rich_text: Vec<serde_json::Value> = Vec::new();
        let mut remaining = paragraph_text.as_str();
        while !remaining.is_empty() {
            let chunk_len = remaining.len().min(2000);
            let chunk = &remaining[..chunk_len];
            rich_text.push(serde_json::json!({
                "type": "text",
                "text": { "content": chunk }
            }));
            remaining = &remaining[chunk_len..];
        }

        blocks.push(serde_json::json!({
            "object": "block",
            "type": "paragraph",
            "paragraph": {
                "rich_text": rich_text
            }
        }));
    }

    // If no blocks were produced from empty input, return a single empty paragraph
    if blocks.is_empty() && !text.is_empty() {
        blocks.push(serde_json::json!({
            "object": "block",
            "type": "paragraph",
            "paragraph": {
                "rich_text": [{
                    "type": "text",
                    "text": { "content": text }
                }]
            }
        }));
    }

    blocks
}

#[async_trait]
impl Integration for NotionConnector {
    fn id(&self) -> &str {
        "notion"
    }

    fn name(&self) -> &str {
        "Notion"
    }

    fn description(&self) -> &str {
        "Access Notion workspaces - search, create, and update pages and databases"
    }

    fn icon(&self) -> &str {
        "📝"
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Read, Capability::Write, Capability::Search]
    }

    async fn status(&self) -> IntegrationStatus {
        // Verify the token with a lightweight search call
        let url = format!("{}/users/me", Self::API_BASE);
        match self.authed_request(reqwest::Method::GET, &url) {
            Ok(req) => match req.send().await {
                Ok(resp) if resp.status().is_success() => IntegrationStatus::Connected,
                Ok(resp) => {
                    let status = resp.status();
                    tracing::warn!("Notion token validation returned status {}", status);
                    IntegrationStatus::Error(format!("API returned {}", status))
                }
                Err(e) => {
                    tracing::warn!("Notion connectivity check failed: {}", e);
                    IntegrationStatus::Error(format!("Connection failed: {}", e))
                }
            },
            Err(_) => IntegrationStatus::NotConfigured,
        }
    }

    async fn execute(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value> {
        match action {
            "search" => self.search(params).await,
            "create_page" => self.create_page(params).await,
            "update_page" => self.update_page(params).await,
            "get_page" => self.get_page(params).await,
            "append_blocks" => self.append_blocks(params).await,
            _ => Err(anyhow!("Unknown action: {}", action)),
        }
    }
}

impl Default for NotionConnector {
    fn default() -> Self {
        Self::new()
    }
}
