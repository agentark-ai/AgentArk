pub mod executor_client;
mod internal_auth;
pub mod workspace_client;

use anyhow::{anyhow, Result};
use std::net::IpAddr;

pub use executor_client::{
    AppLifecycleRequest, AppStatusResponse, CodeExecuteFilePayload, CodeExecuteRequest,
    ExecutorClient, ExecutorClientConfig, StackMemoryStatsResponse,
};
pub(crate) use internal_auth::{
    describe_internal_service_tokens_async, load_internal_service_token_from_default_config_dir,
    load_or_create_internal_service_token, read_persisted_internal_service_token_async,
    restore_internal_service_token_async, rotate_internal_service_token_async, InternalServiceKind,
};
pub use workspace_client::{WorkspaceClient, WorkspaceClientConfig};

pub(crate) fn host_looks_local_or_internal(host: &str) -> bool {
    let normalized = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }
    if matches!(normalized.as_str(), "localhost" | "127.0.0.1" | "::1") {
        return true;
    }
    if let Ok(ip) = normalized.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(ipv4) => ipv4.is_loopback() || ipv4.is_private() || ipv4.is_link_local(),
            IpAddr::V6(ipv6) => {
                ipv6.is_loopback() || ipv6.is_unique_local() || ipv6.is_unicast_link_local()
            }
        };
    }
    if !normalized.contains('.') {
        return true;
    }
    normalized.ends_with(".internal")
        || normalized.ends_with(".svc")
        || normalized.ends_with(".svc.cluster.local")
        || normalized.ends_with(".docker.internal")
}

fn allows_plain_http_internal_host(host: &str) -> bool {
    host_looks_local_or_internal(host)
}

pub(crate) fn validate_internal_service_base_url(base_url: &str, service_name: &str) -> Result<()> {
    let trimmed = base_url.trim();
    let parsed = reqwest::Url::parse(trimmed).map_err(|error| {
        anyhow!(
            "{} base URL '{}' is invalid: {}",
            service_name,
            trimmed,
            error
        )
    })?;

    match parsed.scheme() {
        "https" => Ok(()),
        "http" => {
            let host = parsed
                .host_str()
                .map(|value| value.trim().to_ascii_lowercase())
                .ok_or_else(|| {
                    anyhow!("{} base URL '{}' is missing a host", service_name, trimmed)
                })?;
            if allows_plain_http_internal_host(&host) {
                Ok(())
            } else {
                Err(anyhow!(
                    "{} base URL '{}' uses insecure plain HTTP for a non-internal host. Use HTTPS for off-box internal traffic.",
                    service_name,
                    trimmed
                ))
            }
        }
        other => Err(anyhow!(
            "{} base URL '{}' must use http or https, found '{}'",
            service_name,
            trimmed,
            other
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{host_looks_local_or_internal, validate_internal_service_base_url};

    #[test]
    fn allows_https_for_public_hosts() {
        assert!(
            validate_internal_service_base_url("https://example.com", "Executor service").is_ok()
        );
    }

    #[test]
    fn allows_http_for_loopback_and_private_hosts() {
        assert!(
            validate_internal_service_base_url("http://127.0.0.1:8991", "Executor service").is_ok()
        );
        assert!(
            validate_internal_service_base_url("http://10.0.0.5:8991", "Executor service").is_ok()
        );
        assert!(
            validate_internal_service_base_url("http://[fd00::1]:8991", "Executor service").is_ok()
        );
    }

    #[test]
    fn allows_http_for_internal_service_dns_names() {
        assert!(validate_internal_service_base_url(
            "http://agentark-workspace:8992",
            "Workspace service"
        )
        .is_ok());
        assert!(validate_internal_service_base_url(
            "http://workspace.default.svc.cluster.local:8992",
            "Workspace service"
        )
        .is_ok());
        assert!(validate_internal_service_base_url(
            "http://host.docker.internal:8992",
            "Workspace service"
        )
        .is_ok());
    }

    #[test]
    fn recognizes_local_and_internal_hosts() {
        assert!(host_looks_local_or_internal("localhost"));
        assert!(host_looks_local_or_internal("127.0.0.1"));
        assert!(host_looks_local_or_internal("10.0.0.5"));
        assert!(host_looks_local_or_internal("192.168.1.8"));
        assert!(host_looks_local_or_internal("host.docker.internal"));
        assert!(host_looks_local_or_internal("ollama"));
        assert!(host_looks_local_or_internal(
            "workspace.default.svc.cluster.local"
        ));
        assert!(!host_looks_local_or_internal("openrouter.ai"));
        assert!(!host_looks_local_or_internal("api.openai.com"));
    }

    #[test]
    fn rejects_http_for_public_hosts() {
        let error = validate_internal_service_base_url("http://example.com", "Executor service")
            .expect_err("public http host should be rejected");
        assert!(error
            .to_string()
            .contains("insecure plain HTTP for a non-internal host"));
    }
}
