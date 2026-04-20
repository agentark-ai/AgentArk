//! SSH remote execution integration
//!
//! Allows the agent to execute commands on configured remote servers.
//! Private keys are stored encrypted via SecureConfigManager.

use anyhow::{anyhow, bail, Result};
use std::path::Path;
use std::sync::Arc;

const SSH_CONNECTIONS_KEY: &str = "ssh_connections";
const SSH_KEYS_KEY: &str = "ssh_keys";
const SSH_KNOWN_HOSTS_KEY: &str = "ssh_known_hosts";
const SSH_EXEC_TIMEOUT_SECS: u64 = 60;
const SSH_MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;
const MAX_SSH_AUDIT_COMMAND_CHARS: usize = 240;
const SSH_SUPPORTED_PRIVATE_KEY_MESSAGE: &str =
    "AgentArk accepts Ed25519 or ECDSA OpenSSH private keys only; RSA/id_rsa is not supported.";

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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct SshKnownHostStore {
    /// Map of "host:port" -> SHA256 fingerprint
    pub hosts: std::collections::HashMap<String, String>,
}

fn load_connections(config_dir: &Path) -> Result<Vec<SshConnection>> {
    let manager = crate::core::config::SecureConfigManager::new(config_dir)?;
    match manager.get_custom_secret(SSH_CONNECTIONS_KEY)? {
        Some(payload) => Ok(serde_json::from_str(&payload)?),
        None => Ok(Vec::new()),
    }
}

pub fn list_connections(config_dir: &Path) -> Result<Vec<SshConnection>> {
    load_connections(config_dir)
}

fn connection_is_allowed(allowed_names: Option<&[String]>, connection_name: &str) -> bool {
    match allowed_names {
        Some(allowed) => allowed
            .iter()
            .any(|value| value.trim() == connection_name.trim()),
        None => true,
    }
}

fn filter_connections_by_allowlist(
    connections: Vec<SshConnection>,
    allowed_names: Option<&[String]>,
) -> Vec<SshConnection> {
    connections
        .into_iter()
        .filter(|connection| connection_is_allowed(allowed_names, &connection.name))
        .collect()
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

fn load_known_hosts(config_dir: &Path) -> Result<SshKnownHostStore> {
    let manager = crate::core::config::SecureConfigManager::new(config_dir)?;
    match manager.get_custom_secret(SSH_KNOWN_HOSTS_KEY)? {
        Some(payload) => Ok(serde_json::from_str(&payload)?),
        None => Ok(SshKnownHostStore::default()),
    }
}

fn save_known_hosts(config_dir: &Path, store: &SshKnownHostStore) -> Result<()> {
    let manager = crate::core::config::SecureConfigManager::new(config_dir)?;
    manager.set_custom_secret(SSH_KNOWN_HOSTS_KEY, Some(serde_json::to_string(store)?))?;
    Ok(())
}

fn known_host_store_key(host: &str, port: u16) -> String {
    format!("{}:{}", host.trim().to_ascii_lowercase(), port)
}

fn server_key_fingerprint(server_public_key: &russh::keys::ssh_key::PublicKey) -> String {
    format!(
        "{}",
        server_public_key.fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
    )
}

fn decode_supported_private_key(
    name: &str,
    pem_content: &str,
) -> Result<russh::keys::ssh_key::PrivateKey> {
    let key = russh::keys::decode_secret_key(pem_content, None).map_err(|_| {
        anyhow!(
            "SSH key '{}' is not a supported private key. {}",
            name,
            SSH_SUPPORTED_PRIVATE_KEY_MESSAGE
        )
    })?;

    match key.algorithm() {
        russh::keys::ssh_key::Algorithm::Ed25519
        | russh::keys::ssh_key::Algorithm::Ecdsa { .. } => Ok(key),
        _ => bail!(
            "SSH key '{}' uses an unsupported algorithm. {}",
            name,
            SSH_SUPPORTED_PRIVATE_KEY_MESSAGE
        ),
    }
}

pub fn validate_private_key_pem(name: &str, pem_content: &str) -> Result<()> {
    decode_supported_private_key(name, pem_content).map(|_| ())
}

fn verify_or_learn_server_key(
    config_dir: &Path,
    host: &str,
    port: u16,
    server_public_key: &russh::keys::ssh_key::PublicKey,
) -> Result<()> {
    let key = known_host_store_key(host, port);
    let fingerprint = server_key_fingerprint(server_public_key);
    let mut store = load_known_hosts(config_dir)?;
    match store.hosts.get(&key) {
        Some(expected) if expected == &fingerprint => Ok(()),
        Some(expected) => {
            tracing::warn!(
                "SSH host key mismatch for {}: expected {}, got {}",
                key,
                expected,
                fingerprint
            );
            Err(anyhow!(
                "SSH host key mismatch for {} (expected {}, got {})",
                key,
                expected,
                fingerprint
            ))
        }
        None => {
            tracing::info!(
                "Learning SSH host key for {} with fingerprint {}",
                key,
                fingerprint
            );
            store.hosts.insert(key, fingerprint);
            save_known_hosts(config_dir, &store)
        }
    }
}

fn append_ssh_output(
    stdout: &mut Vec<u8>,
    stderr: &mut Vec<u8>,
    data: &[u8],
    stream_name: &str,
) -> Result<()> {
    let current = stdout.len().saturating_add(stderr.len());
    let remaining = SSH_MAX_OUTPUT_BYTES.saturating_sub(current);
    if data.len() > remaining {
        if remaining > 0 {
            if stream_name == "stderr" {
                stderr.extend_from_slice(&data[..remaining]);
            } else {
                stdout.extend_from_slice(&data[..remaining]);
            }
        }
        return Err(anyhow!(
            "SSH command output exceeded {} bytes while reading {}",
            SSH_MAX_OUTPUT_BYTES,
            stream_name
        ));
    }
    if stream_name == "stderr" {
        stderr.extend_from_slice(data);
    } else {
        stdout.extend_from_slice(data);
    }
    Ok(())
}

fn truncate_ssh_audit_command(command: &str) -> String {
    let normalized = command
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.chars().count() <= MAX_SSH_AUDIT_COMMAND_CHARS {
        normalized
    } else {
        let mut truncated = normalized
            .chars()
            .take(MAX_SSH_AUDIT_COMMAND_CHARS.saturating_sub(3))
            .collect::<String>();
        truncated.push_str("...");
        truncated
    }
}

/// List available SSH connections
pub async fn ssh_list_connections(config_dir: &Path) -> Result<String> {
    ssh_list_connections_scoped(config_dir, None).await
}

/// List available SSH connections, optionally restricted to an allowlist.
pub async fn ssh_list_connections_scoped(
    config_dir: &Path,
    allowed_names: Option<&[String]>,
) -> Result<String> {
    let connections = filter_connections_by_allowlist(load_connections(config_dir)?, allowed_names);
    if connections.is_empty() {
        return Ok(if allowed_names.is_some() {
            "No SSH connections are attached to this agent.".to_string()
        } else {
            "No SSH connections configured. Add connections in Settings > MCP Servers > SSH Access."
                .to_string()
        });
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
    ssh_execute_scoped(config_dir, arguments, None).await
}

/// Execute a command on a remote server via SSH, optionally restricted to an allowlist.
pub async fn ssh_execute_scoped(
    config_dir: &Path,
    arguments: &serde_json::Value,
    allowed_names: Option<&[String]>,
) -> Result<String> {
    let conn_name = arguments
        .get("connection")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing 'connection' name"))?;
    let command = arguments
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing 'command' to execute"))?;

    if !connection_is_allowed(allowed_names, conn_name) {
        return Err(anyhow!(
            "SSH connection '{}' is not attached to this agent.",
            conn_name
        ));
    }

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
    let key_pair = decode_supported_private_key(&conn.key_name, key_pem)?;
    let key_pair = russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key_pair), None);
    let command_audit = truncate_ssh_audit_command(command);
    let started_at = std::time::Instant::now();
    tracing::info!(
        ssh_connection = %conn.name,
        ssh_host = %conn.host,
        ssh_port = conn.port,
        ssh_user = %conn.username,
        ssh_command = %command_audit,
        "SSH command execution started"
    );

    // Connect and execute
    let config = russh::client::Config::default();
    let config = Arc::new(config);

    let handler = Handler {
        config_dir: config_dir.to_path_buf(),
        host: conn.host.clone(),
        port: conn.port,
    };
    let mut session = tokio::time::timeout(
        std::time::Duration::from_secs(SSH_EXEC_TIMEOUT_SECS),
        russh::client::connect(config, (conn.host.as_str(), conn.port), handler),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "SSH connection to {}:{} timed out after {}s",
            conn.host,
            conn.port,
            SSH_EXEC_TIMEOUT_SECS
        )
    })?
    .map_err(|e| {
        anyhow!(
            "SSH connection to {}:{} failed: {}",
            conn.host,
            conn.port,
            e
        )
    })?;

    // Authenticate
    let auth_result = tokio::time::timeout(
        std::time::Duration::from_secs(SSH_EXEC_TIMEOUT_SECS),
        session.authenticate_publickey(&conn.username, key_pair),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "SSH auth timed out for {}@{} after {}s",
            conn.username,
            conn.host,
            SSH_EXEC_TIMEOUT_SECS
        )
    })?
    .map_err(|e| anyhow!("SSH auth failed for {}@{}: {}", conn.username, conn.host, e))?;

    if !auth_result.success() {
        return Err(anyhow!(
            "SSH authentication rejected for {}@{}",
            conn.username,
            conn.host
        ));
    }

    // Open channel and execute
    let mut channel = tokio::time::timeout(
        std::time::Duration::from_secs(SSH_EXEC_TIMEOUT_SECS),
        session.channel_open_session(),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "Opening SSH channel timed out after {}s",
            SSH_EXEC_TIMEOUT_SECS
        )
    })?
    .map_err(|e| anyhow!("Failed to open SSH channel: {}", e))?;

    tokio::time::timeout(
        std::time::Duration::from_secs(SSH_EXEC_TIMEOUT_SECS),
        channel.exec(true, command),
    )
    .await
    .map_err(|_| anyhow!("SSH exec timed out after {}s", SSH_EXEC_TIMEOUT_SECS))?
    .map_err(|e| anyhow!("Failed to execute command: {}", e))?;

    // Collect output
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code = None;

    tokio::time::timeout(
        std::time::Duration::from_secs(SSH_EXEC_TIMEOUT_SECS),
        async {
            loop {
                let msg = channel.wait().await;
                match msg {
                    Some(russh::ChannelMsg::Data { data }) => {
                        append_ssh_output(&mut stdout, &mut stderr, &data, "stdout")?;
                    }
                    Some(russh::ChannelMsg::ExtendedData { data, ext }) => {
                        if ext == 1 {
                            append_ssh_output(&mut stdout, &mut stderr, &data, "stderr")?;
                        }
                    }
                    Some(russh::ChannelMsg::ExitStatus { exit_status }) => {
                        exit_code = Some(exit_status);
                    }
                    Some(russh::ChannelMsg::Eof) | None => break,
                    _ => {}
                }
            }
            Ok::<(), anyhow::Error>(())
        },
    )
    .await
    .map_err(|_| anyhow!("SSH command timed out after {}s", SSH_EXEC_TIMEOUT_SECS))??;

    let stdout_str = String::from_utf8_lossy(&stdout);
    let stderr_str = String::from_utf8_lossy(&stderr);
    let code = exit_code.unwrap_or(0);
    tracing::info!(
        ssh_connection = %conn.name,
        ssh_host = %conn.host,
        ssh_port = conn.port,
        ssh_user = %conn.username,
        ssh_command = %command_audit,
        ssh_exit_code = code,
        ssh_duration_ms = started_at.elapsed().as_millis() as u64,
        ssh_stderr_present = !stderr_str.trim().is_empty(),
        "SSH command execution finished"
    );

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
    validate_private_key_pem(name, pem_content)?;
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

/// SSH client handler with trust-on-first-use host key verification.
struct Handler {
    config_dir: std::path::PathBuf,
    host: String,
    port: u16,
}

impl russh::client::Handler for Handler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> std::result::Result<bool, Self::Error> {
        verify_or_learn_server_key(&self.config_dir, &self.host, self.port, server_public_key)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DeterministicTestRng {
        state: u64,
    }

    impl DeterministicTestRng {
        fn new(seed: u64) -> Self {
            Self { state: seed }
        }

        fn next_word(&mut self) -> u64 {
            self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
            let mut value = self.state;
            value = (value ^ (value >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            value = (value ^ (value >> 27)).wrapping_mul(0x94D049BB133111EB);
            value ^ (value >> 31)
        }
    }

    impl russh::keys::ssh_key::rand_core::TryRng for DeterministicTestRng {
        type Error = russh::keys::ssh_key::rand_core::Infallible;

        fn try_next_u32(&mut self) -> std::result::Result<u32, Self::Error> {
            Ok((self.next_word() >> 32) as u32)
        }

        fn try_next_u64(&mut self) -> std::result::Result<u64, Self::Error> {
            Ok(self.next_word())
        }

        fn try_fill_bytes(&mut self, dst: &mut [u8]) -> std::result::Result<(), Self::Error> {
            for chunk in dst.chunks_mut(8) {
                let bytes = self.next_word().to_le_bytes();
                chunk.copy_from_slice(&bytes[..chunk.len()]);
            }
            Ok(())
        }
    }

    impl russh::keys::ssh_key::rand_core::TryCryptoRng for DeterministicTestRng {}

    fn test_private_key_pem(algorithm: russh::keys::ssh_key::Algorithm, seed: u64) -> String {
        let mut rng = DeterministicTestRng::new(seed);
        let key = russh::keys::ssh_key::PrivateKey::random(&mut rng, algorithm).unwrap();
        key.to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .unwrap()
            .to_string()
    }

    #[test]
    fn filter_connections_by_allowlist_keeps_only_attached_names() {
        let filtered = filter_connections_by_allowlist(
            vec![
                SshConnection {
                    name: "prod".to_string(),
                    host: "prod.example.com".to_string(),
                    port: 22,
                    username: "root".to_string(),
                    key_name: "prod-key".to_string(),
                },
                SshConnection {
                    name: "staging".to_string(),
                    host: "staging.example.com".to_string(),
                    port: 22,
                    username: "root".to_string(),
                    key_name: "staging-key".to_string(),
                },
            ],
            Some(&["prod".to_string()]),
        );

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "prod");
    }

    #[test]
    fn verify_or_learn_server_key_trusts_first_use_and_reuses_fingerprint() {
        let temp = tempfile::tempdir().unwrap();
        let key = russh::keys::parse_public_key_base64(
            "AAAAC3NzaC1lZDI1NTE5AAAAILagOJFgwaMNhBWQINinKOXmqS4Gh5NgxgriXwdOoINJ",
        )
        .unwrap();

        verify_or_learn_server_key(temp.path(), "example.com", 22, &key).unwrap();
        verify_or_learn_server_key(temp.path(), "example.com", 22, &key).unwrap();

        let store = load_known_hosts(temp.path()).unwrap();
        assert_eq!(
            store
                .hosts
                .get(&known_host_store_key("example.com", 22))
                .cloned(),
            Some(server_key_fingerprint(&key))
        );
    }

    #[test]
    fn verify_or_learn_server_key_rejects_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let first = russh::keys::parse_public_key_base64(
            "AAAAC3NzaC1lZDI1NTE5AAAAILagOJFgwaMNhBWQINinKOXmqS4Gh5NgxgriXwdOoINJ",
        )
        .unwrap();
        let second = russh::keys::parse_public_key_base64(
            "AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBMxBTpMIGvo7CnordO7wP0QQRqpBwUjOLl4eMhfucfE1sjTYyK5wmTl1UqoSDS1PtRVTBdl+0+9pquFb46U7fwg=",
        )
        .unwrap();

        verify_or_learn_server_key(temp.path(), "example.com", 22, &first).unwrap();
        let error = verify_or_learn_server_key(temp.path(), "example.com", 22, &second)
            .unwrap_err()
            .to_string();

        assert!(error.contains("mismatch"));
        assert!(error.contains("example.com:22"));
    }

    #[test]
    fn validate_private_key_pem_accepts_ed25519_keys() {
        let pem = test_private_key_pem(russh::keys::ssh_key::Algorithm::Ed25519, 0xED25519);
        validate_private_key_pem("ed25519-key", &pem).unwrap();
    }

    #[test]
    fn validate_private_key_pem_accepts_ecdsa_keys() {
        let pem = test_private_key_pem(
            russh::keys::ssh_key::Algorithm::Ecdsa {
                curve: russh::keys::ssh_key::EcdsaCurve::NistP256,
            },
            0xEC_D5A,
        );
        validate_private_key_pem("ecdsa-key", &pem).unwrap();
    }

    #[test]
    fn validate_private_key_pem_rejects_invalid_keys_with_guidance() {
        let error = validate_private_key_pem("legacy-key", "not a private key")
            .unwrap_err()
            .to_string();

        assert!(error.contains("Ed25519 or ECDSA"));
        assert!(error.contains("RSA/id_rsa"));
    }
}
