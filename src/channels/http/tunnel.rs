use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) enum TunnelControlCommand {
    Start,
    Stop,
    Status,
}

/// Manages the active remote-access process and discovered URL.
pub(super) struct TunnelState {
    /// Child process handle
    pub(super) process: Option<tokio::process::Child>,
    /// Active provider for the running tunnel
    pub provider: TunnelProviderKind,
    /// Access URL assigned by the active provider
    pub url: Option<String>,
    /// If set, only this deployed app is reachable through the active remote-access link.
    pub selected_app_id: Option<String>,
    /// Whether the active tunnel should serve the AgentArk control plane.
    pub control_plane_enabled: bool,
    /// Whether the tunnel is actively running
    pub active: bool,
    /// Error message if tunnel failed
    pub error: Option<String>,
}

impl TunnelState {
    pub(super) fn new() -> Self {
        Self {
            process: None,
            provider: TunnelProviderKind::Cloudflare,
            url: None,
            selected_app_id: None,
            control_plane_enabled: false,
            active: false,
            error: None,
        }
    }
}

trait TunnelProvider {
    fn kind(&self) -> TunnelProviderKind;
    fn label(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn config_fields(&self) -> Vec<IntegrationConfigField>;
    fn config_help(&self) -> Option<String>;
}

struct CloudflareTunnelProvider;
struct NgrokTunnelProvider;
struct TailscalePrivateTunnelProvider;
struct TailscaleTunnelProvider;
struct BoreTunnelProvider;

impl TunnelProvider for CloudflareTunnelProvider {
    fn kind(&self) -> TunnelProviderKind {
        TunnelProviderKind::Cloudflare
    }

    fn label(&self) -> &'static str {
        "Cloudflare"
    }

    fn description(&self) -> &'static str {
        "Default public HTTPS link using Cloudflare Quick Tunnel. Easy setup, encrypted in transit, not end-to-end encrypted."
    }

    fn config_fields(&self) -> Vec<IntegrationConfigField> {
        vec![]
    }

    fn config_help(&self) -> Option<String> {
        Some(
            "Click Start to get a temporary public HTTPS link. No Cloudflare account or token is required for Quick Tunnel."
                .to_string(),
        )
    }
}

impl TunnelProvider for NgrokTunnelProvider {
    fn kind(&self) -> TunnelProviderKind {
        TunnelProviderKind::Ngrok
    }

    fn label(&self) -> &'static str {
        "ngrok"
    }

    fn description(&self) -> &'static str {
        "Public HTTPS tunnel using the local ngrok agent."
    }

    fn config_fields(&self) -> Vec<IntegrationConfigField> {
        vec![IntegrationConfigField {
            key: "authtoken".to_string(),
            label: "Auth Token".to_string(),
            input_type: "password".to_string(),
            placeholder: Some("ngrok auth token".to_string()),
            required: true,
            options: None,
        }]
    }

    fn config_help(&self) -> Option<String> {
        Some(format!(
            "Save an ngrok auth token, then {} will start an `ngrok http` tunnel for remote access to the local AgentArk app.",
            crate::branding::PRODUCT_NAME
        ))
    }
}

impl TunnelProvider for TailscalePrivateTunnelProvider {
    fn kind(&self) -> TunnelProviderKind {
        TunnelProviderKind::TailscalePrivate
    }

    fn label(&self) -> &'static str {
        "Tailscale Private (WireGuard E2EE)"
    }

    fn description(&self) -> &'static str {
        "Private HTTPS access for devices on your tailnet using `tailscale serve`. End-to-end encrypted with WireGuard."
    }

    fn config_fields(&self) -> Vec<IntegrationConfigField> {
        vec![
            IntegrationConfigField {
                key: "auth_key".to_string(),
                label: "Auth Key".to_string(),
                input_type: "password".to_string(),
                placeholder: Some("Optional auth key for tailscale up".to_string()),
                required: false,
                options: None,
            },
            IntegrationConfigField {
                key: "hostname".to_string(),
                label: "Hostname".to_string(),
                input_type: "text".to_string(),
                placeholder: Some("Optional fixed ts.net hostname".to_string()),
                required: false,
                options: None,
            },
        ]
    }

    fn config_help(&self) -> Option<String> {
        Some(format!(
            "Use this for private end-to-end encrypted access from your own Tailscale devices. Save an auth key if this runtime is not already signed in, and make sure the {} runtime can reach the Tailscale CLI plus a running tailscaled or TS_SOCKET mount.",
            crate::branding::PRODUCT_NAME
        ))
    }
}

impl TunnelProvider for TailscaleTunnelProvider {
    fn kind(&self) -> TunnelProviderKind {
        TunnelProviderKind::TailscaleFunnel
    }

    fn label(&self) -> &'static str {
        "Tailscale Funnel"
    }

    fn description(&self) -> &'static str {
        "Public HTTPS URL on your tailnet domain using `tailscale funnel`."
    }

    fn config_fields(&self) -> Vec<IntegrationConfigField> {
        vec![
            IntegrationConfigField {
                key: "auth_key".to_string(),
                label: "Auth Key".to_string(),
                input_type: "password".to_string(),
                placeholder: Some("Optional auth key for tailscale up".to_string()),
                required: false,
                options: None,
            },
            IntegrationConfigField {
                key: "hostname".to_string(),
                label: "Hostname".to_string(),
                input_type: "text".to_string(),
                placeholder: Some("Optional fixed ts.net hostname".to_string()),
                required: false,
                options: None,
            },
        ]
    }

    fn config_help(&self) -> Option<String> {
        Some(format!(
            "Requires a working Tailscale runtime. Save an auth key if this runtime is not already signed in, and make sure the {} runtime can reach the Tailscale CLI plus a running tailscaled or TS_SOCKET mount before opening Funnel.",
            crate::branding::PRODUCT_NAME
        ))
    }
}

impl TunnelProvider for BoreTunnelProvider {
    fn kind(&self) -> TunnelProviderKind {
        TunnelProviderKind::Bore
    }

    fn label(&self) -> &'static str {
        "Bore"
    }

    fn description(&self) -> &'static str {
        "Simple TCP tunnel using the `bore` CLI."
    }

    fn config_fields(&self) -> Vec<IntegrationConfigField> {
        vec![IntegrationConfigField {
            key: "server".to_string(),
            label: "Server".to_string(),
            input_type: "text".to_string(),
            placeholder: Some("bore.pub".to_string()),
            required: true,
            options: None,
        }]
    }

    fn config_help(&self) -> Option<String> {
        Some(format!(
            "Bore exposes the HTTP service over a raw TCP tunnel. {} will surface the resulting public HTTP base URL.",
            crate::branding::PRODUCT_NAME
        ))
    }
}

fn tunnel_provider_defs() -> Vec<Box<dyn TunnelProvider>> {
    vec![
        Box::new(CloudflareTunnelProvider),
        Box::new(TailscalePrivateTunnelProvider),
        Box::new(NgrokTunnelProvider),
        Box::new(TailscaleTunnelProvider),
        Box::new(BoreTunnelProvider),
    ]
}

//  - - - WhatsApp Bridge State  - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - -

pub(super) fn parse_tunnel_command(message: &str) -> Option<TunnelControlCommand> {
    let normalized = message.trim().to_ascii_lowercase().replace(['_', '-'], " ");
    let text = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return None;
    }
    match text.as_str() {
        "/tunnel start" | "/start tunnel" => Some(TunnelControlCommand::Start),
        "/tunnel stop" | "/stop tunnel" => Some(TunnelControlCommand::Stop),
        "/tunnel" | "/tunnel status" => Some(TunnelControlCommand::Status),
        _ => None,
    }
}

#[derive(Debug, Serialize)]
struct TunnelProviderResponse {
    id: String,
    label: String,
    description: String,
    exposure: String,
    e2ee: bool,
    link_label: String,
    available: bool,
    configured: bool,
    config_fields: Vec<IntegrationConfigField>,
    config_values: HashMap<String, String>,
    stored_secret_fields: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    config_help: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct TunnelProvidersResponse {
    selected_provider: String,
    active: bool,
    active_provider: String,
    exposure: String,
    e2ee: bool,
    link_label: String,
    url: Option<String>,
    selected_app_id: Option<String>,
    control_plane_enabled: bool,
    error: Option<String>,
    providers: Vec<TunnelProviderResponse>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct ConfigureTunnelRequest {
    provider: Option<String>,
    #[serde(default)]
    values: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct TunnelTestRequest {
    provider: Option<String>,
}

fn parse_tunnel_provider_kind(value: &str) -> Option<TunnelProviderKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "cloudflare" => Some(TunnelProviderKind::Cloudflare),
        "ngrok" => Some(TunnelProviderKind::Ngrok),
        "tailscale_private" | "tailscale-private" | "tailscale private" => {
            Some(TunnelProviderKind::TailscalePrivate)
        }
        "tailscale" | "tailscale_funnel" | "tailscale-funnel" => {
            Some(TunnelProviderKind::TailscaleFunnel)
        }
        "bore" => Some(TunnelProviderKind::Bore),
        _ => None,
    }
}

fn tunnel_provider_label(kind: TunnelProviderKind) -> &'static str {
    match kind {
        TunnelProviderKind::Cloudflare => "Cloudflare",
        TunnelProviderKind::Ngrok => "ngrok",
        TunnelProviderKind::TailscalePrivate => "Tailscale Private (WireGuard E2EE)",
        TunnelProviderKind::TailscaleFunnel => "Tailscale Funnel",
        TunnelProviderKind::Bore => "Bore",
    }
}

fn tunnel_provider_exposure(kind: TunnelProviderKind) -> &'static str {
    match kind {
        TunnelProviderKind::TailscalePrivate => "tailnet_private",
        TunnelProviderKind::Cloudflare
        | TunnelProviderKind::Ngrok
        | TunnelProviderKind::TailscaleFunnel
        | TunnelProviderKind::Bore => "public",
    }
}

fn tunnel_provider_is_e2ee(kind: TunnelProviderKind) -> bool {
    matches!(kind, TunnelProviderKind::TailscalePrivate)
}

fn tunnel_provider_link_label(kind: TunnelProviderKind) -> &'static str {
    match kind {
        TunnelProviderKind::TailscalePrivate => "Private Tailnet URL",
        _ => "Public Link",
    }
}

fn tunnel_provider_binary_path(kind: TunnelProviderKind, config: &TunnelConfig) -> &str {
    match kind {
        TunnelProviderKind::Cloudflare => config.cloudflare.binary_path.trim(),
        TunnelProviderKind::Ngrok => config.ngrok.binary_path.trim(),
        TunnelProviderKind::TailscalePrivate => config.tailscale_funnel.binary_path.trim(),
        TunnelProviderKind::TailscaleFunnel => config.tailscale_funnel.binary_path.trim(),
        TunnelProviderKind::Bore => config.bore.binary_path.trim(),
    }
}

fn binary_path_available(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }
    let path = FsPath::new(trimmed);
    if path.components().count() > 1 || path.is_absolute() {
        return path.exists();
    }
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let path_exts: Vec<String> = if cfg!(windows) {
        std::env::var_os("PATHEXT")
            .map(|raw| {
                raw.to_string_lossy()
                    .split(';')
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| value.to_ascii_lowercase())
                    .collect()
            })
            .unwrap_or_else(|| vec![".exe".to_string(), ".cmd".to_string(), ".bat".to_string()])
    } else {
        vec![String::new()]
    };
    std::env::split_paths(&path_var).any(|dir| {
        if cfg!(windows)
            && path_exts
                .iter()
                .any(|ext| dir.join(format!("{}{}", trimmed, ext)).exists())
        {
            return true;
        }
        dir.join(trimmed).exists()
    })
}

fn resolve_cloudflared_binary(config: &TunnelCloudflareConfig) -> Option<String> {
    let configured = config.binary_path.trim();
    let mut candidates: Vec<String> = Vec::new();
    if !configured.is_empty() {
        candidates.push(configured.to_string());
    }
    for candidate in [
        "cloudflared",
        "/usr/local/bin/cloudflared",
        "/usr/bin/cloudflared",
        "/opt/homebrew/bin/cloudflared",
        "/snap/bin/cloudflared",
    ] {
        if !candidates.iter().any(|value| value == candidate) {
            candidates.push(candidate.to_string());
        }
    }
    if cfg!(windows) {
        for candidate in [
            "cloudflared.exe",
            r"C:\Program Files\cloudflared\cloudflared.exe",
            r"C:\Program Files (x86)\cloudflared\cloudflared.exe",
        ] {
            if !candidates.iter().any(|value| value == candidate) {
                candidates.push(candidate.to_string());
            }
        }
    }
    candidates
        .into_iter()
        .find(|candidate| binary_path_available(candidate))
}

fn tailscale_runtime_available(config: &TunnelConfig) -> bool {
    let cli_available = binary_path_available(config.tailscale_funnel.binary_path.trim());
    if !cli_available {
        return false;
    }
    if cfg!(target_os = "linux") {
        binary_path_available("tailscaled") || std::env::var_os("TS_SOCKET").is_some()
    } else {
        true
    }
}

fn tunnel_provider_available(kind: TunnelProviderKind, config: &TunnelConfig) -> bool {
    match kind {
        TunnelProviderKind::Cloudflare => resolve_cloudflared_binary(&config.cloudflare).is_some(),
        TunnelProviderKind::TailscalePrivate | TunnelProviderKind::TailscaleFunnel => {
            tailscale_runtime_available(config)
        }
        _ => binary_path_available(tunnel_provider_binary_path(kind, config)),
    }
}

fn tunnel_provider_configured(kind: TunnelProviderKind, config: &TunnelConfig) -> bool {
    match kind {
        TunnelProviderKind::Cloudflare => true,
        TunnelProviderKind::Ngrok => !config.ngrok.authtoken.trim().is_empty(),
        TunnelProviderKind::TailscalePrivate => true,
        TunnelProviderKind::TailscaleFunnel => true,
        TunnelProviderKind::Bore => !config.bore.server.trim().is_empty(),
    }
}

fn tunnel_provider_config_values(
    kind: TunnelProviderKind,
    config: &TunnelConfig,
) -> (HashMap<String, String>, Vec<String>) {
    let mut values = HashMap::new();
    let mut stored_secrets = Vec::new();
    match kind {
        TunnelProviderKind::Cloudflare => {}
        TunnelProviderKind::Ngrok => {
            if let Some(domain) = config.ngrok.domain.as_deref() {
                if !domain.trim().is_empty() {
                    values.insert("domain".to_string(), domain.to_string());
                }
            }
            if !config.ngrok.authtoken.trim().is_empty() {
                stored_secrets.push("authtoken".to_string());
            }
        }
        TunnelProviderKind::TailscalePrivate => {
            if let Some(hostname) = config.tailscale_funnel.hostname.as_deref() {
                if !hostname.trim().is_empty() {
                    values.insert("hostname".to_string(), hostname.to_string());
                }
            }
            if !config.tailscale_funnel.auth_key.trim().is_empty() {
                stored_secrets.push("auth_key".to_string());
            }
        }
        TunnelProviderKind::TailscaleFunnel => {
            if let Some(hostname) = config.tailscale_funnel.hostname.as_deref() {
                if !hostname.trim().is_empty() {
                    values.insert("hostname".to_string(), hostname.to_string());
                }
            }
            if !config.tailscale_funnel.auth_key.trim().is_empty() {
                stored_secrets.push("auth_key".to_string());
            }
        }
        TunnelProviderKind::Bore => {
            values.insert("server".to_string(), config.bore.server.clone());
        }
    }
    (values, stored_secrets)
}

fn tunnel_provider_summary(
    kind: TunnelProviderKind,
    config: &TunnelConfig,
) -> TunnelProviderResponse {
    let defs = tunnel_provider_defs();
    let provider = defs
        .into_iter()
        .find(|item| item.kind() == kind)
        .expect("provider definition missing");
    let (config_values, stored_secret_fields) = tunnel_provider_config_values(kind, config);
    TunnelProviderResponse {
        id: kind.as_str().to_string(),
        label: provider.label().to_string(),
        description: provider.description().to_string(),
        exposure: tunnel_provider_exposure(kind).to_string(),
        e2ee: tunnel_provider_is_e2ee(kind),
        link_label: tunnel_provider_link_label(kind).to_string(),
        available: tunnel_provider_available(kind, config),
        configured: tunnel_provider_configured(kind, config),
        config_fields: provider.config_fields(),
        config_values,
        stored_secret_fields,
        config_help: provider.config_help(),
    }
}

fn tunnel_providers_response(
    runtime: &TunnelState,
    config: &TunnelConfig,
) -> TunnelProvidersResponse {
    let effective_provider = if runtime.active {
        runtime.provider
    } else {
        config.provider
    };
    TunnelProvidersResponse {
        selected_provider: config.provider.as_str().to_string(),
        active: runtime.active,
        active_provider: effective_provider.as_str().to_string(),
        exposure: tunnel_provider_exposure(effective_provider).to_string(),
        e2ee: tunnel_provider_is_e2ee(effective_provider),
        link_label: tunnel_provider_link_label(effective_provider).to_string(),
        url: runtime.url.clone(),
        selected_app_id: runtime.selected_app_id.clone(),
        control_plane_enabled: runtime.control_plane_enabled,
        error: runtime.error.clone(),
        providers: [
            TunnelProviderKind::Cloudflare,
            TunnelProviderKind::TailscalePrivate,
            TunnelProviderKind::TailscaleFunnel,
            TunnelProviderKind::Ngrok,
            TunnelProviderKind::Bore,
        ]
        .into_iter()
        .map(|kind| tunnel_provider_summary(kind, config))
        .collect(),
    }
}

pub(super) async fn handle_tunnel_control_command(
    state: &AppState,
    cmd: TunnelControlCommand,
) -> Result<String, String> {
    match cmd {
        TunnelControlCommand::Start => {
            let config = load_tunnel_config(state).await;
            tunnel_auth::ensure_control_plane_tunnel_ready(state, config.provider)
                .await
                .map_err(|error| error.message().to_string())?;
            spawn_tunnel(state, None).await?;
            {
                let mut tunnel = state.tunnel.write().await;
                tunnel.selected_app_id = None;
                tunnel.control_plane_enabled = true;
            }
            persist_public_tunnel_state(state, None, None).await;

            let url = wait_for_tunnel_url(state.tunnel.clone(), 12).await;
            let (provider, tunnel_url, tunnel_error) = {
                let tunnel = state.tunnel.read().await;
                (
                    if tunnel.active {
                        tunnel.provider
                    } else {
                        config.provider
                    },
                    tunnel.url.clone(),
                    tunnel.error.clone(),
                )
            };
            if let Some(found) = url.or(tunnel_url) {
                persist_public_tunnel_state(state, Some(&found), None).await;
                Ok(format!(
                    "{} started.\n{}: {}",
                    tunnel_provider_label(provider),
                    tunnel_provider_link_label(provider),
                    found
                ))
            } else if let Some(err) = tunnel_error {
                Err(format!(
                    "{} start failed: {}",
                    tunnel_provider_label(provider),
                    err
                ))
            } else {
                Ok(format!(
                    "{} is starting. URL is pending; try `/tunnel status` in ~10s.",
                    tunnel_provider_label(provider)
                ))
            }
        }
        TunnelControlCommand::Stop => match reset_tunnel_to_infrastructure(state).await {
            Ok(Some(url)) => Ok(format!(
                "Public exposure stopped. Tunnel infrastructure remains ready: {}",
                url
            )),
            Ok(None) => {
                Ok("Public exposure stopped. Tunnel infrastructure is starting.".to_string())
            }
            Err(error) => Err(format!(
                "Public exposure stopped, but tunnel infrastructure could not restart: {}",
                error
            )),
        },
        TunnelControlCommand::Status => {
            let config = load_tunnel_config(state).await;
            let tunnel = state.tunnel.read().await;
            let provider = if tunnel.active {
                tunnel.provider
            } else {
                config.provider
            };
            let available = tunnel_provider_available(provider, &config);
            let configured = tunnel_provider_configured(provider, &config);
            let status = if tunnel.active { "active" } else { "inactive" };
            let mut out = format!(
                "Tunnel status: {} ({})",
                status,
                tunnel_provider_label(provider)
            );
            if let Some(url) = tunnel.url.clone() {
                out.push_str(&format!(
                    "\n{}: {}",
                    tunnel_provider_link_label(provider),
                    url
                ));
            }
            if let Some(err) = tunnel.error.clone() {
                out.push_str(&format!("\nLast error: {}", err));
            }
            if !available {
                out.push_str("\nProvider binary is not available on this runtime.");
            } else if !configured {
                out.push_str("\nProvider settings are incomplete.");
            }
            Ok(out)
        }
    }
}

pub(super) async fn load_tunnel_config(state: &AppState) -> TunnelConfig {
    let agent = state.agent.read().await;
    agent.config.tunnel.clone()
}

fn clean_logged_url(candidate: &str) -> String {
    candidate
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | ')' | ']' | '}' | ',' | ';'))
        .trim_end_matches('/')
        .to_string()
}

fn extract_logged_https_urls(line: &str) -> Vec<String> {
    line.split_whitespace()
        .filter_map(|part| {
            let start = part.find("https://")?;
            let raw = &part[start..];
            let cleaned = clean_logged_url(raw);
            if cleaned.is_empty() {
                return None;
            }
            reqwest::Url::parse(&cleaned).ok().map(|_| cleaned)
        })
        .collect()
}

fn tunnel_https_url_matches_provider(provider: TunnelProviderKind, url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    match provider {
        TunnelProviderKind::Cloudflare => {
            host.ends_with(".trycloudflare.com") || host.ends_with(".cfargotunnel.com")
        }
        TunnelProviderKind::Ngrok => host.contains("ngrok"),
        TunnelProviderKind::TailscalePrivate => host.ends_with(".ts.net"),
        TunnelProviderKind::TailscaleFunnel => host.ends_with(".ts.net"),
        TunnelProviderKind::Bore => true,
    }
}

fn extract_bore_url(line: &str, server: &str) -> Option<String> {
    let host = server.trim();
    if host.is_empty() {
        return None;
    }
    let pattern = format!(r"{}\:(\d{{2,5}})", regex::escape(host));
    let re = Regex::new(&pattern).ok()?;
    let captures = re.captures(line)?;
    let port = captures.get(1)?.as_str();
    Some(format!("http://{}:{}", host, port))
}

fn extract_tunnel_url_from_log(
    provider: TunnelProviderKind,
    line: &str,
    bore_server: Option<&str>,
) -> Option<String> {
    for url in extract_logged_https_urls(line) {
        if tunnel_https_url_matches_provider(provider, &url) {
            return Some(url);
        }
    }
    if matches!(
        provider,
        TunnelProviderKind::TailscalePrivate | TunnelProviderKind::TailscaleFunnel
    ) {
        for token in line.split_whitespace() {
            let cleaned = clean_logged_url(token);
            if cleaned.ends_with(".ts.net") {
                return Some(format!("https://{}", cleaned));
            }
        }
    }
    if provider == TunnelProviderKind::Bore {
        return extract_bore_url(line, bore_server.unwrap_or("bore.pub"));
    }
    None
}

fn line_looks_like_tunnel_error(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if lower.contains("failed to sufficiently increase receive buffer size")
        || lower.contains("udp-buffer-sizes")
    {
        return false;
    }
    (lower.contains("error") || lower.contains("failed") || lower.contains("panic"))
        && !lower.contains("no error")
}

fn spawn_tunnel_output_reader<R>(
    tunnel_arc: Arc<RwLock<TunnelState>>,
    provider: TunnelProviderKind,
    reader: R,
    bore_server: Option<String>,
    stream_label: &'static str,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    crate::spawn_logged!("src/channels/http/tunnel.rs:740", async move {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(
                "{} tunnel {}: {}",
                tunnel_provider_label(provider),
                stream_label,
                line
            );
            if let Some(url) = extract_tunnel_url_from_log(provider, &line, bore_server.as_deref())
            {
                let mut tunnel = tunnel_arc.write().await;
                if tunnel.url.as_deref() != Some(url.as_str()) {
                    tracing::info!("{} tunnel URL: {}", tunnel_provider_label(provider), url);
                    tunnel.url = Some(url);
                    tunnel.error = None;
                }
            }
            if line_looks_like_tunnel_error(&line) {
                let mut tunnel = tunnel_arc.write().await;
                if tunnel.url.is_none() {
                    tunnel.error = Some(line);
                } else {
                    tracing::warn!(
                        "{} tunnel diagnostic after URL discovery: {}",
                        tunnel_provider_label(provider),
                        line
                    );
                }
            }
        }
    });
}

fn set_tunnel_running(
    tunnel: &mut TunnelState,
    child: tokio::process::Child,
    provider: TunnelProviderKind,
) {
    tunnel.process = Some(child);
    tunnel.provider = provider;
    tunnel.active = true;
    tunnel.error = None;
    tunnel.url = None;
}

async fn spawn_ngrok_url_probe(tunnel_arc: Arc<RwLock<TunnelState>>) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap_or_default();
    for _ in 0..15 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let active = { tunnel_arc.read().await.active };
        if !active {
            break;
        }
        let Ok(resp) = client.get("http://127.0.0.1:4040/api/tunnels").send().await else {
            continue;
        };
        let Ok(payload) = resp.json::<serde_json::Value>().await else {
            continue;
        };
        let tunnels = payload
            .get("tunnels")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let found = tunnels.iter().find_map(|item| {
            item.get("public_url")
                .and_then(|value| value.as_str())
                .map(clean_logged_url)
                .filter(|value| value.starts_with("https://"))
        });
        if let Some(url) = found {
            let mut tunnel = tunnel_arc.write().await;
            tunnel.url = Some(url);
            tunnel.error = None;
            break;
        }
    }
}

#[derive(Debug, Clone, Default)]
struct TailscaleStatusSnapshot {
    backend_state: Option<String>,
    dns_name: Option<String>,
}

impl TailscaleStatusSnapshot {
    fn is_running(&self) -> bool {
        matches!(self.backend_state.as_deref(), Some("Running"))
    }

    fn display_name(&self) -> String {
        self.dns_name
            .clone()
            .or_else(|| self.backend_state.clone())
            .unwrap_or_else(|| "Tailscale runtime".to_string())
    }
}

fn parse_tailscale_status_snapshot(raw: &str) -> Result<TailscaleStatusSnapshot, String> {
    let payload: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("Invalid tailscale status output: {}", e))?;
    let backend_state = payload
        .get("BackendState")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let dns_name = payload
        .get("Self")
        .and_then(|value| value.get("DNSName"))
        .and_then(|value| value.as_str())
        .map(|value| value.trim_end_matches('.').to_string())
        .filter(|value| !value.trim().is_empty());
    Ok(TailscaleStatusSnapshot {
        backend_state,
        dns_name,
    })
}

fn is_tailscale_binary(binary: &str) -> bool {
    matches!(
        binary
            .rsplit(['/', '\\'])
            .next()
            .map(|value| value.trim().to_ascii_lowercase()),
        Some(name) if name == "tailscale" || name == "tailscale.exe"
    )
}

fn tailscale_socket_override() -> Option<String> {
    std::env::var("TS_SOCKET")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn apply_tailscale_socket_arg(command: &mut tokio::process::Command, binary: &str) {
    if is_tailscale_binary(binary) {
        if let Some(socket) = tailscale_socket_override() {
            command.arg("--socket").arg(socket);
        }
    }
}

async fn read_tailscale_status(
    config: &TunnelTailscaleConfig,
) -> Result<TailscaleStatusSnapshot, String> {
    let raw =
        run_tunnel_test_command(config.binary_path.trim(), &["status", "--json"], &[]).await?;
    parse_tailscale_status_snapshot(&raw)
}

async fn tailscale_up(config: &TunnelTailscaleConfig) -> Result<(), String> {
    let auth_key = config.auth_key.trim();
    if auth_key.is_empty() {
        return Err(
            "Tailscale is installed but not signed in. Save a Tailscale auth key first, then retry."
                .to_string(),
        );
    }

    let mut args = vec![
        "up".to_string(),
        format!("--auth-key={}", auth_key),
        "--accept-dns=false".to_string(),
        "--reset".to_string(),
        "--timeout=60s".to_string(),
    ];
    if let Some(hostname) = config
        .hostname
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.push(format!("--hostname={}", hostname));
    }

    let mut command = tokio::process::Command::new(config.binary_path.trim());
    apply_tailscale_socket_arg(&mut command, config.binary_path.trim());
    command
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = tokio::time::timeout(Duration::from_secs(75), command.output())
        .await
        .map_err(|_| "Timed out while connecting Tailscale runtime.".to_string())?
        .map_err(|e| format!("Failed to run tailscale up: {}", e))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Err(if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("tailscale up exited with {}", output.status)
    })
}

async fn ensure_tailscale_runtime_ready(
    config: &TunnelTailscaleConfig,
) -> Result<TailscaleStatusSnapshot, String> {
    match read_tailscale_status(config).await {
        Ok(snapshot) if snapshot.is_running() => Ok(snapshot),
        Err(error) if error.to_ascii_lowercase().contains("binary not found") => Err(error),
        Ok(_) | Err(_) if config.auth_key.trim().is_empty() => Err(
            "Tailscale runtime is not connected. Save a Tailscale auth key in Settings, then retry."
                .to_string(),
        ),
        Ok(_) | Err(_) => {
            tailscale_up(config).await?;
            let snapshot = read_tailscale_status(config).await?;
            if snapshot.is_running() {
                Ok(snapshot)
            } else {
                Err(format!(
                    "Tailscale started but is not ready yet (state: {}).",
                    snapshot
                        .backend_state
                        .as_deref()
                        .unwrap_or("unknown")
                ))
            }
        }
    }
}

fn tailscale_status_args(provider: TunnelProviderKind) -> Option<[&'static str; 2]> {
    match provider {
        TunnelProviderKind::TailscalePrivate => Some(["serve", "status"]),
        TunnelProviderKind::TailscaleFunnel => Some(["funnel", "status"]),
        _ => None,
    }
}

fn tailscale_reset_args(provider: TunnelProviderKind) -> Option<[&'static str; 2]> {
    match provider {
        TunnelProviderKind::TailscalePrivate => Some(["serve", "reset"]),
        TunnelProviderKind::TailscaleFunnel => Some(["funnel", "reset"]),
        _ => None,
    }
}

async fn spawn_tailscale_url_probe(
    tunnel_arc: Arc<RwLock<TunnelState>>,
    provider: TunnelProviderKind,
    binary: String,
    auth_key: Option<String>,
) {
    let Some(args) = tailscale_status_args(provider) else {
        return;
    };
    for _ in 0..15 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let should_continue = {
            let tunnel = tunnel_arc.read().await;
            tunnel.active && tunnel.provider == provider && tunnel.url.is_none()
        };
        if !should_continue {
            break;
        }

        let mut command = tokio::process::Command::new(&binary);
        apply_tailscale_socket_arg(&mut command, &binary);
        command
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(value) = auth_key.as_deref().filter(|value| !value.trim().is_empty()) {
            command.env("TS_AUTHKEY", value.trim());
        }

        let Ok(output) = tokio::time::timeout(Duration::from_secs(5), command.output()).await
        else {
            continue;
        };
        let Ok(output) = output else {
            continue;
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let found = stdout
            .lines()
            .chain(stderr.lines())
            .find_map(|line| extract_tunnel_url_from_log(provider, line, None));
        if let Some(url) = found {
            let mut tunnel = tunnel_arc.write().await;
            if tunnel.active && tunnel.provider == provider {
                tunnel.url = Some(url);
                tunnel.error = None;
            }
            break;
        }
    }
}

async fn reset_tailscale_provider(provider: TunnelProviderKind, config: &TunnelConfig) {
    let Some(args) = tailscale_reset_args(provider) else {
        return;
    };
    let binary = config.tailscale_funnel.binary_path.trim();
    if !binary_path_available(binary) {
        tracing::warn!(
            "Skipping {} reset because binary is unavailable: {}",
            tunnel_provider_label(provider),
            binary
        );
        return;
    }
    let mut command = tokio::process::Command::new(binary);
    apply_tailscale_socket_arg(&mut command, binary);
    command
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    if !config.tailscale_funnel.auth_key.trim().is_empty() {
        command.env("TS_AUTHKEY", config.tailscale_funnel.auth_key.trim());
    }

    match tokio::time::timeout(Duration::from_secs(8), command.output()).await {
        Ok(Ok(output)) if output.status.success() => {}
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            tracing::warn!(
                "Failed to reset {}: {}",
                tunnel_provider_label(provider),
                if stderr.is_empty() {
                    format!("exit {}", output.status)
                } else {
                    stderr
                }
            );
        }
        Ok(Err(error)) => {
            tracing::warn!(
                "Failed to reset {}: {}",
                tunnel_provider_label(provider),
                error
            );
        }
        Err(_) => {
            tracing::warn!("Timed out resetting {}", tunnel_provider_label(provider));
        }
    }
}

pub(super) async fn persist_public_tunnel_state(
    state: &AppState,
    url: Option<&str>,
    selected_app_id: Option<&str>,
) {
    let agent = state.agent.read().await;
    match url {
        Some(value) if !value.trim().is_empty() => {
            let _ = agent
                .storage
                .set("public_base_url", value.trim().as_bytes())
                .await;
        }
        _ => {
            let _ = agent.storage.delete("public_base_url").await;
        }
    }
    match selected_app_id {
        Some(value) if !value.trim().is_empty() => {
            let _ = agent
                .storage
                .set(PUBLIC_SELECTED_APP_KEY, value.trim().as_bytes())
                .await;
        }
        _ => {
            let _ = agent.storage.delete(PUBLIC_SELECTED_APP_KEY).await;
        }
    }
}

pub(super) async fn load_public_selected_app_id(state: &AppState) -> Option<String> {
    let agent = state.agent.read().await;
    agent
        .storage
        .get(PUBLIC_SELECTED_APP_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn selected_public_app_is_ready(state: &AppState, app_id: &str) -> Result<bool, String> {
    if !is_valid_app_id(app_id) {
        return Err("Saved public app selection is not a valid app id.".to_string());
    }
    if state.app_registry.get_dir(app_id).await.is_none() {
        return Ok(false);
    }
    if !state.app_registry.access_guard_enabled(app_id).await {
        return Err(
            "Saved public app selection no longer has App Guard enabled; not exposing it publicly."
                .to_string(),
        );
    }
    if state
        .app_registry
        .access_key(app_id)
        .await
        .map(|value| value.trim().is_empty())
        .unwrap_or(true)
    {
        return Err(
            "Saved public app selection has no access password; not exposing it publicly."
                .to_string(),
        );
    }
    Ok(true)
}

pub(super) async fn auto_start_selected_app_tunnel(
    state: &AppState,
) -> Result<Option<String>, String> {
    let Some(app_id) = load_public_selected_app_id(state).await else {
        return Ok(None);
    };

    persist_public_tunnel_state(state, None, Some(&app_id)).await;
    for attempt in 0..30 {
        match selected_public_app_is_ready(state, &app_id).await {
            Ok(true) => break,
            Ok(false) if attempt < 29 => {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            Ok(false) => {
                return Err(format!(
                    "Saved public app {} was not restored in time; tunnel auto-start skipped.",
                    app_id
                ));
            }
            Err(error) => {
                persist_public_tunnel_state(state, None, None).await;
                return Err(error);
            }
        }
    }

    spawn_tunnel(state, None).await?;
    {
        let mut tunnel = state.tunnel.write().await;
        tunnel.selected_app_id = Some(app_id.clone());
        tunnel.control_plane_enabled = false;
    }
    let url = wait_for_tunnel_url(state.tunnel.clone(), 12).await;
    if let Some(found) = url.as_deref() {
        persist_public_tunnel_state(state, Some(found), Some(&app_id)).await;
    } else {
        persist_public_tunnel_state(state, None, Some(&app_id)).await;
    }
    Ok(url)
}

pub(super) async fn auto_start_tunnel_infrastructure(
    state: &AppState,
) -> Result<Option<String>, String> {
    {
        let tunnel = state.tunnel.read().await;
        if tunnel.active {
            return Ok(tunnel.url.clone());
        }
    }

    spawn_tunnel(state, None).await?;
    {
        let mut tunnel = state.tunnel.write().await;
        tunnel.selected_app_id = None;
        tunnel.control_plane_enabled = false;
    }
    let url = wait_for_tunnel_url(state.tunnel.clone(), 12).await;
    Ok(url)
}

async fn run_tunnel_test_command(
    binary: &str,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> Result<String, String> {
    if !binary_path_available(binary) {
        return Err(format!("Binary not found: {}", binary));
    }
    let mut cmd = tokio::process::Command::new(binary);
    apply_tailscale_socket_arg(&mut cmd, binary);
    cmd.args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (key, value) in extra_env {
        if !value.trim().is_empty() {
            cmd.env(key, value);
        }
    }
    let output = tokio::time::timeout(Duration::from_secs(8), cmd.output())
        .await
        .map_err(|_| format!("Timed out running {}", binary))?
        .map_err(|e| format!("Failed to run {}: {}", binary, e))?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stdout.is_empty() {
            Ok(format!("{} is available.", binary))
        } else {
            Ok(stdout)
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("{} exited with {}", binary, output.status)
        } else {
            stderr
        })
    }
}

#[derive(Debug, Serialize, Clone)]
struct TunnelSetupCheck {
    id: String,
    label: String,
    status: String,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    remediation: Option<String>,
}

#[derive(Debug)]
struct TunnelTestReport {
    message: String,
    detail: String,
    checks: Vec<TunnelSetupCheck>,
}

#[derive(Debug)]
struct TunnelTestError {
    message: String,
    checks: Vec<TunnelSetupCheck>,
}

fn make_tunnel_setup_check(
    id: &str,
    label: &str,
    status: &str,
    detail: impl Into<String>,
    remediation: Option<&str>,
) -> TunnelSetupCheck {
    TunnelSetupCheck {
        id: id.to_string(),
        label: label.to_string(),
        status: status.to_string(),
        detail: detail.into(),
        remediation: remediation.map(|value| value.to_string()),
    }
}

async fn test_tailscale_provider_connection(
    kind: TunnelProviderKind,
    config: &TunnelConfig,
) -> Result<TunnelTestReport, TunnelTestError> {
    let label = tunnel_provider_label(kind);
    let runtime = &config.tailscale_funnel;
    let binary = runtime.binary_path.trim();
    let mut checks = Vec::new();

    if !binary_path_available(binary) {
        checks.push(make_tunnel_setup_check(
            "tailscale_cli",
            "Tailscale CLI",
            "fail",
            format!(
                "{} could not find `{}` on this runtime.",
                crate::branding::PRODUCT_NAME,
                binary
            ),
            Some(&format!(
                "Install Tailscale in the {} runtime first.",
                crate::branding::PRODUCT_NAME
            )),
        ));
        return Err(TunnelTestError {
            message: format!(
                "{} is not available on this {} runtime.",
                label,
                crate::branding::PRODUCT_NAME
            ),
            checks,
        });
    }
    checks.push(make_tunnel_setup_check(
        "tailscale_cli",
        "Tailscale CLI",
        "pass",
        format!(
            "{} can run `{}` from this runtime.",
            crate::branding::PRODUCT_NAME,
            binary
        ),
        None,
    ));

    if cfg!(target_os = "linux") {
        let socket_override = tailscale_socket_override();
        let daemon_ready = socket_override.is_some() || binary_path_available("tailscaled");
        if !daemon_ready {
            checks.push(make_tunnel_setup_check(
                "tailscale_runtime_socket",
                "tailscaled / TS_SOCKET",
                "fail",
                "No running tailscaled or TS_SOCKET mount was detected for this Linux runtime.",
                Some("In Docker, run tailscaled in the container or mount TS_SOCKET into the container."),
            ));
            return Err(TunnelTestError {
                message: format!("{} is missing its Tailscale runtime daemon.", label),
                checks,
            });
        }
        let runtime_message = if socket_override.is_some() {
            format!(
                "{} found a TS_SOCKET override for the Tailscale daemon.",
                crate::branding::PRODUCT_NAME
            )
        } else {
            format!(
                "{} found a local tailscaled runtime.",
                crate::branding::PRODUCT_NAME
            )
        };
        checks.push(make_tunnel_setup_check(
            "tailscale_runtime_socket",
            "tailscaled / TS_SOCKET",
            "pass",
            runtime_message,
            None,
        ));
    }

    let mut snapshot = match read_tailscale_status(runtime).await {
        Ok(status) if status.is_running() => {
            checks.push(make_tunnel_setup_check(
                "tailscale_signed_in",
                "Signed-in runtime",
                "pass",
                format!("Tailscale is already running as {}.", status.display_name()),
                None,
            ));
            status
        }
        Ok(status) => {
            checks.push(make_tunnel_setup_check(
                "tailscale_signed_in",
                "Signed-in runtime",
                "warn",
                format!(
                    "Tailscale is installed, but the runtime state is {}.",
                    status.backend_state.as_deref().unwrap_or("unknown")
                ),
                Some(&format!(
                    "{} can sign this runtime in if you save an auth key.",
                    crate::branding::PRODUCT_NAME
                )),
            ));
            status
        }
        Err(error) => {
            checks.push(make_tunnel_setup_check(
                "tailscale_signed_in",
                "Signed-in runtime",
                "warn",
                format!(
                    "{} could not confirm a running Tailscale session yet: {}",
                    crate::branding::PRODUCT_NAME,
                    error
                ),
                Some("If this runtime is not already signed in, save an auth key and run Check setup again."),
            ));
            TailscaleStatusSnapshot::default()
        }
    };

    let auth_key_saved = !runtime.auth_key.trim().is_empty();
    if snapshot.is_running() {
        checks.push(make_tunnel_setup_check(
            "tailscale_auth_key",
            "Auth key",
            if auth_key_saved { "pass" } else { "info" },
            if auth_key_saved {
                "A Tailscale auth key is saved for re-authentication if needed."
            } else {
                "No auth key is saved, but this runtime is already signed in."
            },
            if auth_key_saved {
                None
            } else {
                Some("You can leave auth key blank as long as this runtime stays signed in.")
            },
        ));
    } else if !auth_key_saved {
        checks.push(make_tunnel_setup_check(
            "tailscale_auth_key",
            "Auth key",
            "fail",
            "No Tailscale auth key is saved, and this runtime is not signed in yet.",
            Some(&format!(
                "Save a Tailscale auth key so {} can run `tailscale up` in Docker.",
                crate::branding::PRODUCT_NAME
            )),
        ));
        return Err(TunnelTestError {
            message: format!("{} still needs a Tailscale auth key.", label),
            checks,
        });
    } else {
        checks.push(make_tunnel_setup_check(
            "tailscale_auth_key",
            "Auth key",
            "pass",
            "A Tailscale auth key is saved and can be used to sign this runtime in.",
            None,
        ));
        match tailscale_up(runtime).await {
            Ok(()) => match read_tailscale_status(runtime).await {
                Ok(status) if status.is_running() => {
                    checks.push(make_tunnel_setup_check(
                        "tailscale_connect",
                        "Runtime sign-in",
                        "pass",
                        format!(
                            "{} signed the runtime into Tailscale as {}.",
                            crate::branding::PRODUCT_NAME,
                            status.display_name()
                        ),
                        None,
                    ));
                    snapshot = status;
                }
                Ok(status) => {
                    checks.push(make_tunnel_setup_check(
                        "tailscale_connect",
                        "Runtime sign-in",
                        "fail",
                        format!(
                            "{} ran `tailscale up`, but the runtime state is still {}.",
                            crate::branding::PRODUCT_NAME,
                            status.backend_state.as_deref().unwrap_or("unknown")
                        ),
                        Some("Wait a few seconds and run Check setup again."),
                    ));
                    return Err(TunnelTestError {
                        message: format!("{} did not finish connecting to Tailscale.", label),
                        checks,
                    });
                }
                Err(error) => {
                    checks.push(make_tunnel_setup_check(
                        "tailscale_connect",
                        "Runtime sign-in",
                        "fail",
                        error.clone(),
                        Some("Check the auth key and the container's access to Tailscale, then retry."),
                    ));
                    return Err(TunnelTestError {
                        message: format!("{} could not finish Tailscale sign-in.", label),
                        checks,
                    });
                }
            },
            Err(error) => {
                checks.push(make_tunnel_setup_check(
                    "tailscale_connect",
                    "Runtime sign-in",
                    "fail",
                    error.clone(),
                    Some("Check the auth key and the container's access to Tailscale, then retry."),
                ));
                return Err(TunnelTestError {
                    message: format!("{} could not start the Tailscale runtime.", label),
                    checks,
                });
            }
        }
    }

    let status_args: [&str; 2] = if kind == TunnelProviderKind::TailscalePrivate {
        ["serve", "status"]
    } else {
        ["funnel", "status"]
    };
    match run_tunnel_test_command(
        binary,
        &status_args,
        &[("TS_AUTHKEY", runtime.auth_key.trim())],
    )
    .await
    {
        Ok(detail) => {
            checks.push(make_tunnel_setup_check(
                if kind == TunnelProviderKind::TailscalePrivate {
                    "tailscale_serve"
                } else {
                    "tailscale_funnel"
                },
                if kind == TunnelProviderKind::TailscalePrivate {
                    "Serve command"
                } else {
                    "Funnel command"
                },
                "pass",
                if detail.trim().is_empty() {
                    "Tailscale accepted the status check for this access mode.".to_string()
                } else {
                    detail
                },
                None,
            ));
        }
        Err(error) => {
            let lower = error.to_ascii_lowercase();
            let consent_related = lower.contains("https")
                || lower.contains("serve")
                || lower.contains("funnel")
                || lower.contains("consent")
                || lower.contains("enable");
            if consent_related {
                checks.push(make_tunnel_setup_check(
                    if kind == TunnelProviderKind::TailscalePrivate {
                        "tailscale_serve"
                    } else {
                        "tailscale_funnel"
                    },
                    if kind == TunnelProviderKind::TailscalePrivate {
                        "Serve consent"
                    } else {
                        "Funnel consent"
                    },
                    "fail",
                    error.clone(),
                    Some("Enable tailnet HTTPS / serve approval in Tailscale, then retry."),
                ));
                return Err(TunnelTestError {
                    message: format!(
                        "{} still needs a Tailscale HTTPS / serve approval step.",
                        label
                    ),
                    checks,
                });
            }
            checks.push(make_tunnel_setup_check(
                if kind == TunnelProviderKind::TailscalePrivate {
                    "tailscale_serve"
                } else {
                    "tailscale_funnel"
                },
                if kind == TunnelProviderKind::TailscalePrivate {
                    "Serve command"
                } else {
                    "Funnel command"
                },
                "info",
                format!("The management command did not return a clean status yet: {}", error),
                Some(&format!(
                    "Try Start next. If Tailscale still blocks startup, {} will surface the exact runtime error.",
                    crate::branding::PRODUCT_NAME
                )),
            ));
        }
    }

    Ok(TunnelTestReport {
        message: format!("{} setup looks ready.", label),
        detail: format!(
            "Tailscale runtime is connected as {}.",
            snapshot.display_name()
        ),
        checks,
    })
}

async fn test_tunnel_provider_connection(
    kind: TunnelProviderKind,
    config: &TunnelConfig,
) -> Result<TunnelTestReport, TunnelTestError> {
    match kind {
        TunnelProviderKind::Cloudflare => {
            let Some(binary) = resolve_cloudflared_binary(&config.cloudflare) else {
                return Err(TunnelTestError {
                    message: "Cloudflare tunnel is not available on this server.".to_string(),
                    checks: vec![make_tunnel_setup_check(
                        "cloudflared_binary",
                        "cloudflared",
                        "fail",
                        format!(
                            "{} could not find cloudflared on this runtime.",
                            crate::branding::PRODUCT_NAME
                        ),
                        Some(&format!(
                            "Install cloudflared on the {} runtime first.",
                            crate::branding::PRODUCT_NAME
                        )),
                    )],
                });
            };
            match run_tunnel_test_command(binary.trim(), &["--version"], &[]).await {
                Ok(detail) => Ok(TunnelTestReport {
                    message: "Cloudflare setup looks ready.".to_string(),
                    detail: detail.clone(),
                    checks: vec![make_tunnel_setup_check(
                        "cloudflared_binary",
                        "cloudflared",
                        "pass",
                        detail,
                        None,
                    )],
                }),
                Err(error) => Err(TunnelTestError {
                    message: "Cloudflare setup is not ready yet.".to_string(),
                    checks: vec![make_tunnel_setup_check(
                        "cloudflared_binary",
                        "cloudflared",
                        "fail",
                        error,
                        Some(&format!(
                            "Install cloudflared on the {} runtime first.",
                            crate::branding::PRODUCT_NAME
                        )),
                    )],
                }),
            }
        }
        TunnelProviderKind::Ngrok => {
            if config.ngrok.authtoken.trim().is_empty() {
                return Err(TunnelTestError {
                    message: "ngrok still needs its auth token.".to_string(),
                    checks: vec![make_tunnel_setup_check(
                        "ngrok_auth",
                        "Auth token",
                        "fail",
                        "No ngrok auth token is saved yet.",
                        Some("Save an ngrok auth token before testing or starting remote access."),
                    )],
                });
            }
            match run_tunnel_test_command(
                config.ngrok.binary_path.trim(),
                &["version"],
                &[("NGROK_AUTHTOKEN", config.ngrok.authtoken.trim())],
            )
            .await
            {
                Ok(detail) => Ok(TunnelTestReport {
                    message: "ngrok setup looks ready.".to_string(),
                    detail: detail.clone(),
                    checks: vec![
                        make_tunnel_setup_check(
                            "ngrok_auth",
                            "Auth token",
                            "pass",
                            "An ngrok auth token is saved.",
                            None,
                        ),
                        make_tunnel_setup_check("ngrok_binary", "ngrok CLI", "pass", detail, None),
                    ],
                }),
                Err(error) => Err(TunnelTestError {
                    message: "ngrok setup is not ready yet.".to_string(),
                    checks: vec![make_tunnel_setup_check(
                        "ngrok_binary",
                        "ngrok CLI",
                        "fail",
                        error,
                        Some(&format!(
                            "Install ngrok on the {} runtime first.",
                            crate::branding::PRODUCT_NAME
                        )),
                    )],
                }),
            }
        }
        TunnelProviderKind::TailscalePrivate | TunnelProviderKind::TailscaleFunnel => {
            test_tailscale_provider_connection(kind, config).await
        }
        TunnelProviderKind::Bore => {
            if config.bore.server.trim().is_empty() {
                return Err(TunnelTestError {
                    message: "Bore still needs its relay server.".to_string(),
                    checks: vec![make_tunnel_setup_check(
                        "bore_server",
                        "Relay server",
                        "fail",
                        "No Bore relay server is configured yet.",
                        Some(
                            "Save the Bore relay server before testing or starting remote access.",
                        ),
                    )],
                });
            }
            match run_tunnel_test_command(config.bore.binary_path.trim(), &["--help"], &[]).await {
                Ok(detail) => Ok(TunnelTestReport {
                    message: "Bore setup looks ready.".to_string(),
                    detail: detail.clone(),
                    checks: vec![
                        make_tunnel_setup_check(
                            "bore_server",
                            "Relay server",
                            "pass",
                            format!("Bore is configured to use {}.", config.bore.server.trim()),
                            None,
                        ),
                        make_tunnel_setup_check("bore_binary", "Bore CLI", "pass", detail, None),
                    ],
                }),
                Err(error) => Err(TunnelTestError {
                    message: "Bore setup is not ready yet.".to_string(),
                    checks: vec![make_tunnel_setup_check(
                        "bore_binary",
                        "Bore CLI",
                        "fail",
                        error,
                        Some(&format!(
                            "Install Bore on the {} runtime first.",
                            crate::branding::PRODUCT_NAME
                        )),
                    )],
                }),
            }
        }
    }
}

pub(super) async fn get_tunnel_providers(
    State(state): State<AppState>,
) -> Json<TunnelProvidersResponse> {
    let config = load_tunnel_config(&state).await;
    let tunnel = state.tunnel.read().await;
    Json(tunnel_providers_response(&tunnel, &config))
}

pub(super) async fn configure_tunnel(
    State(state): State<AppState>,
    Json(request): Json<ConfigureTunnelRequest>,
) -> Response {
    let save_result = {
        let mut agent = state.agent.write().await;
        let mut next = agent.config.tunnel.clone();
        if let Some(provider_raw) = request.provider.as_deref() {
            let Some(provider) = parse_tunnel_provider_kind(provider_raw) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "Unknown tunnel provider" })),
                )
                    .into_response();
            };
            next.provider = provider;
        }

        let values = request.values;
        if let Some(value) = values
            .get("binary_path")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            match next.provider {
                TunnelProviderKind::Cloudflare => next.cloudflare.binary_path = value.to_string(),
                TunnelProviderKind::Ngrok => next.ngrok.binary_path = value.to_string(),
                TunnelProviderKind::TailscalePrivate => {
                    next.tailscale_funnel.binary_path = value.to_string()
                }
                TunnelProviderKind::TailscaleFunnel => {
                    next.tailscale_funnel.binary_path = value.to_string()
                }
                TunnelProviderKind::Bore => next.bore.binary_path = value.to_string(),
            }
        }
        match next.provider {
            TunnelProviderKind::Cloudflare => {}
            TunnelProviderKind::Ngrok => {
                if let Some(value) = values.get("authtoken") {
                    if !value.trim().is_empty() {
                        next.ngrok.authtoken = value.trim().to_string();
                    }
                }
            }
            TunnelProviderKind::TailscalePrivate => {
                if let Some(value) = values.get("auth_key") {
                    if !value.trim().is_empty() {
                        next.tailscale_funnel.auth_key = value.trim().to_string();
                    }
                }
                if let Some(value) = values.get("hostname") {
                    next.tailscale_funnel.hostname = if value.trim().is_empty() {
                        None
                    } else {
                        Some(value.trim().to_string())
                    };
                }
            }
            TunnelProviderKind::TailscaleFunnel => {
                if let Some(value) = values.get("auth_key") {
                    if !value.trim().is_empty() {
                        next.tailscale_funnel.auth_key = value.trim().to_string();
                    }
                }
                if let Some(value) = values.get("hostname") {
                    next.tailscale_funnel.hostname = if value.trim().is_empty() {
                        None
                    } else {
                        Some(value.trim().to_string())
                    };
                }
            }
            TunnelProviderKind::Bore => {
                if let Some(value) = values.get("server") {
                    if value.trim().is_empty() {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({ "error": "Bore server cannot be empty" })),
                        )
                            .into_response();
                    }
                    next.bore.server = value.trim().to_string();
                }
            }
        }

        agent.config.tunnel = next.clone();
        let result = agent.config.save(&agent.config_dir, Some(&agent.data_dir));
        (result, next)
    };

    let (result, config) = save_result;
    if let Err(error) = result {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to save tunnel settings: {}", error)
            })),
        )
            .into_response();
    }

    {
        let mut tunnel = state.tunnel.write().await;
        if tunnel.active {
            tunnel.error = Some(
                "Tunnel configuration changed. Restart the tunnel to apply the new provider settings."
                    .to_string(),
            );
        }
    }
    let tunnel = state.tunnel.read().await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": true,
            "message": "Tunnel settings saved.",
            "settings": tunnel_providers_response(&tunnel, &config),
        })),
    )
        .into_response()
}

pub(super) async fn test_tunnel_connection(
    State(state): State<AppState>,
    Json(request): Json<TunnelTestRequest>,
) -> Response {
    let config = load_tunnel_config(&state).await;
    let provider = request
        .provider
        .as_deref()
        .and_then(parse_tunnel_provider_kind)
        .unwrap_or(config.provider);
    match test_tunnel_provider_connection(provider, &config).await {
        Ok(report) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "provider": provider.as_str(),
                "message": report.message,
                "detail": report.detail,
                "checks": report.checks,
            })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": false,
                "provider": provider.as_str(),
                "message": error.message,
                "error": error.message,
                "checks": error.checks,
            })),
        )
            .into_response(),
    }
}

pub(super) async fn get_tunnel_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let config = load_tunnel_config(&state).await;
    let tunnel = state.tunnel.read().await;
    let provider = if tunnel.active {
        tunnel.provider
    } else {
        config.provider
    };
    Json(serde_json::json!({
        "active": tunnel.active,
        "url": tunnel.url,
        "selected_app_id": tunnel.selected_app_id,
        "control_plane_enabled": tunnel.control_plane_enabled,
        "error": tunnel.error,
        "provider": provider.as_str(),
        "provider_label": tunnel_provider_label(provider),
        "exposure": tunnel_provider_exposure(provider),
        "e2ee": tunnel_provider_is_e2ee(provider),
        "link_label": tunnel_provider_link_label(provider),
        "available": tunnel_provider_available(provider, &config),
        "configured": tunnel_provider_configured(provider, &config)
    }))
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct StartTunnelRequest {
    #[serde(default)]
    app_id: Option<String>,
    #[serde(default)]
    provider: Option<String>,
}

/// POST /tunnel/start - start the selected public tunnel provider
/// Core tunnel start logic  - used by both the API endpoint and auto-start
async fn spawn_cloudflare_tunnel(
    tunnel_arc: Arc<RwLock<TunnelState>>,
    config: &TunnelCloudflareConfig,
) -> Result<(), String> {
    let Some(binary) = resolve_cloudflared_binary(config) else {
        return Err(
            "cloudflared was not found on this server. Install cloudflared or set a custom binary path in advanced tunnel settings."
                .to_string(),
        );
    };
    match tokio::process::Command::new(&binary)
        .args([
            "tunnel",
            "--no-autoupdate",
            "--url",
            "http://127.0.0.1:8990",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(mut child) => {
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            {
                let mut tunnel = tunnel_arc.write().await;
                set_tunnel_running(&mut tunnel, child, TunnelProviderKind::Cloudflare);
            }
            if let Some(stdout) = stdout {
                spawn_tunnel_output_reader(
                    tunnel_arc.clone(),
                    TunnelProviderKind::Cloudflare,
                    stdout,
                    None,
                    "stdout",
                );
            }
            if let Some(stderr) = stderr {
                spawn_tunnel_output_reader(
                    tunnel_arc,
                    TunnelProviderKind::Cloudflare,
                    stderr,
                    None,
                    "stderr",
                );
            }
            Ok(())
        }
        Err(e) => Err(format!("Failed to start Cloudflare tunnel: {}", e)),
    }
}

async fn spawn_ngrok_tunnel(
    tunnel_arc: Arc<RwLock<TunnelState>>,
    config: &TunnelNgrokConfig,
) -> Result<(), String> {
    let binary = config.binary_path.trim();
    if !binary_path_available(binary) {
        return Err(format!("ngrok binary not found: {}", binary));
    }
    if config.authtoken.trim().is_empty() {
        return Err("ngrok auth token is required before starting the tunnel.".to_string());
    }
    let mut command = tokio::process::Command::new(binary);
    command
        .args([
            "http",
            "http://127.0.0.1:8990",
            "--log=stdout",
            "--log-format=json",
        ])
        .env("NGROK_AUTHTOKEN", config.authtoken.trim())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    match command.spawn() {
        Ok(mut child) => {
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            {
                let mut tunnel = tunnel_arc.write().await;
                set_tunnel_running(&mut tunnel, child, TunnelProviderKind::Ngrok);
            }
            if let Some(stdout) = stdout {
                spawn_tunnel_output_reader(
                    tunnel_arc.clone(),
                    TunnelProviderKind::Ngrok,
                    stdout,
                    None,
                    "stdout",
                );
            }
            if let Some(stderr) = stderr {
                spawn_tunnel_output_reader(
                    tunnel_arc.clone(),
                    TunnelProviderKind::Ngrok,
                    stderr,
                    None,
                    "stderr",
                );
            }
            crate::spawn_logged!(
                "src/channels/http/tunnel.rs:1971",
                spawn_ngrok_url_probe(tunnel_arc)
            );
            Ok(())
        }
        Err(e) => Err(format!("Failed to start ngrok tunnel: {}", e)),
    }
}

async fn spawn_tailscale_tunnel(
    tunnel_arc: Arc<RwLock<TunnelState>>,
    config: &TunnelTailscaleConfig,
) -> Result<(), String> {
    let binary = config.binary_path.trim();
    if !binary_path_available(binary) {
        return Err(format!("Tailscale binary not found: {}", binary));
    }
    let snapshot = ensure_tailscale_runtime_ready(config).await?;
    let mut command = tokio::process::Command::new(binary);
    apply_tailscale_socket_arg(&mut command, binary);
    command
        .arg("funnel")
        .arg("--yes")
        .arg("8990")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if !config.auth_key.trim().is_empty() {
        command.env("TS_AUTHKEY", config.auth_key.trim());
    }
    if let Some(hostname) = config
        .hostname
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        command.env("TS_HOSTNAME", hostname.trim());
    }
    match command.spawn() {
        Ok(mut child) => {
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            {
                let mut tunnel = tunnel_arc.write().await;
                set_tunnel_running(&mut tunnel, child, TunnelProviderKind::TailscaleFunnel);
                if let Some(hostname) = snapshot.dns_name.as_deref() {
                    tunnel.url = Some(format!("https://{}", hostname));
                }
            }
            if let Some(stdout) = stdout {
                spawn_tunnel_output_reader(
                    tunnel_arc.clone(),
                    TunnelProviderKind::TailscaleFunnel,
                    stdout,
                    None,
                    "stdout",
                );
            }
            if let Some(stderr) = stderr {
                spawn_tunnel_output_reader(
                    tunnel_arc.clone(),
                    TunnelProviderKind::TailscaleFunnel,
                    stderr,
                    None,
                    "stderr",
                );
            }
            crate::spawn_logged!(
                "src/channels/http/tunnel.rs:2035",
                spawn_tailscale_url_probe(
                    tunnel_arc,
                    TunnelProviderKind::TailscaleFunnel,
                    binary.to_string(),
                    (!config.auth_key.trim().is_empty())
                        .then(|| config.auth_key.trim().to_string()),
                )
            );
            Ok(())
        }
        Err(e) => Err(format!("Failed to start Tailscale Funnel: {}", e)),
    }
}

async fn spawn_tailscale_private_access(
    tunnel_arc: Arc<RwLock<TunnelState>>,
    config: &TunnelTailscaleConfig,
) -> Result<(), String> {
    let binary = config.binary_path.trim();
    if !binary_path_available(binary) {
        return Err(format!("Tailscale binary not found: {}", binary));
    }
    let snapshot = ensure_tailscale_runtime_ready(config).await?;
    let mut command = tokio::process::Command::new(binary);
    apply_tailscale_socket_arg(&mut command, binary);
    command
        .arg("serve")
        .arg("--yes")
        .arg("8990")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if !config.auth_key.trim().is_empty() {
        command.env("TS_AUTHKEY", config.auth_key.trim());
    }
    if let Some(hostname) = config
        .hostname
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        command.env("TS_HOSTNAME", hostname.trim());
    }
    match command.spawn() {
        Ok(mut child) => {
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            {
                let mut tunnel = tunnel_arc.write().await;
                set_tunnel_running(&mut tunnel, child, TunnelProviderKind::TailscalePrivate);
                if let Some(hostname) = snapshot.dns_name.as_deref() {
                    tunnel.url = Some(format!("https://{}", hostname));
                }
            }
            if let Some(stdout) = stdout {
                spawn_tunnel_output_reader(
                    tunnel_arc.clone(),
                    TunnelProviderKind::TailscalePrivate,
                    stdout,
                    None,
                    "stdout",
                );
            }
            if let Some(stderr) = stderr {
                spawn_tunnel_output_reader(
                    tunnel_arc.clone(),
                    TunnelProviderKind::TailscalePrivate,
                    stderr,
                    None,
                    "stderr",
                );
            }
            crate::spawn_logged!(
                "src/channels/http/tunnel.rs:2104",
                spawn_tailscale_url_probe(
                    tunnel_arc,
                    TunnelProviderKind::TailscalePrivate,
                    binary.to_string(),
                    (!config.auth_key.trim().is_empty())
                        .then(|| config.auth_key.trim().to_string()),
                )
            );
            Ok(())
        }
        Err(e) => Err(format!("Failed to start Tailscale private access: {}", e)),
    }
}

async fn spawn_bore_tunnel(
    tunnel_arc: Arc<RwLock<TunnelState>>,
    config: &crate::core::config::TunnelBoreConfig,
) -> Result<(), String> {
    let binary = config.binary_path.trim();
    if !binary_path_available(binary) {
        return Err(format!("Bore binary not found: {}", binary));
    }
    if config.server.trim().is_empty() {
        return Err("Bore server is required before starting the tunnel.".to_string());
    }
    let mut command = tokio::process::Command::new(binary);
    command
        .args(["local", "8990", "--to", config.server.trim()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    match command.spawn() {
        Ok(mut child) => {
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            {
                let mut tunnel = tunnel_arc.write().await;
                set_tunnel_running(&mut tunnel, child, TunnelProviderKind::Bore);
            }
            let bore_server = Some(config.server.trim().to_string());
            if let Some(stdout) = stdout {
                spawn_tunnel_output_reader(
                    tunnel_arc.clone(),
                    TunnelProviderKind::Bore,
                    stdout,
                    bore_server.clone(),
                    "stdout",
                );
            }
            if let Some(stderr) = stderr {
                spawn_tunnel_output_reader(
                    tunnel_arc,
                    TunnelProviderKind::Bore,
                    stderr,
                    bore_server,
                    "stderr",
                );
            }
            Ok(())
        }
        Err(e) => Err(format!("Failed to start Bore tunnel: {}", e)),
    }
}

pub(super) async fn spawn_tunnel(
    state: &AppState,
    requested_provider: Option<TunnelProviderKind>,
) -> Result<(), String> {
    let config = load_tunnel_config(state).await;
    let provider = requested_provider.unwrap_or(config.provider);
    {
        let tunnel = state.tunnel.read().await;
        if tunnel.active {
            if tunnel.provider == provider {
                return Ok(());
            }
            return Err(format!(
                "{} tunnel is already active. Stop it before switching providers.",
                tunnel_provider_label(tunnel.provider)
            ));
        }
    }

    match provider {
        TunnelProviderKind::Cloudflare => {
            spawn_cloudflare_tunnel(state.tunnel.clone(), &config.cloudflare).await
        }
        TunnelProviderKind::Ngrok => spawn_ngrok_tunnel(state.tunnel.clone(), &config.ngrok).await,
        TunnelProviderKind::TailscalePrivate => {
            spawn_tailscale_private_access(state.tunnel.clone(), &config.tailscale_funnel).await
        }
        TunnelProviderKind::TailscaleFunnel => {
            spawn_tailscale_tunnel(state.tunnel.clone(), &config.tailscale_funnel).await
        }
        TunnelProviderKind::Bore => spawn_bore_tunnel(state.tunnel.clone(), &config.bore).await,
    }
}

/// POST /tunnel/start - start the selected public tunnel provider
pub(super) async fn start_tunnel(
    State(state): State<AppState>,
    Json(request): Json<StartTunnelRequest>,
) -> Response {
    let requested_app_id = request
        .app_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    if let Some(app_id) = requested_app_id.as_deref() {
        if !is_valid_app_id(app_id) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid app_id" })),
            )
                .into_response();
        }
        if state.app_registry.get_dir(app_id).await.is_none() {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "App not found" })),
            )
                .into_response();
        }
        if !state.app_registry.access_guard_enabled(app_id).await {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Public app exposure requires App Guard with an access password. Set one in Apps before starting public access."
                })),
            )
                .into_response();
        }
        if state
            .app_registry
            .access_key(app_id)
            .await
            .map(|value| value.trim().is_empty())
            .unwrap_or(true)
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Public app exposure requires a non-empty access password. Set one in Apps before starting public access."
                })),
            )
                .into_response();
        }
    }

    let requested_provider = match request.provider.as_deref() {
        Some(raw) => {
            let Some(provider) = parse_tunnel_provider_kind(raw) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "Unknown tunnel provider" })),
                )
                    .into_response();
            };
            Some(provider)
        }
        None => None,
    };
    let effective_provider =
        requested_provider.unwrap_or(load_tunnel_config(&state).await.provider);
    if requested_app_id.is_none() {
        if let Err(err) =
            tunnel_auth::ensure_control_plane_tunnel_ready(&state, effective_provider).await
        {
            return err.into_response();
        }
    }

    match spawn_tunnel(&state, requested_provider).await {
        Ok(()) => {
            {
                let mut tunnel = state.tunnel.write().await;
                tunnel.selected_app_id = requested_app_id.clone();
                tunnel.control_plane_enabled = requested_app_id.is_none();
            }
            persist_public_tunnel_state(&state, None, requested_app_id.as_deref()).await;
            let url = wait_for_tunnel_url(state.tunnel.clone(), 12).await;
            if let Some(found) = url.as_deref() {
                persist_public_tunnel_state(&state, Some(found), requested_app_id.as_deref()).await;
            }
            let config = load_tunnel_config(&state).await;
            let (provider, tunnel_active, tunnel_url, tunnel_error, selected_app_id) = {
                let tunnel = state.tunnel.read().await;
                (
                    if tunnel.active {
                        tunnel.provider
                    } else {
                        config.provider
                    },
                    tunnel.active,
                    tunnel.url.clone(),
                    tunnel.error.clone(),
                    tunnel.selected_app_id.clone(),
                )
            };
            if !tunnel_active {
                if let Some(err) = tunnel_error {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": err,
                            "provider": provider.as_str(),
                            "provider_label": tunnel_provider_label(provider)
                        })),
                    )
                        .into_response();
                }
            }
            Json(serde_json::json!({
                "ok": true,
                "url": tunnel_url,
                "selected_app_id": selected_app_id,
                "provider": provider.as_str(),
                "provider_label": tunnel_provider_label(provider),
                "message": if tunnel_url.is_some() {
                    format!("{} tunnel started", tunnel_provider_label(provider))
                } else {
                    format!("{} tunnel starting, URL pending...", tunnel_provider_label(provider))
                }
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": e
            })),
        )
            .into_response(),
    }
}

pub(super) async fn stop_tunnel_internal(state: &AppState) {
    let provider = {
        let mut tunnel = state.tunnel.write().await;
        if let Some(ref mut child) = tunnel.process {
            let _ = child.kill().await;
        }
        let provider = tunnel.provider;
        tunnel.process = None;
        tunnel.active = false;
        tunnel.url = None;
        tunnel.selected_app_id = None;
        tunnel.control_plane_enabled = false;
        tunnel.error = None;
        tracing::info!("Tunnel stopped by user");
        provider
    };
    if matches!(
        provider,
        TunnelProviderKind::TailscalePrivate | TunnelProviderKind::TailscaleFunnel
    ) {
        let config = load_tunnel_config(state).await;
        reset_tailscale_provider(provider, &config).await;
    }
    persist_public_tunnel_state(state, None, None).await;
}

pub(super) async fn reset_tunnel_to_infrastructure(
    state: &AppState,
) -> Result<Option<String>, String> {
    stop_tunnel_internal(state).await;
    auto_start_tunnel_infrastructure(state).await
}

/// POST /tunnel/stop - stop the active public tunnel
pub(super) async fn stop_tunnel(State(state): State<AppState>) -> Response {
    match reset_tunnel_to_infrastructure(&state).await {
        Ok(url) => Json(serde_json::json!({
            "ok": true,
            "message": "Public exposure stopped; tunnel infrastructure remains ready.",
            "active": true,
            "url": url,
            "selected_app_id": null,
            "control_plane_enabled": false
        }))
        .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "ok": false,
                "error": format!("Public exposure stopped, but tunnel infrastructure could not restart: {}", error),
                "selected_app_id": null,
                "control_plane_enabled": false
            })),
        )
            .into_response(),
    }
}

pub(super) async fn wait_for_tunnel_url(
    tunnel_arc: Arc<RwLock<TunnelState>>,
    attempts: usize,
) -> Option<String> {
    for _ in 0..attempts {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let tunnel = tunnel_arc.read().await;
        if let Some(url) = tunnel
            .url
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        {
            return Some(url);
        }
    }
    None
}
