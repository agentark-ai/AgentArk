use super::*;

pub(super) fn normalize_origin(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let uri: Uri = trimmed.parse().ok()?;
    let scheme = uri.scheme_str()?.to_ascii_lowercase();
    let authority = uri.authority()?.as_str().to_ascii_lowercase();
    Some(format!("{}://{}", scheme, authority))
}

pub(super) fn generate_ephemeral_token() -> String {
    use rand::Rng;
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    base64::engine::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes)
}

pub(super) fn parse_env_truthy(key: &str) -> Option<bool> {
    std::env::var(key).ok().and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else if trimmed.eq_ignore_ascii_case("true") || trimmed == "1" {
            Some(true)
        } else if trimmed.eq_ignore_ascii_case("false") || trimmed == "0" {
            Some(false)
        } else {
            None
        }
    })
}

pub(super) fn normalize_optional_url(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_end_matches('/').to_string())
}

pub(super) fn deployment_mode_from_config(
    config: &crate::core::config::AgentConfig,
) -> DeploymentMode {
    if let Some(force_mode) = std::env::var("AGENTARK_DEPLOYMENT_MODE")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
    {
        if force_mode == "internet_facing" || force_mode == "internet-facing" {
            return DeploymentMode::InternetFacing;
        }
        if force_mode == "trusted_local" || force_mode == "trusted-local" {
            return DeploymentMode::TrustedLocal;
        }
    }
    config.deployment_mode
}

pub(super) fn public_app_bind_addr_from_config(
    config: &crate::core::config::AgentConfig,
    deployment_mode: DeploymentMode,
) -> Option<String> {
    normalize_optional_url(std::env::var("AGENTARK_PUBLIC_APP_BIND").ok().as_deref())
        .or_else(|| {
            config
                .public_apps
                .bind_addr
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
        .or_else(|| {
            if deployment_mode == DeploymentMode::InternetFacing {
                Some("127.0.0.1:8992".to_string())
            } else {
                None
            }
        })
}

pub(super) fn public_app_base_url_from_config(
    config: &crate::core::config::AgentConfig,
) -> Option<String> {
    normalize_optional_url(
        std::env::var("AGENTARK_PUBLIC_APP_BASE_URL")
            .ok()
            .as_deref(),
    )
    .or_else(|| normalize_optional_url(config.public_apps.base_url.as_deref()))
}

pub(super) fn display_addr_for_bind_addr(bind_addr: &str) -> Option<String> {
    let trimmed = bind_addr.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = if trimmed.starts_with("0.0.0.0:") {
        format!("localhost:{}", trimmed.trim_start_matches("0.0.0.0:"))
    } else if trimmed == "0.0.0.0" {
        "localhost".to_string()
    } else if trimmed.starts_with("[::]:") {
        format!("localhost:{}", trimmed.trim_start_matches("[::]:"))
    } else if trimmed == "[::]" || trimmed == "::" {
        "localhost".to_string()
    } else if trimmed.starts_with("127.0.0.1:") || trimmed == "127.0.0.1" {
        trimmed.replacen("127.0.0.1", "localhost", 1)
    } else {
        trimmed.to_string()
    };
    Some(normalized)
}

pub(super) fn display_url_for_bind_addr(bind_addr: &str, scheme: &str) -> Option<String> {
    let normalized = display_addr_for_bind_addr(bind_addr)?;
    Some(format!(
        "{}://{}",
        scheme.trim_end_matches("://"),
        normalized.trim_end_matches('/')
    ))
}

pub(super) fn default_base_url_for_bind_addr(bind_addr: &str) -> Option<String> {
    let normalized = display_addr_for_bind_addr(bind_addr)?;
    Some(format!("http://{}", normalized.trim_end_matches('/')))
}

pub(super) fn bind_addr_host(bind_addr: &str) -> Option<String> {
    let trimmed = bind_addr.trim();
    if trimmed.is_empty() {
        return None;
    }
    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{}", trimmed)
    };
    reqwest::Url::parse(&candidate)
        .ok()?
        .host_str()
        .map(|value| value.trim().to_ascii_lowercase())
}

pub(super) fn bind_addr_is_loopback(bind_addr: &str) -> bool {
    bind_addr_host(bind_addr)
        .as_deref()
        .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"))
}

pub(super) fn should_warn_for_direct_control_plane_exposure(
    deployment_mode: DeploymentMode,
    bind_addr: &str,
) -> bool {
    deployment_mode == DeploymentMode::InternetFacing && !bind_addr_is_loopback(bind_addr)
}

pub(super) fn validate_public_app_listener_posture(
    deployment_mode: DeploymentMode,
    public_app_bind_addr: Option<&str>,
    configured_public_app_base_url: Option<&str>,
    direct_tls_enabled: bool,
) -> Result<()> {
    if !internet_facing_apps_should_be_isolated(deployment_mode, public_app_bind_addr) {
        return Ok(());
    }

    let bind_addr = public_app_bind_addr
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Internet-facing public apps require a bind address"))?;
    let base_url = configured_public_app_base_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Internet-facing public apps require AGENTARK_PUBLIC_APP_BASE_URL or [public_apps].base_url to be set to the external HTTPS origin"
            )
        })?;
    let parsed_base_url = reqwest::Url::parse(base_url).map_err(|error| {
        anyhow::anyhow!("Public app base URL '{}' is invalid: {}", base_url, error)
    })?;
    if !parsed_base_url.scheme().eq_ignore_ascii_case("https") {
        anyhow::bail!("Internet-facing public app base URL must use HTTPS");
    }

    if !bind_addr_is_loopback(bind_addr) {
        if !direct_tls_enabled {
            anyhow::bail!(
                "Internet-facing public app listener '{}' is non-loopback but TLS is not configured. Either bind public apps to loopback behind an HTTPS reverse proxy, or configure tls_cert_path/tls_key_path for direct HTTPS.",
                bind_addr
            );
        }
        bind_addr.parse::<SocketAddr>().map_err(|error| {
            anyhow::anyhow!(
                "Direct HTTPS public app listener '{}' must be a concrete socket address: {}",
                bind_addr,
                error
            )
        })?;
    }

    Ok(())
}

pub(super) fn internet_facing_apps_should_be_isolated(
    deployment_mode: DeploymentMode,
    public_app_bind_addr: Option<&str>,
) -> bool {
    deployment_mode == DeploymentMode::InternetFacing
        && public_app_bind_addr
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
}

pub(super) fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
