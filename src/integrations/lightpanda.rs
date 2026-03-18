//! Lightpanda fast content extraction
//!
//! Shells out to the `lightpanda` CLI binary for fast markdown extraction.
//! Falls back gracefully when the binary is not installed.

use anyhow::{anyhow, Result};
use std::time::Duration;
use tokio::process::Command;

/// Maximum time to wait for a single Lightpanda fetch.
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

/// Maximum output size before truncation (200KB).
const MAX_OUTPUT_BYTES: usize = 200_000;

/// Check whether the `lightpanda` binary is available on PATH.
#[cfg_attr(not(test), allow(dead_code))]
pub async fn is_available() -> bool {
    Command::new("lightpanda")
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Fetch a URL and return its content as markdown.
///
/// Uses `lightpanda fetch --dump markdown <url>`.
/// Returns `Err` if the binary is missing, the process times out, or exits non-zero.
pub async fn fetch_markdown(url: &str) -> Result<String> {
    let child = Command::new("lightpanda")
        .args([
            "fetch",
            "--dump",
            "markdown",
            "--http_timeout",
            "12000",
            url,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| anyhow!("lightpanda binary not found or not executable: {}", e))?;

    let output = tokio::time::timeout(FETCH_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| anyhow!("lightpanda fetch timed out after {:?}", FETCH_TIMEOUT))?
        .map_err(|e| anyhow!("lightpanda process error: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "lightpanda exited with {}: {}",
            output.status,
            stderr.chars().take(500).collect::<String>()
        ));
    }

    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.len() > MAX_OUTPUT_BYTES {
        text.truncate(MAX_OUTPUT_BYTES);
        text.push_str("\n\n(content truncated)");
    }

    if text.trim().is_empty() {
        return Err(anyhow!("lightpanda returned empty content for {}", url));
    }

    Ok(text)
}

/// Fetch a URL and return raw HTML.
///
/// Uses `lightpanda fetch --dump html <url>`.
pub async fn fetch_html(url: &str) -> Result<String> {
    let child = Command::new("lightpanda")
        .args(["fetch", "--dump", "html", "--http_timeout", "12000", url])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| anyhow!("lightpanda binary not found: {}", e))?;

    let output = tokio::time::timeout(FETCH_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| anyhow!("lightpanda fetch timed out"))?
        .map_err(|e| anyhow!("lightpanda process error: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "lightpanda exited with {}: {}",
            output.status,
            stderr.chars().take(500).collect::<String>()
        ));
    }

    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.len() > MAX_OUTPUT_BYTES {
        text.truncate(MAX_OUTPUT_BYTES);
        text.push_str("\n\n(content truncated)");
    }

    Ok(text)
}
