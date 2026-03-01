//! SSH remote execution integration
//!
//! Allows the agent to execute commands on configured remote servers.
//! Private keys are stored encrypted via SecureConfigManager.

use anyhow::{anyhow, Result};
use std::path::Path;
use std::sync::Arc;

const SSH_CONNECTIONS_KEY: &str = "ssh_connections";
const SSH_KEYS_KEY: &str = "ssh_keys";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SshConnection {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub key_name: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct SshKeyStore {
    /// Map of key_name -> PEM-encoded private key content
    pub keys: std::collections::HashMap<String, String>,
}

fn load_connections(config_dir: &Path) -> Result<Vec<SshConnection>> {
    let manager = crate::core::config::SecureConfigManager::new(config_dir)?;
    match manager.get_custom_secret(SSH_CONNECTIONS_KEY)? {
        Some(payload) => Ok(serde_json::from_str(&payload)?),
        None => Ok(Vec::new()),
    }
}

fn save_connections(config_dir: &Path, connections: &[SshConnection]) -> Result<()> {
    let manager = crate::core::config::SecureConfigManager::new(config_dir)?;
    manager.set_custom_secret(
        SSH_CONNECTIONS_KEY,
        Some(serde_json::to_string(connections)?),
    )?;
    Ok(())
}

fn load_keys(config_dir: &Path) -> Result<SshKeyStore> {
    let manager = crate::core::config::SecureConfigManager::new(config_dir)?;
    match manager.get_custom_secret(SSH_KEYS_KEY)? {
        Some(payload) => Ok(serde_json::from_str(&payload)?),
        None => Ok(SshKeyStore::default()),
    }
}

fn save_keys(config_dir: &Path, store: &SshKeyStore) -> Result<()> {
    let manager = crate::core::config::SecureConfigManager::new(config_dir)?;
    manager.set_custom_secret(SSH_KEYS_KEY, Some(serde_json::to_string(store)?))?;
    Ok(())
}

/// List available SSH connections
pub async fn ssh_list_connections(config_dir: &Path) -> Result<String> {
    let connections = load_connections(config_dir)?;
    if connections.is_empty() {
        return Ok(
            "No SSH connections configured. Add connections in Settings > MCP Servers > SSH Access."
                .to_string(),
        );
    }

    let mut output = format!("{} SSH connection(s):\n", connections.len());
    for c in &connections {
        output.push_str(&format!(
            "- {} ({}@{}:{}, key: {})\n",
            c.name, c.username, c.host, c.port, c.key_name
        ));
    }
    Ok(output)
}

/// Execute a command on a remote server via SSH
pub async fn ssh_execute(config_dir: &Path, arguments: &serde_json::Value) -> Result<String> {
    let conn_name = arguments
        .get("connection")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing 'connection' name"))?;
    let command = arguments
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing 'command' to execute"))?;

    let connections = load_connections(config_dir)?;
    let conn = connections
        .iter()
        .find(|c| c.name == conn_name)
        .ok_or_else(|| {
            anyhow!(
                "SSH connection '{}' not found. Available: {}",
                conn_name,
                connections
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

    let key_store = load_keys(config_dir)?;
    let key_pem = key_store
        .keys
        .get(&conn.key_name)
        .ok_or_else(|| anyhow!("SSH key '{}' not found", conn.key_name))?;

    // Parse the private key
    let key_pair = russh::keys::decode_secret_key(key_pem, None)
        .map_err(|e| anyhow!("Failed to parse SSH key '{}': {}", conn.key_name, e))?;
    let key_pair = russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key_pair), None);

    // Connect and execute
    let config = russh::client::Config::default();
    let config = Arc::new(config);

    let mut session = russh::client::connect(config, (conn.host.as_str(), conn.port), Handler)
        .await
        .map_err(|e| {
            anyhow!(
                "SSH connection to {}:{} failed: {}",
                conn.host,
                conn.port,
                e
            )
        })?;

    // Authenticate
    let auth_result = session
        .authenticate_publickey(&conn.username, key_pair)
        .await
        .map_err(|e| anyhow!("SSH auth failed for {}@{}: {}", conn.username, conn.host, e))?;

    if !auth_result.success() {
        return Err(anyhow!(
            "SSH authentication rejected for {}@{}",
            conn.username,
            conn.host
        ));
    }

    // Open channel and execute
    let mut channel = session
        .channel_open_session()
        .await
        .map_err(|e| anyhow!("Failed to open SSH channel: {}", e))?;

    channel
        .exec(true, command)
        .await
        .map_err(|e| anyhow!("Failed to execute command: {}", e))?;

    // Collect output
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code = None;

    loop {
        let msg = channel.wait().await;
        match msg {
            Some(russh::ChannelMsg::Data { data }) => {
                stdout.extend_from_slice(&data);
            }
            Some(russh::ChannelMsg::ExtendedData { data, ext }) => {
                if ext == 1 {
                    // stderr
                    stderr.extend_from_slice(&data);
                }
            }
            Some(russh::ChannelMsg::ExitStatus { exit_status }) => {
                exit_code = Some(exit_status);
            }
            Some(russh::ChannelMsg::Eof) | None => break,
            _ => {}
        }
    }

    let stdout_str = String::from_utf8_lossy(&stdout);
    let stderr_str = String::from_utf8_lossy(&stderr);
    let code = exit_code.unwrap_or(0);

    let mut result = format!("[{}@{} exit:{}]\n", conn.username, conn.host, code);
    if !stdout_str.is_empty() {
        result.push_str(&stdout_str);
    }
    if !stderr_str.is_empty() {
        if !stdout_str.is_empty() {
            result.push('\n');
        }
        result.push_str(&format!("[stderr] {}", stderr_str));
    }

    session
        .disconnect(russh::Disconnect::ByApplication, "done", "en")
        .await
        .ok();

    Ok(result)
}

/// Add an SSH connection
pub fn add_connection(config_dir: &Path, conn: SshConnection) -> Result<()> {
    let mut connections = load_connections(config_dir)?;
    connections.retain(|c| c.name != conn.name);
    connections.push(conn);
    save_connections(config_dir, &connections)
}

/// Remove an SSH connection
pub fn remove_connection(config_dir: &Path, name: &str) -> Result<bool> {
    let mut connections = load_connections(config_dir)?;
    let before = connections.len();
    connections.retain(|c| c.name != name);
    save_connections(config_dir, &connections)?;
    Ok(before != connections.len())
}

/// Store an SSH private key (encrypted)
pub fn store_key(config_dir: &Path, name: &str, pem_content: &str) -> Result<()> {
    let mut store = load_keys(config_dir)?;
    store.keys.insert(name.to_string(), pem_content.to_string());
    save_keys(config_dir, &store)
}

/// Remove an SSH key
pub fn remove_key(config_dir: &Path, name: &str) -> Result<bool> {
    let mut store = load_keys(config_dir)?;
    let removed = store.keys.remove(name).is_some();
    save_keys(config_dir, &store)?;
    Ok(removed)
}

/// List key names (never returns key content)
pub fn list_key_names(config_dir: &Path) -> Result<Vec<String>> {
    let store = load_keys(config_dir)?;
    Ok(store.keys.keys().cloned().collect())
}

/// Minimal SSH client handler (accepts all host keys - user manages trust via connection config)
struct Handler;

impl russh::client::Handler for Handler {
    type Error = anyhow::Error;

    fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> impl std::future::Future<Output = std::result::Result<bool, Self::Error>> + Send {
        // Accept all host keys - trust is managed by the user configuring connections
        async { Ok(true) }
    }
}
