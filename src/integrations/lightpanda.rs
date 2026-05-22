//! Lightpanda fast content extraction
//!
//! Shells out to the `lightpanda` CLI binary for fast markdown extraction.
//! Falls back gracefully when the binary is not installed.

use anyhow::{Result, anyhow};
use once_cell::sync::Lazy;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

/// Maximum time to wait for a single Lightpanda fetch.
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

/// Maximum output size before truncation (200KB).
const MAX_OUTPUT_BYTES: usize = 200_000;

static LIGHTPANDA_BINARY: Lazy<Option<PathBuf>> = Lazy::new(resolve_lightpanda_binary);

pub fn is_available() -> bool {
    LIGHTPANDA_BINARY.is_some()
}

pub fn binary_path() -> Option<&'static Path> {
    LIGHTPANDA_BINARY.as_deref()
}

fn resolve_lightpanda_binary() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("AGENTARK_LIGHTPANDA_PATH") {
        let candidate = PathBuf::from(explicit);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    let mut executable_names = vec!["lightpanda".to_string()];
    if cfg!(windows) {
        let pathext = std::env::var_os("PATHEXT")
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string());
        for ext in pathext
            .split(';')
            .map(str::trim)
            .filter(|ext| !ext.is_empty())
        {
            executable_names.push(format!("lightpanda{}", ext));
        }
    }

    let Some(path_var) = std::env::var_os("PATH") else {
        return None;
    };

    for dir in std::env::split_paths(&path_var) {
        for executable_name in &executable_names {
            let candidate = dir.join(executable_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

fn lightpanda_program() -> Result<&'static Path> {
    binary_path().ok_or_else(|| {
        anyhow!(
            "lightpanda binary is unavailable in this runtime; update or rebuild the bundled AgentArk runtime so it includes Lightpanda"
        )
    })
}

/// Fetch a URL and return its content as markdown.
///
/// Uses `lightpanda fetch --dump markdown <url>`.
/// Returns `Err` if the binary is missing, the process times out, or exits non-zero.
pub async fn fetch_markdown(url: &str) -> Result<String> {
    let child = Command::new(lightpanda_program()?)
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
        .map_err(|e| {
            anyhow!(
                "bundled Lightpanda is unavailable or not executable in this runtime: {}",
                e
            )
        })?;

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
    let child = Command::new(lightpanda_program()?)
        .args(["fetch", "--dump", "html", "--http_timeout", "12000", url])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            anyhow!(
                "bundled Lightpanda is unavailable or not executable in this runtime: {}",
                e
            )
        })?;

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
