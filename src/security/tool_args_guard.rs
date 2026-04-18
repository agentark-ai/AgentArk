//! Tool-argument guard.
//!
//! Outward-facing tools (web request actions, MCP bridges, browser
//! automation, shell runners) must not execute with arguments that point
//! at internal infrastructure (loopback, RFC1918, link-local, cloud
//! metadata) unless the operator has explicitly opted in for a specific
//! host. This module wraps the existing `core::net` checks with a
//! deterministic per-project whitelist that lets self-hosted deployments
//! reach known internal hosts without weakening the default posture.
//!
//! The guard uses only structural signals (URL parsing, IP-range
//! membership, exact-match hostnames) — nothing about attacker phrasing or
//! anticipated wording.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::core::net::{
    is_disallowed_public_hostname, is_private_or_local_ip, validate_public_https_url,
};

/// Hosts and IPs the operator has explicitly allowed for outward tool
/// invocations even though they match an internal address shape.
///
/// Entries are normalized to lowercase, trimmed, and stripped of a trailing
/// dot. An entry matches when the URL's host equals the entry exactly, OR
/// when the URL's host is a subdomain of the entry (e.g. whitelisting
/// `api.internal` also allows `billing.api.internal`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolArgsGuardConfig {
    #[serde(default)]
    pub host_whitelist: Vec<String>,
}

#[allow(dead_code)]
impl ToolArgsGuardConfig {
    fn normalized_entries(&self) -> Vec<String> {
        self.host_whitelist
            .iter()
            .map(|value| value.trim().trim_end_matches('.').to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .collect()
    }

    pub fn contains_host(&self, host: &str) -> bool {
        let needle = host.trim().trim_end_matches('.').to_ascii_lowercase();
        if needle.is_empty() {
            return false;
        }
        for entry in self.normalized_entries() {
            if needle == entry {
                return true;
            }
            if needle.ends_with(&format!(".{}", entry)) {
                return true;
            }
        }
        false
    }
}

/// Result of a guard check. The caller can convert to an `anyhow::Error`
/// with `into_error` if it prefers the existing error-return style; the
/// typed variant exists so the caller (and future telemetry layer) can log
/// a structured reason rather than string-matching error messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[allow(dead_code)]
pub enum GuardDenial {
    DisallowedHostname,
    PrivateOrLocalIp,
    DnsResolutionFailed,
    InvalidUrl,
    SchemeNotHttps,
    UserinfoNotAllowed,
    NonStandardPort,
    HostMissing,
}

#[allow(dead_code)]
impl GuardDenial {
    pub fn reason_code(&self) -> &'static str {
        match self {
            GuardDenial::DisallowedHostname => "disallowed_hostname",
            GuardDenial::PrivateOrLocalIp => "private_or_local_ip",
            GuardDenial::DnsResolutionFailed => "dns_resolution_failed",
            GuardDenial::InvalidUrl => "invalid_url",
            GuardDenial::SchemeNotHttps => "scheme_not_https",
            GuardDenial::UserinfoNotAllowed => "userinfo_not_allowed",
            GuardDenial::NonStandardPort => "non_standard_port",
            GuardDenial::HostMissing => "host_missing",
        }
    }

    pub fn into_error(self) -> anyhow::Error {
        anyhow!("tool argument denied ({})", self.reason_code())
    }
}

/// Parse and validate an outward HTTPS URL, consulting the whitelist.
///
/// If the URL would otherwise be rejected because its host resolves to a
/// private/local IP OR matches a disallowed hostname category (`localhost`,
/// `*.local`, etc.), and the host is present in `config.host_whitelist`,
/// the URL is accepted anyway. All other rejections (invalid URL, wrong
/// scheme, userinfo, non-443 port) stand regardless of whitelist membership
/// — those are not "internal host" concerns and widening them would weaken
/// the posture for whitelisted entries.
#[allow(dead_code)]
pub async fn check_outward_url(
    raw_url: &str,
    config: &ToolArgsGuardConfig,
) -> std::result::Result<reqwest::Url, GuardDenial> {
    match validate_public_https_url(raw_url).await {
        Ok(url) => Ok(url),
        Err(_) => {
            let url = reqwest::Url::parse(raw_url).map_err(|_| GuardDenial::InvalidUrl)?;
            if url.scheme() != "https" {
                return Err(GuardDenial::SchemeNotHttps);
            }
            if !url.username().is_empty() || url.password().is_some() {
                return Err(GuardDenial::UserinfoNotAllowed);
            }
            if let Some(port) = url.port() {
                if port != 443 {
                    return Err(GuardDenial::NonStandardPort);
                }
            }
            let host = url.host().ok_or(GuardDenial::HostMissing)?;
            let host_str = host.to_string();
            if config.contains_host(&host_str) {
                // Operator explicitly allowed this host. Still verify the URL
                // is structurally well-formed (done above) before returning.
                return Ok(url);
            }

            // Re-run structural checks to classify the denial precisely so
            // telemetry can distinguish SSRF-style rejections from malformed
            // input.
            match host {
                url::Host::Domain(domain) => {
                    if is_disallowed_public_hostname(domain) {
                        return Err(GuardDenial::DisallowedHostname);
                    }
                    match tokio::net::lookup_host((domain, 443)).await {
                        Ok(addrs) => {
                            let mut any = false;
                            for addr in addrs {
                                any = true;
                                if is_private_or_local_ip(addr.ip()) {
                                    return Err(GuardDenial::PrivateOrLocalIp);
                                }
                            }
                            if !any {
                                return Err(GuardDenial::DnsResolutionFailed);
                            }
                        }
                        Err(_) => return Err(GuardDenial::DnsResolutionFailed),
                    }
                    Err(GuardDenial::DisallowedHostname)
                }
                url::Host::Ipv4(ip) => {
                    if is_private_or_local_ip(std::net::IpAddr::V4(ip)) {
                        Err(GuardDenial::PrivateOrLocalIp)
                    } else {
                        Err(GuardDenial::InvalidUrl)
                    }
                }
                url::Host::Ipv6(ip) => {
                    if is_private_or_local_ip(std::net::IpAddr::V6(ip)) {
                        Err(GuardDenial::PrivateOrLocalIp)
                    } else {
                        Err(GuardDenial::InvalidUrl)
                    }
                }
            }
        }
    }
}

/// Convenience wrapper that returns `Result<Url>` for callers that already
/// use `anyhow::Error` throughout.
#[allow(dead_code)]
pub async fn check_outward_url_anyhow(
    raw_url: &str,
    config: &ToolArgsGuardConfig,
) -> Result<reqwest::Url> {
    check_outward_url(raw_url, config)
        .await
        .map_err(|denial| denial.into_error())
}

fn collect_absolute_http_urls(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if let Ok(url) = reqwest::Url::parse(trimmed) {
                if matches!(url.scheme(), "http" | "https") {
                    out.push(trimmed.to_string());
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_absolute_http_urls(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for value in map.values() {
                collect_absolute_http_urls(value, out);
            }
        }
        _ => {}
    }
}

pub async fn check_outward_urls_in_json_anyhow(
    value: &serde_json::Value,
    config: &ToolArgsGuardConfig,
) -> Result<()> {
    let mut urls = Vec::new();
    collect_absolute_http_urls(value, &mut urls);
    for url in urls {
        check_outward_url_anyhow(&url, config).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist_entry_matches_exact_host() {
        let config = ToolArgsGuardConfig {
            host_whitelist: vec!["api.internal".into()],
        };
        assert!(config.contains_host("api.internal"));
        assert!(config.contains_host("API.Internal"));
        assert!(config.contains_host("api.internal."));
    }

    #[test]
    fn whitelist_entry_matches_subdomain() {
        let config = ToolArgsGuardConfig {
            host_whitelist: vec!["internal".into()],
        };
        assert!(config.contains_host("billing.internal"));
        assert!(config.contains_host("service.billing.internal"));
    }

    #[test]
    fn whitelist_rejects_unrelated_hosts() {
        let config = ToolArgsGuardConfig {
            host_whitelist: vec!["internal".into()],
        };
        assert!(!config.contains_host("external.example.com"));
        assert!(!config.contains_host(""));
    }

    #[test]
    fn denial_reason_codes_are_stable() {
        assert_eq!(
            GuardDenial::PrivateOrLocalIp.reason_code(),
            "private_or_local_ip"
        );
        assert_eq!(
            GuardDenial::DisallowedHostname.reason_code(),
            "disallowed_hostname"
        );
    }
}
