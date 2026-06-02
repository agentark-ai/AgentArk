//! MCP registry for managing external servers and tool/resource bindings

use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::actions::ActionDef;
use crate::core::config::{
    AgentConfig, McpAuthConfig, McpAuthSecret, McpServerConfig, McpTransportConfig, Secrets,
};
use crate::runtime::{ActionRuntime, McpBinding, McpBindingKind};
use crate::safety::{RuleAction, RuleTrigger, SafetyEngine, SafetyRule};
use crate::storage::Storage;

use super::client::{McpAuth, McpClient};
use super::{McpResource, McpTool};

#[derive(Debug, Clone, Serialize)]
pub struct McpServerView {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub resources_enabled: bool,
    pub transport: McpTransportView,
    pub auth: McpAuthView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_profile_id: Option<String>,
    pub tool_allowlist: Vec<String>,
    pub tool_blocklist: Vec<String>,
    pub resource_allowlist: Vec<String>,
    pub timeout_secs: u64,
    pub max_response_bytes: usize,
    pub tool_count: usize,
    pub resource_count: usize,
    pub warnings: Vec<String>,
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<McpTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<Vec<McpResource>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransportView {
    Http {
        url: String,
    },
    Stdio {
        command: String,
        args: Vec<String>,
        working_dir: Option<String>,
        env_keys: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct McpAuthView {
    pub auth_type: String,
    pub has_auth: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

pub struct McpRegistry {
    storage: Storage,
    servers: HashMap<String, McpServerState>,
}

struct McpServerState {
    config: McpServerConfig,
    client: tokio::sync::Mutex<McpClient>,
    tools: Vec<McpTool>,
    resources: Vec<McpResource>,
    catalog_hash: String,
    warnings: Vec<String>,
    last_error: Option<String>,
    has_auth: bool,
    last_call_at: Option<Instant>,
}

const MCP_MIN_CALL_INTERVAL: Duration = Duration::from_millis(500);

async fn enforce_mcp_call_rate_limit(state: &mut McpServerState) {
    if let Some(last_call_at) = state.last_call_at {
        let elapsed = last_call_at.elapsed();
        if elapsed < MCP_MIN_CALL_INTERVAL {
            tokio::time::sleep(MCP_MIN_CALL_INTERVAL - elapsed).await;
        }
    }
    state.last_call_at = Some(Instant::now());
}

impl McpRegistry {
    pub fn new(storage: Storage) -> Self {
        Self {
            storage,
            servers: HashMap::new(),
        }
    }

    pub async fn list_servers(&self, include_details: bool) -> Result<Vec<McpServerView>> {
        let mut views = Vec::with_capacity(self.servers.len());
        for state in self.servers.values() {
            views.push(
                state.view_with_has_auth(include_details, self.server_has_auth(state).await?),
            );
        }
        Ok(views)
    }

    pub async fn get_server(
        &self,
        id: &str,
        include_details: bool,
    ) -> Result<Option<McpServerView>> {
        let Some(state) = self.servers.get(id) else {
            return Ok(None);
        };
        Ok(Some(state.view_with_has_auth(
            include_details,
            self.server_has_auth(state).await?,
        )))
    }

    pub async fn sync_from_config(
        &mut self,
        config: &AgentConfig,
        secrets: &Secrets,
        runtime: &ActionRuntime,
        safety: &SafetyEngine,
    ) -> Result<()> {
        runtime.unregister_mcp_actions().await;
        self.servers.clear();

        for server in &config.mcp.servers {
            let state = build_server_state(server, config, secrets, runtime, safety).await?;
            self.servers.insert(server.id.clone(), state);
        }

        Ok(())
    }

    pub async fn refresh_server(
        &mut self,
        id: &str,
        config: &AgentConfig,
        secrets: &Secrets,
        runtime: &ActionRuntime,
        safety: &SafetyEngine,
    ) -> Result<()> {
        runtime.unregister_mcp_actions_for_server(id).await;
        let server = config
            .mcp
            .servers
            .iter()
            .find(|server| server.id == id)
            .ok_or_else(|| anyhow!("MCP server not found"))?;
        let state = build_server_state(server, config, secrets, runtime, safety).await?;
        self.servers.insert(id.to_string(), state);
        Ok(())
    }

    pub async fn call_tool(
        &mut self,
        server_id: &str,
        tool_name: &str,
        arguments: &Value,
    ) -> Result<String> {
        let state = self
            .servers
            .get_mut(server_id)
            .ok_or_else(|| anyhow!("MCP server not found"))?;
        if !state.config.enabled {
            return Err(anyhow!("MCP server is disabled"));
        }
        enforce_mcp_call_rate_limit(state).await;
        let sanitized_arguments = crate::security::sanitize_outbound_json(
            arguments,
            &crate::security::OutboundPrivacyPolicy::default(),
        )
        .sanitized_value;
        if matches!(&state.config.transport, McpTransportConfig::Http { .. }) {
            if let Some(auth_profile_id) = state
                .config
                .auth_profile_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let resolved = crate::core::auth_profiles::AuthProfileControlPlane::resolve_http(
                    &self.storage,
                    auth_profile_id,
                )
                .await?;
                let auth = auth_profile_to_mcp_auth(&resolved);
                state.client = tokio::sync::Mutex::new(McpClient::new(
                    &state.config,
                    auth,
                    std::collections::HashMap::new(),
                )?);
            }
        }
        let mut client = state.client.lock().await;
        let current_tools = filter_tools(
            client.list_tools().await?,
            &state.config.tool_allowlist,
            &state.config.tool_blocklist,
        );
        if tool_catalog_hash(&current_tools, &state.resources)? != state.catalog_hash {
            return Err(anyhow!(
                "MCP tool catalog changed after registration; refresh and review this server before calling tools"
            ));
        }
        let result = client.call_tool(tool_name, &sanitized_arguments).await?;
        if matches!(&state.config.transport, McpTransportConfig::Http { .. }) {
            if let Some(auth_profile_id) = state.config.auth_profile_id.as_deref() {
                let _ = crate::core::auth_profiles::AuthProfileControlPlane::mark_used(
                    &self.storage,
                    auth_profile_id,
                )
                .await;
            }
        }
        Ok(format_mcp_result(&result))
    }

    pub async fn read_resource(&mut self, server_id: &str, uri: &str) -> Result<String> {
        let state = self
            .servers
            .get_mut(server_id)
            .ok_or_else(|| anyhow!("MCP server not found"))?;
        if !state.config.enabled || !state.config.resources_enabled {
            return Err(anyhow!("MCP resources are disabled"));
        }
        enforce_mcp_call_rate_limit(state).await;
        validate_resource_uri(uri)?;
        if matches!(&state.config.transport, McpTransportConfig::Http { .. }) {
            if let Some(auth_profile_id) = state
                .config
                .auth_profile_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let resolved = crate::core::auth_profiles::AuthProfileControlPlane::resolve_http(
                    &self.storage,
                    auth_profile_id,
                )
                .await?;
                let auth = auth_profile_to_mcp_auth(&resolved);
                state.client = tokio::sync::Mutex::new(McpClient::new(
                    &state.config,
                    auth,
                    std::collections::HashMap::new(),
                )?);
            }
        }
        let mut client = state.client.lock().await;
        let current_resources = filter_resources(
            client.list_resources().await?,
            &state.config.resource_allowlist,
        );
        if tool_catalog_hash(&state.tools, &current_resources)? != state.catalog_hash {
            return Err(anyhow!(
                "MCP resource catalog changed after registration; refresh and review this server before reading resources"
            ));
        }
        let result = client.read_resource(uri).await?;
        if matches!(&state.config.transport, McpTransportConfig::Http { .. }) {
            if let Some(auth_profile_id) = state.config.auth_profile_id.as_deref() {
                let _ = crate::core::auth_profiles::AuthProfileControlPlane::mark_used(
                    &self.storage,
                    auth_profile_id,
                )
                .await;
            }
        }
        Ok(format_mcp_result(&result))
    }

    async fn server_has_auth(&self, state: &McpServerState) -> Result<bool> {
        let Some(auth_profile_id) = state
            .config
            .auth_profile_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(state.has_auth);
        };
        Ok(
            crate::core::auth_profiles::AuthProfileControlPlane::get(
                &self.storage,
                auth_profile_id,
            )
            .await?
            .is_some_and(|profile| profile.ready),
        )
    }
}

async fn build_server_state(
    server: &McpServerConfig,
    config: &AgentConfig,
    secrets: &Secrets,
    runtime: &ActionRuntime,
    safety: &SafetyEngine,
) -> Result<McpServerState> {
    let auth_secret = secrets.mcp_auth.get(&server.id);
    let (auth, auth_warnings, has_auth) = if let Some(auth_profile_id) = server
        .auth_profile_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if !matches!(&server.transport, McpTransportConfig::Http { .. }) {
            (
                None,
                vec!["HTTP auth profiles cannot be attached to stdio MCP transports.".to_string()],
                false,
            )
        } else {
            let storage = runtime.storage().ok_or_else(|| {
                anyhow!("Storage is required for auth profile-backed MCP servers")
            })?;
            match crate::core::auth_profiles::AuthProfileControlPlane::resolve_http(
                &storage,
                auth_profile_id,
            )
            .await
            {
                Ok(resolved) => (auth_profile_to_mcp_auth(&resolved), Vec::new(), true),
                Err(error) => (
                    None,
                    vec![format!(
                        "MCP auth profile '{}' is not ready: {}",
                        auth_profile_id, error
                    )],
                    false,
                ),
            }
        }
    } else {
        resolve_auth(&server.auth, auth_secret)
    };
    let mut warnings = compute_mcp_warnings(server);
    warnings.extend(auth_warnings);
    let env = resolve_stdio_env(server, config, secrets);
    if let McpTransportConfig::Http { url } = &server.transport {
        crate::core::net::validate_external_https_url(url).await?;
    }

    let mut client = McpClient::new(server, auth, env)?;
    let mut tools = Vec::new();
    let mut resources = Vec::new();
    let mut last_error = None;

    if server.enabled {
        match client.list_tools().await {
            Ok(list) => tools = filter_tools(list, &server.tool_allowlist, &server.tool_blocklist),
            Err(e) => last_error = Some(e.to_string()),
        }

        if server.resources_enabled {
            match client.list_resources().await {
                Ok(list) => resources = filter_resources(list, &server.resource_allowlist),
                Err(e) => last_error = Some(e.to_string()),
            }
        }
    }

    if server.enabled && last_error.is_none() {
        register_actions(
            runtime, safety, server, &tools, &resources, &warnings, has_auth,
        )
        .await?;
    }

    let catalog_hash = tool_catalog_hash(&tools, &resources)?;
    Ok(McpServerState {
        config: server.clone(),
        client: tokio::sync::Mutex::new(client),
        tools,
        resources,
        catalog_hash,
        warnings,
        last_error,
        has_auth,
        last_call_at: None,
    })
}

impl McpServerState {
    fn view_with_has_auth(&self, include_details: bool, has_auth: bool) -> McpServerView {
        McpServerView {
            id: self.config.id.clone(),
            name: self.config.name.clone(),
            description: self.config.description.clone(),
            enabled: self.config.enabled,
            resources_enabled: self.config.resources_enabled,
            transport: transport_view(&self.config.transport),
            auth: if self.config.auth_profile_id.is_some() {
                McpAuthView {
                    auth_type: "auth_profile".to_string(),
                    has_auth,
                    header: None,
                    name: None,
                }
            } else {
                auth_view(&self.config.auth, has_auth)
            },
            auth_profile_id: self.config.auth_profile_id.clone(),
            tool_allowlist: self.config.tool_allowlist.clone(),
            tool_blocklist: self.config.tool_blocklist.clone(),
            resource_allowlist: self.config.resource_allowlist.clone(),
            timeout_secs: self.config.timeout_secs,
            max_response_bytes: self.config.max_response_bytes,
            tool_count: self.tools.len(),
            resource_count: self.resources.len(),
            warnings: self.warnings.clone(),
            last_error: self.last_error.clone(),
            tools: if include_details {
                Some(self.tools.clone())
            } else {
                None
            },
            resources: if include_details {
                Some(self.resources.clone())
            } else {
                None
            },
        }
    }
}

fn transport_view(transport: &McpTransportConfig) -> McpTransportView {
    match transport {
        McpTransportConfig::Http { url } => McpTransportView::Http { url: url.clone() },
        McpTransportConfig::Stdio {
            command,
            args,
            working_dir,
            env_keys,
        } => McpTransportView::Stdio {
            command: command.clone(),
            args: args.clone(),
            working_dir: working_dir.clone(),
            env_keys: env_keys.clone(),
        },
    }
}

fn auth_view(auth: &Option<McpAuthConfig>, has_auth: bool) -> McpAuthView {
    match auth {
        None => McpAuthView {
            auth_type: "none".to_string(),
            has_auth,
            header: None,
            name: None,
        },
        Some(McpAuthConfig::Bearer { header }) => McpAuthView {
            auth_type: "bearer".to_string(),
            has_auth,
            header: Some(header.clone()),
            name: None,
        },
        Some(McpAuthConfig::Basic) => McpAuthView {
            auth_type: "basic".to_string(),
            has_auth,
            header: None,
            name: None,
        },
        Some(McpAuthConfig::Header { name }) => McpAuthView {
            auth_type: "header".to_string(),
            has_auth,
            header: None,
            name: Some(name.clone()),
        },
        Some(McpAuthConfig::Query { name }) => McpAuthView {
            auth_type: "query".to_string(),
            has_auth,
            header: None,
            name: Some(name.clone()),
        },
    }
}

fn auth_profile_to_mcp_auth(
    resolved: &crate::core::auth_profiles::AuthProfileResolution,
) -> Option<McpAuth> {
    let headers = resolved
        .overlay
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<Vec<_>>();
    let query = resolved
        .overlay
        .query
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<Vec<_>>();
    let basic = resolved.overlay.basic.clone();

    if headers.is_empty() && query.is_empty() && basic.is_none() {
        None
    } else {
        Some(McpAuth::Composite {
            headers,
            query,
            basic,
        })
    }
}

fn resolve_stdio_env(
    server: &McpServerConfig,
    _config: &AgentConfig,
    secrets: &Secrets,
) -> HashMap<String, String> {
    let allowed_keys: HashSet<String> = match &server.transport {
        McpTransportConfig::Stdio { env_keys, .. } => env_keys
            .iter()
            .map(|key| key.trim())
            .filter(|key| !key.is_empty())
            .map(str::to_string)
            .collect(),
        McpTransportConfig::Http { .. } => return HashMap::new(),
    };
    if allowed_keys.is_empty() {
        return HashMap::new();
    }
    let mut env = secrets.mcp_env.get(&server.id).cloned().unwrap_or_default();

    env.retain(|key, value| {
        let trimmed_key = key.trim();
        !trimmed_key.is_empty() && !value.trim().is_empty() && allowed_keys.contains(trimmed_key)
    });
    env
}

pub fn compute_mcp_warnings(config: &McpServerConfig) -> Vec<String> {
    let mut warnings = Vec::new();
    match &config.transport {
        McpTransportConfig::Http { url } => {
            if let Ok(parsed) = url::Url::parse(url) {
                if parsed.scheme() != "https" {
                    warnings.push(
                        "MCP server uses non-TLS HTTP. Credentials and data may be exposed."
                            .to_string(),
                    );
                }
                if let Some(host) = parsed.host_str() {
                    if is_private_host(host) {
                        warnings.push("MCP server points to a private/local address. Only connect to servers you trust.".to_string());
                    }
                }
            } else {
                warnings.push("MCP server URL is invalid.".to_string());
            }
        }
        McpTransportConfig::Stdio { .. } => {
            warnings.push("MCP stdio runs a local process. Only use trusted binaries.".to_string());
        }
    }
    if config.resources_enabled {
        warnings.push("MCP resources are enabled. Resource content is untrusted and may contain malicious instructions.".to_string());
    }
    warnings
}

fn is_private_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
            std::net::IpAddr::V6(v6) => {
                v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local()
            }
        };
    }
    false
}

fn resolve_auth(
    auth: &Option<McpAuthConfig>,
    secret: Option<&McpAuthSecret>,
) -> (Option<McpAuth>, Vec<String>, bool) {
    let has_auth = secret
        .and_then(|s| {
            s.token
                .as_ref()
                .or(s.username.as_ref())
                .or(s.password.as_ref())
        })
        .is_some();
    let mut warnings = Vec::new();

    match auth {
        None => (None, warnings, has_auth),
        Some(McpAuthConfig::Bearer { header }) => {
            let token = secret.and_then(|s| s.token.clone()).unwrap_or_default();
            if token.is_empty() {
                warnings.push("MCP auth configured (bearer) but no token stored.".to_string());
                return (None, warnings, false);
            }
            (
                Some(McpAuth::Bearer {
                    header: header.clone(),
                    token,
                }),
                warnings,
                true,
            )
        }
        Some(McpAuthConfig::Basic) => {
            let username = secret.and_then(|s| s.username.clone()).unwrap_or_default();
            let password = secret.and_then(|s| s.password.clone()).unwrap_or_default();
            if username.is_empty() || password.is_empty() {
                warnings
                    .push("MCP auth configured (basic) but username/password missing.".to_string());
                return (None, warnings, false);
            }
            (Some(McpAuth::Basic { username, password }), warnings, true)
        }
        Some(McpAuthConfig::Header { name }) => {
            let value = secret.and_then(|s| s.token.clone()).unwrap_or_default();
            if value.is_empty() {
                warnings.push("MCP auth configured (header) but no value stored.".to_string());
                return (None, warnings, false);
            }
            (
                Some(McpAuth::Header {
                    name: name.clone(),
                    value,
                }),
                warnings,
                true,
            )
        }
        Some(McpAuthConfig::Query { name }) => {
            let value = secret.and_then(|s| s.token.clone()).unwrap_or_default();
            if value.is_empty() {
                warnings.push("MCP auth configured (query) but no value stored.".to_string());
                return (None, warnings, false);
            }
            (
                Some(McpAuth::Query {
                    name: name.clone(),
                    value,
                }),
                warnings,
                true,
            )
        }
    }
}

fn filter_tools(tools: Vec<McpTool>, allowlist: &[String], blocklist: &[String]) -> Vec<McpTool> {
    let blocked: HashSet<&str> = blocklist.iter().map(|s| s.as_str()).collect();
    let allowed: HashSet<&str> = allowlist.iter().map(|s| s.as_str()).collect();
    tools
        .into_iter()
        .filter(|t| {
            // Blocklist always takes precedence
            if blocked.contains(t.name.as_str()) {
                return false;
            }
            // If allowlist is empty, allow all non-blocked tools
            if allowed.is_empty() {
                return true;
            }
            // Otherwise, must be in allowlist
            allowed.contains(t.name.as_str())
        })
        .map(|mut tool| {
            tool.description = crate::security::sanitize_untrusted_output(
                "mcp_tool_description",
                &tool.description,
            );
            tool.input_schema = crate::security::sanitize_input_schema(&tool.input_schema);
            tool
        })
        .collect()
}

fn filter_resources(resources: Vec<McpResource>, allowlist: &[String]) -> Vec<McpResource> {
    if allowlist.is_empty() {
        return Vec::new();
    }
    let allowed: HashSet<&str> = allowlist.iter().map(|s| s.as_str()).collect();
    resources
        .into_iter()
        .filter(|r| allowed.contains(r.uri.as_str()) && validate_resource_uri(&r.uri).is_ok())
        .map(|mut resource| {
            resource.description = crate::security::sanitize_untrusted_output(
                "mcp_resource_description",
                &resource.description,
            );
            resource
        })
        .collect()
}

fn validate_resource_uri(uri: &str) -> Result<()> {
    let trimmed = uri.trim();
    anyhow::ensure!(!trimmed.is_empty(), "MCP resource URI is empty");
    let parsed = url::Url::parse(trimmed).map_err(|_| anyhow!("MCP resource URI is invalid"))?;
    match parsed.scheme() {
        "agentark" | "https" | "mcp" | "urn" => {}
        other => anyhow::bail!("MCP resource URI scheme '{}' is not allowed", other),
    }
    anyhow::ensure!(
        !parsed
            .path_segments()
            .is_some_and(|mut segments| segments.any(|segment| segment == "..")),
        "MCP resource URI contains path traversal"
    );
    Ok(())
}

fn tool_catalog_hash(tools: &[McpTool], resources: &[McpResource]) -> Result<String> {
    let canonical = serde_json::to_vec(&serde_json::json!({
        "tools": tools,
        "resources": resources,
    }))?;
    let mut hasher = Sha256::new();
    hasher.update(b"agentark-mcp-tool-catalog-v1");
    hasher.update([0]);
    hasher.update(canonical);
    Ok(hex::encode(hasher.finalize()))
}

async fn register_actions(
    runtime: &ActionRuntime,
    safety: &SafetyEngine,
    server: &McpServerConfig,
    tools: &[McpTool],
    resources: &[McpResource],
    warnings: &[String],
    has_auth: bool,
) -> Result<()> {
    let mut used_names: HashMap<String, usize> = HashMap::new();

    for tool in tools {
        let action_name = unique_action_name(&server.id, "tool", &tool.name, &mut used_names);
        let def = ActionDef {
            name: action_name.clone(),
            description:
                "MCP tool from a configured server. Review server tools in settings before use."
                    .to_string(),
            version: "1.0.0".to_string(),
            input_schema: tool.input_schema.clone(),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(crate::runtime::SandboxMode::Native),
            source: crate::actions::ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        };
        runtime
            .register_mcp_action(
                def,
                McpBinding {
                    server_id: server.id.clone(),
                    server_name: server.name.clone(),
                    warnings: warnings.to_vec(),
                    auth_profile_id: server.auth_profile_id.clone(),
                    auth_required: server.auth.is_some() || server.auth_profile_id.is_some(),
                    auth_configured: if server.auth.is_some() || server.auth_profile_id.is_some() {
                        has_auth
                    } else {
                        true
                    },
                    kind: McpBindingKind::Tool {
                        name: tool.name.clone(),
                    },
                },
            )
            .await;

        safety.add_rule(SafetyRule {
            name: format!("mcp_approve_{}", action_name),
            description: mcp_safety_rule_description(server, &tool.name, false),
            trigger: RuleTrigger::Action { name: action_name },
            condition: None,
            action: mcp_safety_rule_action(server, &tool.name, false),
            verified: true,
        });
    }

    for resource in resources {
        let action_name =
            unique_action_name(&server.id, "resource", &resource.name, &mut used_names);
        let def = ActionDef {
            name: action_name.clone(),
            description:
                "MCP resource from a configured server. Review server resources in settings before use."
                    .to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(crate::runtime::SandboxMode::Native),
            source: crate::actions::ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        };
        runtime
            .register_mcp_action(
                def,
                McpBinding {
                    server_id: server.id.clone(),
                    server_name: server.name.clone(),
                    warnings: warnings.to_vec(),
                    auth_profile_id: server.auth_profile_id.clone(),
                    auth_required: server.auth.is_some() || server.auth_profile_id.is_some(),
                    auth_configured: if server.auth.is_some() || server.auth_profile_id.is_some() {
                        has_auth
                    } else {
                        true
                    },
                    kind: McpBindingKind::Resource {
                        uri: resource.uri.clone(),
                    },
                },
            )
            .await;

        safety.add_rule(SafetyRule {
            name: format!("mcp_approve_{}", action_name),
            description: mcp_safety_rule_description(server, &resource.name, true),
            trigger: RuleTrigger::Action { name: action_name },
            condition: None,
            action: mcp_safety_rule_action(server, &resource.name, true),
            verified: true,
        });
    }

    Ok(())
}

fn mcp_safety_rule_action(
    _server: &McpServerConfig,
    _item_name: &str,
    _is_resource: bool,
) -> RuleAction {
    RuleAction::RequireApproval
}

fn mcp_safety_rule_description(
    server: &McpServerConfig,
    _item_name: &str,
    is_resource: bool,
) -> String {
    let action = mcp_safety_rule_action(server, _item_name, is_resource);
    let kind = if is_resource { "resource" } else { "tool" };
    match action {
        RuleAction::LogAndAllow => {
            format!(
                "Configured MCP {} is trusted and logged without approval.",
                kind
            )
        }
        RuleAction::RequireApproval => format!("Configured MCP {} requires approval.", kind),
        RuleAction::Allow => format!("Configured MCP {} is allowed.", kind),
        RuleAction::Block { .. } => format!("Configured MCP {} is blocked.", kind),
        RuleAction::Delay { .. } => format!("Configured MCP {} is delayed.", kind),
    }
}

fn unique_action_name(
    server_id: &str,
    kind: &str,
    name: &str,
    used: &mut HashMap<String, usize>,
) -> String {
    let digest = Sha256::digest(server_id.as_bytes());
    let server_tag = hex::encode(&digest[..6]);
    let base = format!("mcp_{}_{}_{}", server_tag, kind, normalize_segment(name));
    let mut candidate = enforce_action_length(&base, name);
    if let Some(count) = used.get_mut(&candidate) {
        *count += 1;
        candidate = enforce_action_length(&format!("{}_{}", candidate, count), name);
    } else {
        used.insert(candidate.clone(), 1);
    }
    candidate
}

fn normalize_segment(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "tool".to_string()
    } else {
        out
    }
}

fn enforce_action_length(base: &str, seed: &str) -> String {
    const MAX_LEN: usize = 64;
    if base.len() <= MAX_LEN {
        return base.to_string();
    }
    let hash = blake3::hash(seed.as_bytes()).to_hex();
    let suffix = &hash[..6];
    let mut trimmed: String = base.chars().take(MAX_LEN - 7).collect();
    trimmed.push('_');
    trimmed.push_str(suffix);
    trimmed
}

fn format_mcp_result(result: &Value) -> String {
    let is_error = result
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if let Some(items) = result.get("content").and_then(|v| v.as_array()) {
        let mut parts = Vec::new();
        for item in items {
            if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                parts.push(text.to_string());
                continue;
            }
            if let Some(mime) = item.get("mimeType").and_then(|v| v.as_str()) {
                parts.push(format!("[MCP_CONTENT {}]", mime));
                continue;
            }
            parts.push(item.to_string());
        }
        let combined = parts.join("\n");
        let formatted = if is_error {
            format!("MCP Error:\n{}", combined)
        } else {
            combined
        };
        return crate::security::sanitize_untrusted_output("mcp", &formatted);
    }

    if let Some(contents) = result.get("contents").and_then(|v| v.as_array()) {
        let mut parts = Vec::new();
        for item in contents {
            if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                parts.push(text.to_string());
            } else {
                parts.push(item.to_string());
            }
        }
        let combined = parts.join("\n");
        let formatted = if is_error {
            format!("MCP Error:\n{}", combined)
        } else {
            combined
        };
        return crate::security::sanitize_untrusted_output("mcp", &formatted);
    }

    if let Some(text) = result.get("text").and_then(|v| v.as_str()) {
        let formatted = if is_error {
            format!("MCP Error:\n{}", text)
        } else {
            text.to_string()
        };
        return crate::security::sanitize_untrusted_output("mcp", &formatted);
    }

    let fallback = serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string());
    let formatted = if is_error {
        format!("MCP Error:\n{}", fallback)
    } else {
        fallback
    };
    crate::security::sanitize_untrusted_output("mcp", &formatted)
}
