use super::super::*;

impl ActionRuntime {
    pub(in crate::runtime) fn loopback_http_get_allowed(url: &reqwest::Url) -> Result<()> {
        let port = url
            .port_or_known_default()
            .ok_or_else(|| anyhow::anyhow!("URL is missing a usable port"))?;
        if port != LOCAL_APP_HTTP_PORT {
            anyhow::bail!(
                "Loopback http_get is restricted to the local app host on port {}",
                LOCAL_APP_HTTP_PORT
            );
        }

        let path = url.path();
        if path != "/apps" && !path.starts_with("/apps/") {
            anyhow::bail!("Loopback http_get is restricted to deployed app URLs under /apps/");
        }
        Ok(())
    }

    pub(in crate::runtime) fn host_is_explicitly_local(host: &str) -> bool {
        let normalized = host.trim().trim_end_matches('.').to_ascii_lowercase();
        if normalized == "localhost" {
            return true;
        }
        normalized.parse::<IpAddr>().is_ok_and(|ip| match ip {
            IpAddr::V4(v4) => v4.is_loopback(),
            IpAddr::V6(v6) => v6.is_loopback(),
        })
    }

    pub(in crate::runtime) fn ipv4_is_public(ip: Ipv4Addr) -> bool {
        let octets = ip.octets();
        !(ip.is_private()
            || ip.is_loopback()
            || ip.is_link_local()
            || ip.is_multicast()
            || ip.is_unspecified()
            || octets == [255, 255, 255, 255]
            || octets[0] == 0
            || (octets[0] == 100 && (64..=127).contains(&octets[1]))
            || (octets[0] == 169 && octets[1] == 254)
            || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
            || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
            || (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
            || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
            || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113))
    }

    pub(in crate::runtime) fn ipv6_is_public(ip: Ipv6Addr) -> bool {
        !(ip.is_loopback()
            || ip.is_unspecified()
            || ip.is_multicast()
            || ip.is_unique_local()
            || ip.is_unicast_link_local())
    }

    pub(in crate::runtime) fn ip_is_public(ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(v4) => Self::ipv4_is_public(v4),
            IpAddr::V6(v6) => Self::ipv6_is_public(v6),
        }
    }

    pub(in crate::runtime) fn parse_http_get_url(raw_url: &str) -> Result<reqwest::Url> {
        let trimmed = raw_url.trim();
        if trimmed.is_empty() {
            anyhow::bail!("Missing URL");
        }
        let candidate = if trimmed.contains("://") {
            trimmed.to_string()
        } else {
            format!("https://{}", trimmed.trim_start_matches("//"))
        };
        let parsed = reqwest::Url::parse(&candidate)?;
        if !matches!(parsed.scheme(), "http" | "https") {
            anyhow::bail!("http_get only supports http:// and https:// URLs");
        }
        if parsed.host_str().is_none() {
            anyhow::bail!("URL must include a host");
        }
        Ok(parsed)
    }

    pub(in crate::runtime) fn http_get_url_is_privateish(url: &reqwest::Url) -> bool {
        let host = url
            .host_str()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        Self::host_is_explicitly_local(&host)
            || host.ends_with(".local")
            || host.ends_with(".internal")
            || host.ends_with(".home")
            || host.ends_with(".lan")
            || host
                .parse::<IpAddr>()
                .is_ok_and(|ip| !Self::ip_is_public(ip))
    }

    pub(in crate::runtime) async fn validate_http_get_url(
        &self,
        raw_url: &str,
    ) -> Result<reqwest::Url> {
        let parsed = Self::parse_http_get_url(raw_url)?;
        if !parsed.username().is_empty() || parsed.password().is_some() {
            anyhow::bail!("Embedded credentials are not allowed in http_get URLs");
        }

        let host = parsed
            .host_str()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if Self::host_is_explicitly_local(&host) {
            Self::loopback_http_get_allowed(&parsed)?;
            return Ok(parsed);
        }
        if host.ends_with(".local")
            || host.ends_with(".internal")
            || host.ends_with(".home")
            || host.ends_with(".lan")
        {
            anyhow::bail!("Local network hostnames are blocked by http_get");
        }

        if let Ok(ip) = host.parse::<IpAddr>() {
            if !Self::ip_is_public(ip) {
                anyhow::bail!("http_get cannot target private or link-local IP addresses");
            }
            return Ok(parsed);
        }

        let port = parsed.port_or_known_default().unwrap_or(80);
        let mut resolved_any = false;
        for addr in tokio::net::lookup_host((host.as_str(), port)).await? {
            resolved_any = true;
            if !Self::ip_is_public(addr.ip()) {
                anyhow::bail!(
                    "http_get cannot target internal address {} resolved from {}",
                    addr.ip(),
                    host
                );
            }
        }
        if !resolved_any {
            anyhow::bail!("Unable to resolve host '{}'", host);
        }

        Ok(parsed)
    }

    pub(in crate::runtime) async fn resolve_http_get_url_for_context(
        &self,
        raw_url: &str,
        auth_context: &ActionAuthorizationContext,
    ) -> Result<reqwest::Url> {
        if Self::direct_trusted_chat_tool_override(auth_context) {
            return Self::parse_http_get_url(raw_url);
        }
        self.validate_http_get_url(raw_url).await
    }

    pub(in crate::runtime) async fn validate_connector_request_url(
        &self,
        raw_url: &str,
    ) -> Result<reqwest::Url> {
        let parsed = reqwest::Url::parse(raw_url)?;
        if !matches!(parsed.scheme(), "http" | "https") {
            anyhow::bail!("Public HTTP requests only support http:// and https:// URLs");
        }
        if parsed.host_str().is_none() {
            anyhow::bail!("Public HTTP requests require a URL host");
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            anyhow::bail!("Embedded credentials are not allowed in public HTTP request URLs");
        }

        let host = parsed
            .host_str()
            .unwrap_or_default()
            .trim()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if Self::host_is_explicitly_local(&host) {
            anyhow::bail!("Public HTTP requests cannot target localhost or loopback addresses");
        }
        if host.ends_with(".local")
            || host.ends_with(".internal")
            || host.ends_with(".home")
            || host.ends_with(".lan")
        {
            anyhow::bail!("Public HTTP requests cannot target local network hostnames");
        }

        if let Ok(ip) = host.parse::<IpAddr>() {
            if !Self::ip_is_public(ip) {
                anyhow::bail!(
                    "Public HTTP requests cannot target private or link-local IP addresses"
                );
            }
            return Ok(parsed);
        }

        let port = parsed.port_or_known_default().unwrap_or(80);
        let mut resolved_any = false;
        for addr in tokio::net::lookup_host((host.as_str(), port)).await? {
            resolved_any = true;
            if !Self::ip_is_public(addr.ip()) {
                anyhow::bail!(
                    "Public HTTP requests cannot target internal address {} resolved from {}",
                    addr.ip(),
                    host
                );
            }
        }
        if !resolved_any {
            anyhow::bail!("Unable to resolve host '{}'", host);
        }

        Ok(parsed)
    }

    pub(in crate::runtime) async fn resolve_upload_for_sandbox(
        &self,
        upload_id: &str,
    ) -> Result<SandboxUploadFile> {
        let normalized_id = uuid::Uuid::parse_str(upload_id.trim())
            .map_err(|_| anyhow::anyhow!("Invalid upload ID '{}'", upload_id))?
            .to_string();
        let storage = self
            .storage
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Upload-backed code execution requires storage"))?;
        let manifest = storage
            .load_upload_manifest(&normalized_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Upload '{}' was not found", normalized_id))?;
        let uploads_dir = managed_uploads_dir(self.data_dir());
        let uploads_root = tokio::fs::canonicalize(&uploads_dir)
            .await
            .with_context(|| {
                format!(
                    "Upload directory '{}' is not available",
                    uploads_dir.display()
                )
            })?;
        let resolved = tokio::fs::canonicalize(uploads_root.join(&manifest.stored_name))
            .await
            .with_context(|| {
                format!("Upload payload for '{}' is missing on disk", normalized_id)
            })?;
        if !resolved.starts_with(&uploads_root) {
            anyhow::bail!(
                "Upload '{}' resolved outside the managed upload directory",
                normalized_id
            );
        }
        let bytes = tokio::fs::read(&resolved)
            .await
            .with_context(|| format!("Failed to read upload payload '{}'", normalized_id))?;
        let filename: String = manifest.original_name.chars().collect::<String>();
        let filename = Self::sanitize_upload_filename(&filename);
        Ok(SandboxUploadFile {
            filename,
            content_type: manifest.content_type,
            bytes,
        })
    }

    pub(in crate::runtime) async fn sandbox_upload_from_resource(
        &self,
        resource: RuntimeResourceRef,
    ) -> Result<SandboxUploadFile> {
        let path = self.resolve_runtime_resource_path(&resource).await?;
        let bytes = tokio::fs::read(&path)
            .await
            .with_context(|| format!("Failed to read resource '{}'", path.display()))?;
        let filename = path
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_string)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| resource.id.clone());
        Ok(SandboxUploadFile {
            filename: Self::sanitize_upload_filename(&filename),
            content_type: resource
                .mime
                .or_else(|| mime_guess::from_path(&path).first_raw().map(str::to_string)),
            bytes,
        })
    }

    pub(in crate::runtime) fn collect_native_env_overrides(
        arguments: &serde_json::Value,
    ) -> Result<Vec<(String, String)>> {
        let Some(obj) = arguments.get("env").and_then(|v| v.as_object()) else {
            return Ok(Vec::new());
        };

        if obj.len() > MAX_NATIVE_ENV_OVERRIDES {
            anyhow::bail!(
                "Too many environment overrides: {} (max {})",
                obj.len(),
                MAX_NATIVE_ENV_OVERRIDES
            );
        }

        let mut out = Vec::with_capacity(obj.len());
        for (key, value) in obj {
            if key.is_empty()
                || !key
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                || key.chars().next().is_some_and(|ch| ch.is_ascii_digit())
            {
                anyhow::bail!("Invalid environment variable name '{}'", key);
            }

            let upper = key.to_ascii_uppercase();
            let blocked = matches!(
                upper.as_str(),
                "PATH"
                    | "HOME"
                    | "TMPDIR"
                    | "TMP"
                    | "TEMP"
                    | "PWD"
                    | "SHELL"
                    | "ENV"
                    | "BASH_ENV"
                    | "NODE_OPTIONS"
                    | "PYTHONPATH"
                    | "PYTHONHOME"
                    | "RUBYLIB"
                    | "RUBYOPT"
                    | "PERL5OPT"
            ) || upper.starts_with("LD_")
                || upper.starts_with("DYLD_");
            if blocked {
                anyhow::bail!("Environment override '{}' is not allowed", key);
            }

            let string_value = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Environment override '{}' must be a string", key))?
                .to_string();
            if string_value.contains('\0') {
                anyhow::bail!("Environment override '{}' contains a NUL byte", key);
            }
            out.push((key.clone(), string_value));
        }

        Ok(out)
    }
}
