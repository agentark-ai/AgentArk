use anyhow::{Context, Result};
use serde::Deserialize;
use std::cmp::Ordering;

pub const DEFAULT_RELEASE_REPO: &str = "agentark-ai/AgentArk";

const RELEASE_REPO_ENV_KEYS: &[&str] = &["AGENTARK_RELEASE_REPO"];

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedReleaseVersion {
    major: u64,
    minor: u64,
    patch: u64,
    pre_release: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LatestReleaseInfo {
    pub tag_name: String,
    pub version: String,
    pub html_url: String,
}

#[derive(Debug, Deserialize)]
struct GitHubLatestReleasePayload {
    tag_name: String,
    html_url: String,
}

pub fn configured_release_repo_from_env() -> Option<String> {
    RELEASE_REPO_ENV_KEYS.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

pub fn release_repo_slug() -> String {
    configured_release_repo_from_env().unwrap_or_else(|| DEFAULT_RELEASE_REPO.to_string())
}

pub fn strip_release_tag_prefix(tag: &str) -> String {
    tag.trim().trim_start_matches(['v', 'V']).to_string()
}

pub fn image_repository(image_ref: &str) -> String {
    let without_digest = image_ref
        .trim()
        .split('@')
        .next()
        .unwrap_or(image_ref.trim());
    let last_slash = without_digest.rfind('/');
    let last_colon = without_digest.rfind(':');
    if let Some(colon_idx) = last_colon {
        if last_slash
            .map(|slash_idx| colon_idx > slash_idx)
            .unwrap_or(true)
        {
            return without_digest[..colon_idx].to_string();
        }
    }
    without_digest.to_string()
}

pub fn runtime_image_repository() -> String {
    image_repository(&crate::core::runtime::runtime_image::default_runtime_image())
}

pub fn ui_update_supported_image(image_ref: &str) -> bool {
    image_repository(image_ref).contains('/')
}

pub fn is_release_version_newer(current_version: &str, latest_version: &str) -> Option<bool> {
    let current = parse_release_version(current_version)?;
    let latest = parse_release_version(latest_version)?;
    Some(latest.cmp(&current).is_gt())
}

pub async fn fetch_latest_release_info(
    client: &reqwest::Client,
    repo_slug: &str,
) -> Result<LatestReleaseInfo> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        repo_slug.trim().trim_matches('/')
    );
    let payload = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header(
            reqwest::header::USER_AGENT,
            crate::branding::versioned_user_agent(),
        )
        .send()
        .await
        .context("Failed to request the latest AgentArk release")?
        .error_for_status()
        .context("Latest AgentArk release request returned an error")?
        .json::<GitHubLatestReleasePayload>()
        .await
        .context("Failed to decode latest AgentArk release metadata")?;
    Ok(LatestReleaseInfo {
        version: strip_release_tag_prefix(&payload.tag_name),
        tag_name: payload.tag_name,
        html_url: payload.html_url,
    })
}

fn parse_release_version(input: &str) -> Option<ParsedReleaseVersion> {
    let trimmed = strip_release_tag_prefix(input);
    let core = trimmed
        .split_once('+')
        .map(|(base, _)| base)
        .unwrap_or(&trimmed);
    let (version_core, pre_release) = match core.split_once('-') {
        Some((base, pre)) if !pre.trim().is_empty() => (base, Some(pre.trim().to_string())),
        _ => (core, None),
    };
    let mut parts = version_core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(ParsedReleaseVersion {
        major,
        minor,
        patch,
        pre_release,
    })
}

impl Ord for ParsedReleaseVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre_release, &other.pre_release) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(left), Some(right)) => left.cmp(right),
            })
    }
}

impl PartialOrd for ParsedReleaseVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        image_repository, is_release_version_newer, strip_release_tag_prefix,
        ui_update_supported_image,
    };

    #[test]
    fn strips_release_tag_prefix() {
        assert_eq!(strip_release_tag_prefix("v1.2.3"), "1.2.3");
        assert_eq!(strip_release_tag_prefix(" V2.0.0 "), "2.0.0");
        assert_eq!(strip_release_tag_prefix("1.4.0"), "1.4.0");
    }

    #[test]
    fn compares_release_versions() {
        assert_eq!(is_release_version_newer("1.2.3", "1.2.4"), Some(true));
        assert_eq!(is_release_version_newer("1.2.3", "1.2.3"), Some(false));
        assert_eq!(is_release_version_newer("1.2.3-rc1", "1.2.3"), Some(true));
        assert_eq!(is_release_version_newer("invalid", "1.2.3"), None);
    }

    #[test]
    fn extracts_image_repository() {
        assert_eq!(
            image_repository("ghcr.io/agentark-ai/agentark:1.2.3"),
            "ghcr.io/agentark-ai/agentark"
        );
        assert_eq!(
            image_repository("registry.example.com:5000/agentark/runtime:latest"),
            "registry.example.com:5000/agentark/runtime"
        );
        assert_eq!(
            image_repository("ghcr.io/agentark-ai/agentark@sha256:abc"),
            "ghcr.io/agentark-ai/agentark"
        );
    }

    #[test]
    fn detects_ui_update_support_for_managed_images() {
        assert!(ui_update_supported_image(
            "ghcr.io/agentark-ai/agentark:latest"
        ));
        assert!(!ui_update_supported_image("agentark:dev"));
    }
}
