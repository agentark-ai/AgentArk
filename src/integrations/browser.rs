//! Browser automation integration via Playwright bridge
//! HTTP client that communicates with the local Playwright bridge
//! to control a headless browser for web automation tasks.

use super::{Capability, Integration, IntegrationStatus};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Browser integration — thin HTTP client over the Playwright bridge
pub struct BrowserIntegration {
    client: reqwest::Client,
    bridge_url: String,
}

#[derive(Debug, Deserialize)]
struct SessionResponse {
    session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserSidecarSessionState {
    pub session_id: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub claimed: bool,
    #[serde(default)]
    pub claimed_at: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub live_view_enabled: bool,
    #[serde(default)]
    pub live_view_port: Option<u16>,
    #[serde(default)]
    pub live_view_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NavigateResponse {
    #[serde(rename = "status")]
    _status: String,
    url: Option<String>,
    title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageContent {
    pub title: String,
    pub url: String,
    pub body_text: String,
    pub elements: Vec<PageElement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageElement {
    pub index: usize,
    pub tag: String,
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub href: String,
    pub x: i32,
    pub y: i32,
}

fn browser_target_host_allowed(host: &url::Host<&str>) -> bool {
    match host {
        url::Host::Domain(domain) => {
            let lower = domain.trim().to_ascii_lowercase();
            !(lower == "localhost"
                || lower.ends_with(".localhost")
                || lower.ends_with(".local")
                || lower == "metadata.google.internal")
        }
        url::Host::Ipv4(ip) => {
            !((*ip).is_loopback()
                || (*ip).is_private()
                || (*ip).is_link_local()
                || (*ip).is_multicast()
                || (*ip).is_unspecified()
                || *ip == std::net::Ipv4Addr::new(169, 254, 169, 254))
        }
        url::Host::Ipv6(ip) => {
            !((*ip).is_loopback()
                || (*ip).is_unique_local()
                || (*ip).is_unicast_link_local()
                || (*ip).is_multicast()
                || (*ip).is_unspecified())
        }
    }
}

fn browser_app_path_allowed(path: &str) -> bool {
    path == "/apps" || path == "/apps/" || path.starts_with("/apps/")
}

fn browser_host_is_local_or_wildcard(host: &url::Host<&str>) -> bool {
    match host {
        url::Host::Domain(domain) => {
            let lower = domain.trim().trim_end_matches('.').to_ascii_lowercase();
            lower == "localhost"
                || lower.ends_with(".localhost")
                || lower == "0.0.0.0"
                || lower == "::"
        }
        url::Host::Ipv4(ip) => ip.is_loopback() || ip.is_unspecified(),
        url::Host::Ipv6(ip) => ip.is_loopback() || ip.is_unspecified(),
    }
}

fn browser_hosts_match_for_internal_app(
    target: &url::Host<&str>,
    internal: &url::Host<&str>,
) -> bool {
    if target.to_string().eq_ignore_ascii_case(&internal.to_string()) {
        return true;
    }
    browser_host_is_local_or_wildcard(target) && browser_host_is_local_or_wildcard(internal)
}

fn browser_target_is_internal_app_url(parsed: &url::Url) -> bool {
    if !browser_app_path_allowed(parsed.path()) {
        return false;
    }
    let Ok(internal_base) = url::Url::parse(&crate::core::net::internal_api_base_url()) else {
        return false;
    };
    if parsed.scheme() != internal_base.scheme() {
        return false;
    }
    if parsed.port_or_known_default() != internal_base.port_or_known_default() {
        return false;
    }
    let Some(target_host) = parsed.host() else {
        return false;
    };
    let Some(internal_host) = internal_base.host() else {
        return false;
    };
    browser_hosts_match_for_internal_app(&target_host, &internal_host)
}

fn validate_browser_url(url: &str) -> Result<url::Url> {
    let parsed = url::Url::parse(url).map_err(|e| anyhow!("Invalid browser URL: {}", e))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(anyhow!(
                "Browser only allows http/https URLs, got '{}'",
                other
            ));
        }
    }
    let host = parsed
        .host()
        .ok_or_else(|| anyhow!("Browser URL must include a host"))?;
    if !browser_target_host_allowed(&host) && !browser_target_is_internal_app_url(&parsed) {
        return Err(anyhow!("Browser URL target is not allowed"));
    }
    Ok(parsed)
}

impl BrowserIntegration {
    pub fn new() -> Self {
        let bridge_url = std::env::var("PLAYWRIGHT_BRIDGE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3100".to_string());
        tracing::debug!("BrowserIntegration initialized: bridge_url={}", bridge_url);
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(35))
                .build()
                .unwrap_or_default(),
            bridge_url,
        }
    }

    /// Create a new browser session, returns session_id
    pub async fn create_session(&self) -> Result<String> {
        tracing::info!("Creating browser session via sidecar");
        let resp: SessionResponse = self
            .client
            .post(format!("{}/session", self.bridge_url))
            .json(&serde_json::json!({ "mode": "interactive" }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        tracing::info!(
            "Browser session created: sidecar_id={}",
            &resp.session_id[..resp.session_id.len().min(8)]
        );
        Ok(resp.session_id)
    }

    pub async fn get_session_state(&self, session_id: &str) -> Result<BrowserSidecarSessionState> {
        let state: BrowserSidecarSessionState = self
            .client
            .get(format!("{}/session/{}/state", self.bridge_url, session_id))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(state)
    }

    pub async fn claim_session(&self, session_id: &str) -> Result<BrowserSidecarSessionState> {
        let state: BrowserSidecarSessionState = self
            .client
            .post(format!("{}/session/{}/claim", self.bridge_url, session_id))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(state)
    }

    pub async fn release_session(&self, session_id: &str) -> Result<BrowserSidecarSessionState> {
        let state: BrowserSidecarSessionState = self
            .client
            .post(format!(
                "{}/session/{}/release",
                self.bridge_url, session_id
            ))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(state)
    }

    /// Navigate to a URL
    pub async fn navigate(&self, session_id: &str, url: &str) -> Result<(String, String)> {
        let validated_url = validate_browser_url(url)?;
        tracing::debug!(
            "Browser navigate: session={}, url_len={}",
            &session_id[..8],
            validated_url.as_str().len()
        );
        let resp: NavigateResponse = self
            .client
            .post(format!(
                "{}/session/{}/navigate",
                self.bridge_url, session_id
            ))
            .json(&serde_json::json!({ "url": validated_url.as_str() }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let final_url = resp.url.unwrap_or_default();
        let title = resp.title.unwrap_or_default();
        tracing::debug!("Browser navigated: title_len={}", title.len());
        Ok((final_url, title))
    }

    /// Take a screenshot, returns PNG bytes
    pub async fn screenshot(&self, session_id: &str) -> Result<Vec<u8>> {
        tracing::debug!("Browser screenshot: session={}", &session_id[..8]);
        let bytes = self
            .client
            .get(format!(
                "{}/session/{}/screenshot",
                self.bridge_url, session_id
            ))
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        tracing::debug!("Browser screenshot taken: bytes={}", bytes.len());
        Ok(bytes.to_vec())
    }

    /// Click an element by selector, text, or coordinates
    pub async fn click(
        &self,
        session_id: &str,
        selector: Option<&str>,
        text: Option<&str>,
        x: Option<i32>,
        y: Option<i32>,
    ) -> Result<()> {
        let mut body = serde_json::Map::new();
        if let Some(s) = selector {
            body.insert("selector".into(), serde_json::Value::String(s.to_string()));
        }
        if let Some(t) = text {
            body.insert("text".into(), serde_json::Value::String(t.to_string()));
        }
        if let Some(xv) = x {
            body.insert("x".into(), serde_json::json!(xv));
        }
        if let Some(yv) = y {
            body.insert("y".into(), serde_json::json!(yv));
        }
        self.client
            .post(format!("{}/session/{}/click", self.bridge_url, session_id))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// Type text into an element or the focused element
    pub async fn type_text(
        &self,
        session_id: &str,
        text: &str,
        selector: Option<&str>,
        clear: bool,
    ) -> Result<()> {
        let mut body = serde_json::json!({ "text": text, "clear": clear });
        if let Some(s) = selector {
            body["selector"] = serde_json::Value::String(s.to_string());
        }
        self.client
            .post(format!("{}/session/{}/type", self.bridge_url, session_id))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// Scroll the page
    pub async fn scroll(
        &self,
        session_id: &str,
        direction: &str,
        amount: Option<i32>,
    ) -> Result<()> {
        let mut body = serde_json::json!({ "direction": direction });
        if let Some(a) = amount {
            body["amount"] = serde_json::json!(a);
        }
        self.client
            .post(format!("{}/session/{}/scroll", self.bridge_url, session_id))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// Press a keyboard key
    pub async fn press_key(&self, session_id: &str, key: &str) -> Result<()> {
        self.client
            .post(format!("{}/session/{}/press", self.bridge_url, session_id))
            .json(&serde_json::json!({ "key": key }))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// Get page content and interactive elements
    pub async fn get_content(&self, session_id: &str) -> Result<PageContent> {
        let content: PageContent = self
            .client
            .get(format!(
                "{}/session/{}/content",
                self.bridge_url, session_id
            ))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(content)
    }

    /// Close a browser session
    pub async fn close_session(&self, session_id: &str) -> Result<()> {
        tracing::info!(
            "Closing browser session: session={}",
            &session_id[..session_id.len().min(8)]
        );
        self.client
            .delete(format!("{}/session/{}", self.bridge_url, session_id))
            .send()
            .await?
            .error_for_status()?;
        tracing::info!(
            "Browser session closed: session={}",
            &session_id[..session_id.len().min(8)]
        );
        Ok(())
    }

    /// Check if the sidecar is reachable
    pub async fn is_available(&self) -> bool {
        let result = self
            .client
            .get(format!("{}/health", self.bridge_url))
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        tracing::debug!(
            "Playwright sidecar health check: available={}, url={}",
            result,
            self.bridge_url
        );
        result
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::validate_browser_url;

    #[test]
    fn browser_rejects_local_targets() {
        assert!(validate_browser_url("http://127.0.0.1:8080").is_err());
        assert!(validate_browser_url("http://localhost:8080").is_err());
        assert!(validate_browser_url("http://169.254.169.254/latest/meta-data").is_err());
    }

    #[test]
    fn browser_accepts_internal_app_targets_only() {
        let base = crate::core::net::internal_api_base_url();
        let app_url = format!("{}/apps/demo/", base.trim_end_matches('/'));
        let control_url = format!("{}/api/health", base.trim_end_matches('/'));

        assert!(validate_browser_url(&app_url).is_ok());
        assert!(validate_browser_url(&control_url).is_err());
    }

    #[test]
    fn browser_accepts_public_https_target() {
        assert!(validate_browser_url("https://example.com/path").is_ok());
    }
}

#[async_trait]
impl Integration for BrowserIntegration {
    fn id(&self) -> &str {
        "browser"
    }
    fn name(&self) -> &str {
        "Browser Automation"
    }
    fn description(&self) -> &str {
        "Control a real web browser to automate tasks — navigate, click, type, screenshot"
    }
    fn icon(&self) -> &str {
        "🌐"
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Read, Capability::Write, Capability::Search]
    }

    async fn status(&self) -> IntegrationStatus {
        if self.is_available().await {
            IntegrationStatus::Connected
        } else {
            IntegrationStatus::NotConfigured
        }
    }

    async fn execute(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value> {
        match action {
            "create_session" => {
                let sid = self.create_session().await?;
                Ok(serde_json::json!({ "session_id": sid }))
            }
            "claim_session" => {
                let sid = params
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("session_id required"))?;
                Ok(serde_json::to_value(self.claim_session(sid).await?)?)
            }
            "release_session" => {
                let sid = params
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("session_id required"))?;
                Ok(serde_json::to_value(self.release_session(sid).await?)?)
            }
            "session_state" => {
                let sid = params
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("session_id required"))?;
                Ok(serde_json::to_value(self.get_session_state(sid).await?)?)
            }
            "navigate" => {
                let sid = params
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("session_id required"))?;
                let url = params
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("url required"))?;
                let (final_url, title) = self.navigate(sid, url).await?;
                Ok(serde_json::json!({ "url": final_url, "title": title }))
            }
            "screenshot" => {
                let sid = params
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("session_id required"))?;
                let bytes = self.screenshot(sid).await?;
                let b64 = base64::engine::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &bytes,
                );
                Ok(serde_json::json!({ "image_base64": b64, "size_bytes": bytes.len() }))
            }
            "click" => {
                let sid = params
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("session_id required"))?;
                let selector = params.get("selector").and_then(|v| v.as_str());
                let text = params.get("text").and_then(|v| v.as_str());
                let x = params.get("x").and_then(|v| v.as_i64()).map(|v| v as i32);
                let y = params.get("y").and_then(|v| v.as_i64()).map(|v| v as i32);
                self.click(sid, selector, text, x, y).await?;
                Ok(serde_json::json!({ "status": "clicked" }))
            }
            "type_text" => {
                let sid = params
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("session_id required"))?;
                let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let selector = params.get("selector").and_then(|v| v.as_str());
                let clear = params
                    .get("clear")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                self.type_text(sid, text, selector, clear).await?;
                Ok(serde_json::json!({ "status": "typed" }))
            }
            "scroll" => {
                let sid = params
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("session_id required"))?;
                let direction = params
                    .get("direction")
                    .and_then(|v| v.as_str())
                    .unwrap_or("down");
                let amount = params
                    .get("amount")
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32);
                self.scroll(sid, direction, amount).await?;
                Ok(serde_json::json!({ "status": "scrolled" }))
            }
            "get_content" => {
                let sid = params
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("session_id required"))?;
                let content = self.get_content(sid).await?;
                Ok(serde_json::to_value(content)?)
            }
            "close_session" => {
                let sid = params
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("session_id required"))?;
                self.close_session(sid).await?;
                Ok(serde_json::json!({ "status": "closed" }))
            }
            _ => Err(anyhow!("Unknown browser action: {}", action)),
        }
    }
}
