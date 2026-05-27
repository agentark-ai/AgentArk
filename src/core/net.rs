use anyhow::{anyhow, Result};

pub fn internal_api_base_url() -> String {
    let bind_addr = std::env::var("AGENTARK_BIND").unwrap_or_else(|_| "127.0.0.1:8990".to_string());
    let tls_enabled = std::env::var("AGENTARK_TLS_CERT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .is_some()
        && std::env::var("AGENTARK_TLS_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .is_some();
    let scheme = if tls_enabled { "https" } else { "http" };
    format!("{}://{}", scheme, bind_addr)
}

pub fn build_internal_control_client(timeout_secs: u64) -> Result<reqwest::Client> {
    let mut builder =
        reqwest::Client::builder().timeout(std::time::Duration::from_secs(timeout_secs.max(1)));

    if let Some(cert_path) = std::env::var("AGENTARK_TLS_CERT")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        let cert_bytes = std::fs::read(&cert_path)
            .map_err(|e| anyhow::anyhow!("Failed to read TLS cert '{}': {}", cert_path, e))?;
        let cert = reqwest::Certificate::from_pem(&cert_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to parse TLS cert '{}': {}", cert_path, e))?;
        builder = builder.add_root_certificate(cert);
    }

    Ok(builder.build()?)
}

pub fn is_private_or_local_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.octets()[0] == 0
                || (v4.octets()[0] == 169 && v4.octets()[1] == 254)
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || v6.is_unicast_link_local()
                || v6.is_unique_local()
        }
    }
}

pub fn is_disallowed_public_hostname(host: &str) -> bool {
    let normalized = host.trim().trim_end_matches('.').to_ascii_lowercase();
    normalized.is_empty()
        || normalized == "localhost"
        || normalized.ends_with(".localhost")
        || normalized.ends_with(".local")
        || normalized == "0.0.0.0"
        || normalized == "[::]"
}

pub fn validate_no_userinfo(url: &reqwest::Url) -> Result<()> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(anyhow!("Userinfo is not allowed in external URLs"));
    }
    Ok(())
}

pub async fn validate_external_https_url(raw: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(raw).map_err(|error| anyhow!("Invalid URL: {}", error))?;
    if url.scheme() != "https" {
        return Err(anyhow!(
            "Only HTTPS URLs are supported for external extensions"
        ));
    }
    validate_no_userinfo(&url)?;
    validate_public_url_host(&url).await?;
    Ok(url)
}

pub async fn validate_public_url_host(url: &reqwest::Url) -> Result<()> {
    let host = url
        .host()
        .ok_or_else(|| anyhow!("URL must include a host"))?;
    let port = url.port_or_known_default().unwrap_or(443);
    if port == 8990 {
        return Err(anyhow!(
            "AgentArk control ports are not valid external extension endpoints"
        ));
    }
    match host {
        url::Host::Domain(domain) => {
            if is_disallowed_public_hostname(domain) {
                return Err(anyhow!("Disallowed public host"));
            }
            let mut resolved_any = false;
            let addrs = tokio::net::lookup_host((domain, port))
                .await
                .map_err(|_| anyhow!("Failed to resolve host"))?;
            for addr in addrs {
                resolved_any = true;
                if is_private_or_local_ip(addr.ip()) {
                    return Err(anyhow!("URL resolves to a private or local IP"));
                }
            }
            if !resolved_any {
                return Err(anyhow!("Failed to resolve host"));
            }
        }
        url::Host::Ipv4(ip) => {
            if is_private_or_local_ip(std::net::IpAddr::V4(ip)) {
                return Err(anyhow!("URL IP is private or local"));
            }
        }
        url::Host::Ipv6(ip) => {
            if is_private_or_local_ip(std::net::IpAddr::V6(ip)) {
                return Err(anyhow!("URL IP is private or local"));
            }
        }
    }
    Ok(())
}

pub async fn validate_public_https_url(raw: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(raw).map_err(|error| anyhow!("Invalid URL: {}", error))?;
    if url.scheme() != "https" {
        return Err(anyhow!("Only HTTPS URLs are supported"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(anyhow!("Userinfo is not allowed in public URLs"));
    }
    if let Some(port) = url.port() {
        if port != 443 {
            return Err(anyhow!("Only port 443 is allowed in public URLs"));
        }
    }

    let host = url
        .host()
        .ok_or_else(|| anyhow!("URL must include a host"))?;
    match host {
        url::Host::Domain(domain) => {
            if is_disallowed_public_hostname(domain) {
                return Err(anyhow!("Disallowed public host"));
            }
            let mut resolved_any = false;
            let addrs = tokio::net::lookup_host((domain, 443))
                .await
                .map_err(|_| anyhow!("Failed to resolve host"))?;
            for addr in addrs {
                resolved_any = true;
                if is_private_or_local_ip(addr.ip()) {
                    return Err(anyhow!("URL resolves to a private or local IP"));
                }
            }
            if !resolved_any {
                return Err(anyhow!("Failed to resolve host"));
            }
        }
        url::Host::Ipv4(ip) => {
            if is_private_or_local_ip(std::net::IpAddr::V4(ip)) {
                return Err(anyhow!("URL IP is private or local"));
            }
        }
        url::Host::Ipv6(ip) => {
            if is_private_or_local_ip(std::net::IpAddr::V6(ip)) {
                return Err(anyhow!("URL IP is private or local"));
            }
        }
    }

    Ok(url)
}
