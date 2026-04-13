use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures::StreamExt;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::process::Command;

const DEFAULT_LAN_HELPER_BIND: &str = "127.0.0.1:8995";
const DEFAULT_LAN_HELPER_URL: &str = "http://host.docker.internal:8995";
const DEFAULT_MAX_HOSTS: u16 = 64;
const HARD_MAX_HOSTS: u16 = 512;
const HTTP_METADATA_MAX_BYTES: usize = 16_384;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LanDiscoverArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cidr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hosts: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_host_local: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_http_metadata: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LanDiscoveryReport {
    pub devices: Vec<LanDevice>,
    pub host_apps: Vec<HostApp>,
    pub scan_scope: LanScanScope,
    pub warnings: Vec<String>,
    pub next_steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LanDevice {
    pub ip: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub confidence: f32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostApp {
    pub url: String,
    pub host: String,
    pub port: u16,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LanScanScope {
    pub mode: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cidrs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub probe_types: Vec<String>,
    pub max_hosts: u16,
    pub helper_used: bool,
    pub docker_detected: bool,
}

#[derive(Debug, Clone)]
struct HelperState {
    token: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct HelperError {
    error: String,
}

#[derive(Debug, Clone)]
struct Ipv4Cidr {
    raw: String,
    prefix: u8,
    first: u32,
    last: u32,
}

#[derive(Debug, Clone)]
struct DeviceSeed {
    ip: String,
    source: String,
    mac: Option<String>,
    hostname: Option<String>,
    category: Option<String>,
    confidence: f32,
    metadata: BTreeMap<String, String>,
}

pub async fn lan_discover(arguments: &serde_json::Value) -> Result<String> {
    let args: LanDiscoverArgs = serde_json::from_value(arguments.clone())
        .map_err(|e| anyhow::anyhow!("Invalid lan_discover arguments: {}", e))?;
    let report = discover_with_helper_or_fallback(args).await?;
    Ok(serde_json::to_string_pretty(&report)?)
}

pub async fn run_lan_helper(bind: String, token: Option<String>) -> Result<()> {
    let bind = if bind.trim().is_empty() {
        DEFAULT_LAN_HELPER_BIND.to_string()
    } else {
        bind.trim().to_string()
    };
    let addr: SocketAddr = bind
        .parse()
        .with_context(|| format!("Invalid LAN helper bind address '{}'", bind))?;

    let token = token
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("LAN helper requires --token or AGENTARK_LAN_HELPER_TOKEN")
        })?;
    let state = HelperState { token: Some(token) };
    let app = Router::new()
        .route("/health", get(lan_helper_health))
        .route("/discover", post(lan_helper_discover))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("AgentArk LAN helper listening on http://{}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn lan_helper_health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn lan_helper_discover(
    State(state): State<HelperState>,
    headers: HeaderMap,
    Json(args): Json<LanDiscoverArgs>,
) -> impl IntoResponse {
    if !helper_authorized(&headers, state.token.as_deref()) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(HelperError {
                error: "Missing or invalid LAN helper token".to_string(),
            }),
        )
            .into_response();
    }

    match discover_direct(args, "host_helper").await {
        Ok(report) => (StatusCode::OK, Json(report)).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(HelperError {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

fn helper_authorized(headers: &HeaderMap, token: Option<&str>) -> bool {
    let Some(expected) = token else {
        return true;
    };
    let Some(value) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    value
        .strip_prefix("Bearer ")
        .is_some_and(|actual| actual.trim() == expected)
}

async fn discover_with_helper_or_fallback(args: LanDiscoverArgs) -> Result<LanDiscoveryReport> {
    if let Some(helper_url) = resolve_helper_url() {
        match call_lan_helper(&helper_url, &args).await {
            Ok(mut report) => {
                report.scan_scope.helper_used = true;
                report.scan_scope.mode = "host_helper".to_string();
                return Ok(report);
            }
            Err(error) => {
                let mut report = discover_direct(args, "container_direct").await?;
                report.warnings.insert(
                    0,
                    format!(
                        "LAN helper at {} was not usable, so AgentArk fell back to container-visible discovery: {}",
                        helper_url, error
                    ),
                );
                return Ok(report);
            }
        }
    }

    discover_direct(args, "container_direct").await
}

fn resolve_helper_url() -> Option<String> {
    let explicit = std::env::var("AGENTARK_LAN_HELPER_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty());
    if explicit.is_some() {
        return explicit;
    }

    if running_in_docker()
        && std::env::var("AGENTARK_LAN_HELPER_TOKEN")
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
    {
        return Some(DEFAULT_LAN_HELPER_URL.to_string());
    }

    None
}

async fn call_lan_helper(helper_url: &str, args: &LanDiscoverArgs) -> Result<LanDiscoveryReport> {
    let token = std::env::var("AGENTARK_LAN_HELPER_TOKEN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("AGENTARK_LAN_HELPER_TOKEN is required for LAN helper use")
        })?;
    let url = format!("{}/discover", helper_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()?;
    let response = client
        .post(url)
        .bearer_auth(token)
        .json(args)
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("helper returned {} {}", status.as_u16(), body);
    }
    Ok(response.json::<LanDiscoveryReport>().await?)
}

async fn discover_direct(args: LanDiscoverArgs, mode: &str) -> Result<LanDiscoveryReport> {
    let docker_detected = running_in_docker();
    let max_hosts = args
        .max_hosts
        .unwrap_or(DEFAULT_MAX_HOSTS)
        .clamp(1, HARD_MAX_HOSTS);
    let cidr = validate_optional_cidr(args.cidr.as_deref(), max_hosts)?;
    let include_host_local = args.include_host_local.unwrap_or(true);
    let include_http_metadata = args.include_http_metadata.unwrap_or(true);
    let target = args
        .target
        .as_deref()
        .unwrap_or("all")
        .trim()
        .to_ascii_lowercase();

    let mut warnings = Vec::new();
    let mut probe_types = vec!["arp_neighbor_table".to_string(), "ssdp".to_string()];
    let mut devices = BTreeMap::<String, LanDevice>::new();

    if docker_detected && mode != "host_helper" {
        warnings.push(
            "AgentArk is running inside Docker bridge networking; mDNS/SSDP and host localhost visibility may be incomplete without the LAN helper.".to_string(),
        );
    }

    for seed in collect_neighbor_table_devices().await {
        merge_device_seed(&mut devices, seed);
    }

    match discover_ssdp_devices().await {
        Ok(seeds) => {
            for seed in seeds {
                merge_device_seed(&mut devices, seed);
            }
        }
        Err(error) => warnings.push(format!("SSDP discovery did not complete: {}", error)),
    }

    if let Some(cidr) = cidr.as_ref() {
        probe_types.push("bounded_private_cidr_hint".to_string());
        warnings.push(format!(
            "CIDR {} was validated as private and bounded; v1 discovery uses it as a scope hint and does not run a full subnet port scan.",
            cidr.raw
        ));
    }

    if include_http_metadata {
        probe_types.push("light_http_metadata".to_string());
        let ports = device_metadata_ports(&target);
        let device_ips = devices
            .keys()
            .take(max_hosts as usize)
            .cloned()
            .collect::<Vec<_>>();
        for seed in probe_device_http_metadata(device_ips, ports).await {
            merge_device_seed(&mut devices, seed);
        }
    }

    let host_apps = if include_host_local {
        probe_types.push("host_local_http_metadata".to_string());
        probe_host_apps(mode, docker_detected, &target).await
    } else {
        Vec::new()
    };

    let mut devices = devices.into_values().collect::<Vec<_>>();
    devices.sort_by(|a, b| a.ip.cmp(&b.ip));

    let mut next_steps = Vec::new();
    if devices.is_empty() && host_apps.is_empty() {
        next_steps.push(
            "No local devices or host apps were visible from this network vantage point."
                .to_string(),
        );
    } else {
        next_steps.push(
            "Review the candidates and ask before attempting any device-specific control action."
                .to_string(),
        );
    }
    if docker_detected && mode != "host_helper" {
        next_steps.push(
            "For better Docker discovery, run `agentark lan-helper --token <token>` on the host and set AGENTARK_LAN_HELPER_TOKEN plus AGENTARK_LAN_HELPER_URL in the AgentArk container.".to_string(),
        );
    }

    Ok(LanDiscoveryReport {
        devices,
        host_apps,
        scan_scope: LanScanScope {
            mode: mode.to_string(),
            cidrs: cidr.into_iter().map(|value| value.raw).collect(),
            probe_types,
            max_hosts,
            helper_used: mode == "host_helper",
            docker_detected,
        },
        warnings,
        next_steps,
    })
}

fn merge_device_seed(devices: &mut BTreeMap<String, LanDevice>, seed: DeviceSeed) {
    let entry = devices.entry(seed.ip.clone()).or_insert_with(|| LanDevice {
        ip: seed.ip.clone(),
        confidence: seed.confidence,
        ..Default::default()
    });
    if entry.mac.is_none() {
        entry.mac = seed.mac;
    }
    if entry.hostname.is_none() {
        entry.hostname = seed.hostname;
    }
    if entry.category.is_none() {
        entry.category = seed.category;
    }
    entry.confidence = entry.confidence.max(seed.confidence);
    if !entry.sources.iter().any(|source| source == &seed.source) {
        entry.sources.push(seed.source);
        entry.sources.sort();
    }
    for (key, value) in seed.metadata {
        entry.metadata.entry(key).or_insert(value);
    }
}

async fn collect_neighbor_table_devices() -> Vec<DeviceSeed> {
    let commands: &[(&str, &[&str])] = if cfg!(target_os = "windows") {
        &[("arp", &["-a"])]
    } else if cfg!(target_os = "macos") {
        &[("arp", &["-a"]), ("ifconfig", &[])]
    } else {
        &[("ip", &["neigh"]), ("arp", &["-an"])]
    };

    let mut seeds = Vec::new();
    let mut seen = BTreeSet::new();
    for (program, args) in commands {
        let Some(output) = run_short_command(program, args, Duration::from_secs(2)).await else {
            continue;
        };
        for (ip, mac) in parse_ip_mac_pairs(&output) {
            if !ip_is_allowed_lan_candidate(&ip) {
                continue;
            }
            let key = format!("{}:{}", ip, mac);
            if !seen.insert(key) {
                continue;
            }
            seeds.push(DeviceSeed {
                ip: ip.to_string(),
                source: "arp_neighbor_table".to_string(),
                mac: Some(normalize_mac(&mac)),
                hostname: None,
                category: None,
                confidence: 0.62,
                metadata: BTreeMap::new(),
            });
        }
    }
    seeds
}

async fn run_short_command(program: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut command = Command::new(program);
    command.args(args).kill_on_drop(true);
    let output = match tokio::time::timeout(timeout, command.output()).await {
        Ok(Ok(output)) => output,
        _ => return None,
    };
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

fn parse_ip_mac_pairs(output: &str) -> Vec<(Ipv4Addr, String)> {
    let re = Regex::new(r"(?i)(\d{1,3}(?:\.\d{1,3}){3}).{0,100}?(([0-9a-f]{2}[:-]){5}[0-9a-f]{2})")
        .expect("valid neighbor regex");
    re.captures_iter(output)
        .filter_map(|cap| {
            let ip = cap.get(1)?.as_str().parse::<Ipv4Addr>().ok()?;
            let mac = cap.get(2)?.as_str().to_string();
            Some((ip, mac))
        })
        .collect()
}

fn normalize_mac(raw: &str) -> String {
    raw.trim().replace('-', ":").to_ascii_lowercase()
}

async fn discover_ssdp_devices() -> Result<Vec<DeviceSeed>> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    let message = concat!(
        "M-SEARCH * HTTP/1.1\r\n",
        "HOST: 239.255.255.250:1900\r\n",
        "MAN: \"ssdp:discover\"\r\n",
        "MX: 1\r\n",
        "ST: ssdp:all\r\n",
        "\r\n"
    );
    socket
        .send_to(message.as_bytes(), "239.255.255.250:1900")
        .await?;

    let deadline = Instant::now() + Duration::from_millis(1400);
    let mut buf = vec![0u8; 4096];
    let mut seeds = Vec::new();
    let mut seen = BTreeSet::new();
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let Ok(Ok((len, _addr))) =
            tokio::time::timeout(remaining, socket.recv_from(&mut buf)).await
        else {
            break;
        };
        let raw = String::from_utf8_lossy(&buf[..len]).to_string();
        if let Some(seed) = parse_ssdp_response(&raw) {
            if seen.insert(seed.ip.clone()) {
                seeds.push(seed);
            }
        }
    }
    Ok(seeds)
}

fn parse_ssdp_response(raw: &str) -> Option<DeviceSeed> {
    let mut headers = BTreeMap::new();
    for line in raw.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        headers.insert(
            key.trim().to_ascii_lowercase(),
            value.trim().trim_matches('"').to_string(),
        );
    }
    let location = headers.get("location")?;
    let parsed = reqwest::Url::parse(location).ok()?;
    let host = parsed.host_str()?.trim_matches(['[', ']']);
    let ip = host.parse::<Ipv4Addr>().ok()?;
    if !ip_is_allowed_lan_candidate(&ip) {
        return None;
    }

    let mut metadata = BTreeMap::new();
    metadata.insert("location".to_string(), location.to_string());
    for key in ["server", "st", "usn"] {
        if let Some(value) = headers.get(key) {
            metadata.insert(key.to_string(), value.clone());
        }
    }
    let category = infer_category_from_text(
        &[
            headers.get("server").map(String::as_str).unwrap_or(""),
            headers.get("st").map(String::as_str).unwrap_or(""),
            headers.get("usn").map(String::as_str).unwrap_or(""),
            location,
        ]
        .join(" "),
    );
    Some(DeviceSeed {
        ip: ip.to_string(),
        source: "ssdp".to_string(),
        mac: None,
        hostname: None,
        category,
        confidence: 0.78,
        metadata,
    })
}

async fn probe_host_apps(mode: &str, docker_detected: bool, target: &str) -> Vec<HostApp> {
    if !matches!(target, "all" | "localhost_apps" | "apps" | "dev" | "local") {
        return Vec::new();
    }
    let hosts = if mode == "host_helper" || !docker_detected {
        vec!["127.0.0.1".to_string()]
    } else {
        vec!["host.docker.internal".to_string()]
    };
    let ports = vec![
        3000, 3001, 4200, 5000, 5001, 5173, 5174, 8000, 8080, 8990, 8992, 11434,
    ];
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(900))
        .redirect(reqwest::redirect::Policy::limited(2))
        .build()
        .ok();
    let Some(client) = client else {
        return Vec::new();
    };
    let probes = hosts
        .into_iter()
        .flat_map(|host| ports.iter().map(move |port| (host.clone(), *port)))
        .collect::<Vec<_>>();
    futures::stream::iter(probes.into_iter().map(|(host, port)| {
        let client = client.clone();
        async move { probe_host_app(client, host, port).await }
    }))
    .buffer_unordered(8)
    .filter_map(|result| async move { result })
    .collect::<Vec<_>>()
    .await
}

async fn probe_host_app(client: reqwest::Client, host: String, port: u16) -> Option<HostApp> {
    let url = format!("http://{}:{}/", host, port);
    let response = client.get(&url).send().await.ok()?;
    let status = response.status();
    let server = response
        .headers()
        .get(reqwest::header::SERVER)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let body = response
        .text()
        .await
        .unwrap_or_default()
        .chars()
        .take(HTTP_METADATA_MAX_BYTES)
        .collect::<String>();
    Some(HostApp {
        url,
        host,
        port,
        status: status.as_u16(),
        title: extract_title(&body),
        server,
    })
}

fn device_metadata_ports(target: &str) -> Vec<u16> {
    match target {
        "sonos" | "speaker" | "speakers" => vec![1400, 80],
        "lights" | "light" | "hue" => vec![80, 443],
        "all" => vec![80],
        _ => vec![80],
    }
}

async fn probe_device_http_metadata(ips: Vec<String>, ports: Vec<u16>) -> Vec<DeviceSeed> {
    if ips.is_empty() || ports.is_empty() {
        return Vec::new();
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(900))
        .redirect(reqwest::redirect::Policy::limited(1))
        .build()
        .ok();
    let Some(client) = client else {
        return Vec::new();
    };
    let probes = ips
        .into_iter()
        .flat_map(|ip| ports.iter().map(move |port| (ip.clone(), *port)))
        .collect::<Vec<_>>();
    futures::stream::iter(probes.into_iter().map(|(ip, port)| {
        let client = client.clone();
        async move { probe_device_http(client, ip, port).await }
    }))
    .buffer_unordered(12)
    .filter_map(|result| async move { result })
    .collect::<Vec<_>>()
    .await
}

async fn probe_device_http(client: reqwest::Client, ip: String, port: u16) -> Option<DeviceSeed> {
    let url = if port == 1400 {
        format!("http://{}:{}/xml/device_description.xml", ip, port)
    } else {
        format!("http://{}:{}/", ip, port)
    };
    let response = client.get(&url).send().await.ok()?;
    let status = response.status();
    let server = response
        .headers()
        .get(reqwest::header::SERVER)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let body = response
        .text()
        .await
        .unwrap_or_default()
        .chars()
        .take(HTTP_METADATA_MAX_BYTES)
        .collect::<String>();
    let mut metadata = BTreeMap::new();
    metadata.insert(format!("http_{}_status", port), status.as_u16().to_string());
    metadata.insert(format!("http_{}_url", port), url);
    if let Some(server) = server {
        metadata.insert(format!("http_{}_server", port), server);
    }
    if let Some(title) = extract_title(&body) {
        metadata.insert(format!("http_{}_title", port), title);
    }
    let category = infer_category_from_text(&body);
    Some(DeviceSeed {
        ip,
        source: "light_http_metadata".to_string(),
        mac: None,
        hostname: None,
        category,
        confidence: 0.7,
        metadata,
    })
}

fn extract_title(body: &str) -> Option<String> {
    let re = Regex::new(r"(?is)<title[^>]*>(.*?)</title>").expect("valid title regex");
    re.captures(body)
        .and_then(|cap| cap.get(1))
        .map(|value| value.as_str().trim().replace('\n', " "))
        .filter(|value| !value.is_empty())
}

fn infer_category_from_text(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("sonos") {
        Some("sonos".to_string())
    } else if lower.contains("hue") || lower.contains("philips lighting") {
        Some("lighting".to_string())
    } else if lower.contains("chromecast") || lower.contains("google cast") {
        Some("media_cast".to_string())
    } else if lower.contains("roku") {
        Some("media_player".to_string())
    } else if lower.contains("printer") {
        Some("printer".to_string())
    } else {
        None
    }
}

fn validate_optional_cidr(raw: Option<&str>, max_hosts: u16) -> Result<Option<Ipv4Cidr>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let cidr = parse_ipv4_cidr(raw)?;
    if cidr.prefix < 24 {
        anyhow::bail!(
            "CIDR '{}' is too broad for v1 LAN discovery; use /24 or narrower",
            raw
        );
    }
    let host_count = cidr.host_count();
    if host_count > max_hosts as u32 {
        anyhow::bail!(
            "CIDR '{}' covers {} addresses, above max_hosts {}",
            raw,
            host_count,
            max_hosts
        );
    }
    let first = Ipv4Addr::from(cidr.first);
    let last = Ipv4Addr::from(cidr.last);
    if !first.is_private() || !last.is_private() {
        anyhow::bail!(
            "CIDR '{}' must stay inside private RFC1918 address space",
            raw
        );
    }
    Ok(Some(cidr))
}

fn parse_ipv4_cidr(raw: &str) -> Result<Ipv4Cidr> {
    let (ip_raw, prefix_raw) = raw.split_once('/').ok_or_else(|| {
        anyhow::anyhow!(
            "CIDR '{}' must include a prefix, for example 192.168.1.0/24",
            raw
        )
    })?;
    let ip: Ipv4Addr = ip_raw
        .parse()
        .with_context(|| format!("Invalid IPv4 CIDR address '{}'", ip_raw))?;
    let prefix: u8 = prefix_raw
        .parse()
        .with_context(|| format!("Invalid IPv4 CIDR prefix '{}'", prefix_raw))?;
    if prefix > 32 {
        anyhow::bail!("CIDR '{}' has an invalid prefix", raw);
    }
    let ip_u32 = u32::from(ip);
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    let network = ip_u32 & mask;
    let broadcast = network | !mask;
    Ok(Ipv4Cidr {
        raw: raw.to_string(),
        prefix,
        first: network,
        last: broadcast,
    })
}

impl Ipv4Cidr {
    fn host_count(&self) -> u32 {
        self.last.saturating_sub(self.first).saturating_add(1)
    }
}

fn ip_is_allowed_lan_candidate(ip: &Ipv4Addr) -> bool {
    ip.is_private() && !ip.is_loopback() && !ip.is_link_local() && !ip.is_multicast()
}

fn running_in_docker() -> bool {
    std::env::var("AGENTARK_STACK_ROLE")
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
        || std::path::Path::new("/.dockerenv").exists()
        || std::fs::read_to_string("/proc/1/cgroup")
            .ok()
            .is_some_and(|value| value.contains("docker") || value.contains("containerd"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cidr_validation_allows_private_bounded_scope() {
        let cidr = validate_optional_cidr(Some("192.168.1.0/25"), 128)
            .unwrap()
            .unwrap();
        assert_eq!(Ipv4Addr::from(cidr.first), Ipv4Addr::new(192, 168, 1, 0));
        assert_eq!(cidr.host_count(), 128);
    }

    #[test]
    fn cidr_validation_rejects_public_scope() {
        let err = validate_optional_cidr(Some("8.8.8.0/24"), 256).unwrap_err();
        assert!(err.to_string().contains("private"));
    }

    #[test]
    fn cidr_validation_rejects_overbroad_scope() {
        let err = validate_optional_cidr(Some("192.168.0.0/16"), 512).unwrap_err();
        assert!(err.to_string().contains("too broad"));
    }

    #[test]
    fn cidr_validation_rejects_scope_above_max_hosts() {
        let err = validate_optional_cidr(Some("10.0.0.0/24"), 64).unwrap_err();
        assert!(err.to_string().contains("max_hosts"));
    }

    #[test]
    fn ssdp_response_extracts_private_location_host() {
        let response = concat!(
            "HTTP/1.1 200 OK\r\n",
            "LOCATION: http://192.168.1.20:1400/xml/device_description.xml\r\n",
            "SERVER: Linux UPnP/1.0 Sonos/70.3\r\n",
            "ST: urn:schemas-upnp-org:device:ZonePlayer:1\r\n",
            "\r\n"
        );
        let seed = parse_ssdp_response(response).unwrap();
        assert_eq!(seed.ip, "192.168.1.20");
        assert_eq!(seed.category.as_deref(), Some("sonos"));
    }
}
