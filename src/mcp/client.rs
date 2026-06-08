//! MCP client for connecting to external servers (HTTP or stdio)

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::timeout;

use super::{McpResource, McpTool};
use crate::core::runtime::config::{McpServerConfig, McpTransportConfig};

#[derive(Debug, Clone)]
pub enum McpAuth {
    Bearer {
        header: String,
        token: String,
    },
    Basic {
        username: String,
        password: String,
    },
    Header {
        name: String,
        value: String,
    },
    Query {
        name: String,
        value: String,
    },
    Composite {
        headers: Vec<(String, String)>,
        query: Vec<(String, String)>,
        basic: Option<(String, String)>,
    },
}

#[derive(Debug, Serialize)]
struct RpcRequest {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Deserialize)]
struct RpcResponse {
    _jsonrpc: Option<String>,
    id: Option<Value>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    code: i64,
    message: String,
    #[serde(default)]
    _data: Option<Value>,
}

pub struct McpClient {
    transport: McpTransport,
    initialized: bool,
    next_id: u64,
    timeout: Duration,
    max_response_bytes: usize,
}

enum McpTransport {
    Http(HttpTransport),
    Stdio(Box<StdioTransport>),
}

struct HttpTransport {
    url: url::Url,
    client: reqwest::Client,
    auth: Option<McpAuth>,
}

struct StdioTransport {
    command: String,
    args: Vec<String>,
    working_dir: Option<std::path::PathBuf>,
    env: std::collections::HashMap<String, String>,
    session: Option<StdioSession>,
}

struct StdioSession {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    buf: Vec<u8>,
}

impl McpClient {
    pub fn new(
        config: &McpServerConfig,
        auth: Option<McpAuth>,
        env: std::collections::HashMap<String, String>,
    ) -> Result<Self> {
        let timeout = Duration::from_secs(config.timeout_secs);
        let max_response_bytes = config.max_response_bytes;
        let transport = match &config.transport {
            McpTransportConfig::Http { url } => {
                let url = url::Url::parse(url).map_err(|e| anyhow!("Invalid MCP URL: {}", e))?;
                let client = reqwest::Client::builder()
                    .timeout(timeout)
                    .redirect(reqwest::redirect::Policy::none())
                    .build()?;
                McpTransport::Http(HttpTransport { url, client, auth })
            }
            McpTransportConfig::Stdio {
                command,
                args,
                working_dir,
                ..
            } => McpTransport::Stdio(Box::new(StdioTransport {
                command: command.clone(),
                args: args.clone(),
                working_dir: working_dir.as_ref().map(std::path::PathBuf::from),
                env,
                session: None,
            })),
        };

        Ok(Self {
            transport,
            initialized: false,
            next_id: 1,
            timeout,
            max_response_bytes,
        })
    }

    async fn ensure_initialized(&mut self) -> Result<()> {
        if self.initialized {
            return Ok(());
        }

        let params = serde_json::json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {
                "tools": { "listChanged": false },
                "resources": { "subscribe": false, "listChanged": false }
            },
            "clientInfo": {
                "name": "agentark",
                "version": env!("CARGO_PKG_VERSION")
            }
        });

        let _ = self.request("initialize", params).await?;
        let _ = self
            .notify("notifications/initialized", serde_json::json!({}))
            .await;
        self.initialized = true;
        Ok(())
    }

    pub async fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        self.ensure_initialized().await?;
        let mut all_tools = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut params = serde_json::json!({});
            if let Some(ref c) = cursor {
                params["cursor"] = Value::String(c.clone());
            }
            let result = self.request("tools/list", params).await?;
            if let Some(Value::Array(arr)) = result.get("tools") {
                let page: Vec<McpTool> = serde_json::from_value(Value::Array(arr.clone()))?;
                all_tools.extend(page);
            }
            cursor = result
                .get("nextCursor")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if cursor.is_none() {
                break;
            }
        }
        Ok(all_tools)
    }

    pub async fn list_resources(&mut self) -> Result<Vec<McpResource>> {
        self.ensure_initialized().await?;
        let mut all_resources = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut params = serde_json::json!({});
            if let Some(ref c) = cursor {
                params["cursor"] = Value::String(c.clone());
            }
            let result = self.request("resources/list", params).await?;
            if let Some(Value::Array(arr)) = result.get("resources") {
                let page: Vec<McpResource> = serde_json::from_value(Value::Array(arr.clone()))?;
                all_resources.extend(page);
            }
            cursor = result
                .get("nextCursor")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if cursor.is_none() {
                break;
            }
        }
        Ok(all_resources)
    }

    pub async fn call_tool(&mut self, name: &str, arguments: &Value) -> Result<Value> {
        self.ensure_initialized().await?;
        self.request(
            "tools/call",
            serde_json::json!({
                "name": name,
                "arguments": arguments
            }),
        )
        .await
    }

    pub async fn read_resource(&mut self, uri: &str) -> Result<Value> {
        self.ensure_initialized().await?;
        self.request("resources/read", serde_json::json!({ "uri": uri }))
            .await
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = Value::Number(self.next_id.into());
        self.next_id += 1;
        let request = RpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(id.clone()),
            method: method.to_string(),
            params,
        };
        let response = self.send_request(&request).await?;
        match response.error {
            Some(err) => Err(anyhow!("MCP error {}: {}", err.code, err.message)),
            None => Ok(response.result.unwrap_or(Value::Null)),
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let request = RpcRequest {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: method.to_string(),
            params,
        };
        self.send_notification(&request).await?;
        Ok(())
    }

    async fn send_request(&mut self, request: &RpcRequest) -> Result<RpcResponse> {
        let fut = async {
            match &mut self.transport {
                McpTransport::Http(transport) => {
                    transport
                        .send_request(request, self.max_response_bytes)
                        .await
                }
                McpTransport::Stdio(transport) => {
                    transport
                        .send_request(request, self.max_response_bytes)
                        .await
                }
            }
        };

        match timeout(self.timeout, fut).await {
            Ok(res) => res,
            Err(_) => Err(anyhow!("MCP request timed out")),
        }
    }

    async fn send_notification(&mut self, request: &RpcRequest) -> Result<()> {
        let fut = async {
            match &mut self.transport {
                McpTransport::Http(transport) => transport.send_notification(request).await,
                McpTransport::Stdio(transport) => transport.send_notification(request).await,
            }
        };

        match timeout(self.timeout, fut).await {
            Ok(res) => res,
            Err(_) => Err(anyhow!("MCP notification timed out")),
        }
    }
}

impl HttpTransport {
    async fn send_request(
        &self,
        request: &RpcRequest,
        max_response_bytes: usize,
    ) -> Result<RpcResponse> {
        let mut url = self.url.clone();
        let mut req = self.client.post(url.clone());

        if let Some(auth) = &self.auth {
            match auth {
                McpAuth::Bearer { header, token } => {
                    req = req.header(header, format!("Bearer {}", token));
                }
                McpAuth::Basic { username, password } => {
                    req = req.basic_auth(username, Some(password));
                }
                McpAuth::Header { name, value } => {
                    req = req.header(name, value);
                }
                McpAuth::Query { name, value } => {
                    url.query_pairs_mut().append_pair(name, value);
                    req = self.client.post(url.clone());
                }
                McpAuth::Composite {
                    headers,
                    query,
                    basic,
                } => {
                    for (name, value) in query {
                        url.query_pairs_mut().append_pair(name, value);
                    }
                    req = self.client.post(url.clone());
                    if let Some((username, password)) = basic {
                        req = req.basic_auth(username, Some(password));
                    }
                    for (name, value) in headers {
                        req = req.header(name, value);
                    }
                }
            }
        }

        let response = req
            .header("Content-Type", "application/json")
            .header("MCP-Protocol-Version", "2025-03-26")
            .header("Accept", "application/json, text/event-stream")
            .json(request)
            .send()
            .await?;

        let status = response.status();
        // Check Content-Length header before downloading body (prevents memory exhaustion)
        if let Some(content_length) = response.content_length() {
            if content_length as usize > max_response_bytes {
                return Err(anyhow!(
                    "MCP response too large ({} bytes, limit {})",
                    content_length,
                    max_response_bytes
                ));
            }
        }
        let bytes = response.bytes().await?;
        if bytes.len() > max_response_bytes {
            return Err(anyhow!(
                "MCP response too large ({} bytes, limit {})",
                bytes.len(),
                max_response_bytes
            ));
        }
        if !status.is_success() {
            let body = String::from_utf8_lossy(&bytes);
            return Err(anyhow!("MCP HTTP error {}: {}", status.as_u16(), body));
        }

        let rpc: RpcResponse = serde_json::from_slice(&bytes)
            .map_err(|e| anyhow!("Failed to parse MCP response: {}", e))?;
        Ok(rpc)
    }

    async fn send_notification(&self, request: &RpcRequest) -> Result<()> {
        let mut url = self.url.clone();
        let mut req = self.client.post(url.clone());
        if let Some(auth) = &self.auth {
            match auth {
                McpAuth::Bearer { header, token } => {
                    req = req.header(header, format!("Bearer {}", token));
                }
                McpAuth::Basic { username, password } => {
                    req = req.basic_auth(username, Some(password));
                }
                McpAuth::Header { name, value } => {
                    req = req.header(name, value);
                }
                McpAuth::Query { name, value } => {
                    url.query_pairs_mut().append_pair(name, value);
                    req = self.client.post(url.clone());
                }
                McpAuth::Composite {
                    headers,
                    query,
                    basic,
                } => {
                    for (name, value) in query {
                        url.query_pairs_mut().append_pair(name, value);
                    }
                    req = self.client.post(url.clone());
                    if let Some((username, password)) = basic {
                        req = req.basic_auth(username, Some(password));
                    }
                    for (name, value) in headers {
                        req = req.header(name, value);
                    }
                }
            }
        }
        let _ = req
            .header("Content-Type", "application/json")
            .header("MCP-Protocol-Version", "2025-03-26")
            .header("Accept", "application/json, text/event-stream")
            .json(request)
            .send()
            .await?;
        Ok(())
    }
}

impl StdioTransport {
    async fn send_request(
        &mut self,
        request: &RpcRequest,
        max_response_bytes: usize,
    ) -> Result<RpcResponse> {
        self.ensure_session().await?;
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| anyhow!("MCP stdio not available"))?;
        session.send_request(request, max_response_bytes).await
    }

    async fn send_notification(&mut self, request: &RpcRequest) -> Result<()> {
        self.ensure_session().await?;
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| anyhow!("MCP stdio not available"))?;
        session.send_notification(request).await
    }

    async fn ensure_session(&mut self) -> Result<()> {
        let needs_spawn = match self.session.as_mut() {
            Some(sess) => matches!(sess.child.try_wait(), Ok(Some(_))),
            None => true,
        };

        if !needs_spawn {
            return Ok(());
        }

        let mut cmd = Command::new(&self.command);
        cmd.args(&self.args)
            .env_clear()
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(dir) = &self.working_dir {
            cmd.current_dir(dir);
        }
        if !self.env.is_empty() {
            cmd.envs(&self.env);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("Failed to start MCP stdio server: {}", e))?;

        // Drain stderr in background to prevent blocking and capture diagnostics
        if let Some(stderr) = child.stderr.take() {
            crate::spawn_logged!("src/mcp/client.rs:472", async move {
                use tokio::io::AsyncBufReadExt;
                let reader = tokio::io::BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(
                        target: "mcp_stdio_stderr",
                        "{}",
                        crate::security::redact_secret_input(&line).text
                    );
                }
            });
        }

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Failed to open MCP stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Failed to open MCP stdout"))?;
        self.session = Some(StdioSession {
            child,
            stdin,
            stdout,
            buf: Vec::new(),
        });
        Ok(())
    }
}

impl StdioSession {
    async fn send_request(
        &mut self,
        request: &RpcRequest,
        max_response_bytes: usize,
    ) -> Result<RpcResponse> {
        let json = serde_json::to_vec(request)?;
        // Use line-delimited JSON (newline-terminated) — compatible with both
        // LSP-style Content-Length servers and line-delimited servers.
        self.stdin.write_all(&json).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;

        let expected_id = request.id.clone();
        loop {
            let response = self.read_message(max_response_bytes).await?;
            if response.id == expected_id {
                return Ok(response);
            }
        }
    }

    async fn send_notification(&mut self, request: &RpcRequest) -> Result<()> {
        let json = serde_json::to_vec(request)?;
        self.stdin.write_all(&json).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn read_message(&mut self, max_response_bytes: usize) -> Result<RpcResponse> {
        loop {
            if let Some((msg, used)) = try_parse_message(&self.buf, max_response_bytes)? {
                self.buf.drain(0..used);
                return Ok(msg);
            }

            let mut tmp = vec![0u8; 4096];
            let n = self.stdout.read(&mut tmp).await?;
            if n == 0 {
                return Err(anyhow!("MCP stdio closed"));
            }
            self.buf.extend_from_slice(&tmp[..n]);
            if self.buf.len() > max_response_bytes * 2 {
                return Err(anyhow!("MCP stdio buffer exceeded limit"));
            }
        }
    }
}

fn try_parse_message(
    buf: &[u8],
    max_response_bytes: usize,
) -> Result<Option<(RpcResponse, usize)>> {
    if let Some(header_end) = find_header_end(buf) {
        let header_bytes = &buf[..header_end];
        let header_str = std::str::from_utf8(header_bytes)
            .map_err(|_| anyhow!("Invalid MCP header encoding"))?;
        let content_len = parse_content_length(header_str)
            .ok_or_else(|| anyhow!("Missing Content-Length header"))?;
        if content_len > max_response_bytes {
            return Err(anyhow!("MCP response too large ({} bytes)", content_len));
        }
        let body_start = header_end + 4;
        let body_end = body_start + content_len;
        if buf.len() < body_end {
            return Ok(None);
        }
        let body = &buf[body_start..body_end];
        let response: RpcResponse = serde_json::from_slice(body)
            .map_err(|e| anyhow!("Invalid MCP JSON response: {}", e))?;
        return Ok(Some((response, body_end)));
    }

    if let Some(line_end) = buf.iter().position(|b| *b == b'\n') {
        let line = &buf[..line_end];
        if line.len() > max_response_bytes {
            return Err(anyhow!("MCP response too large ({} bytes)", line.len()));
        }
        if line.is_empty() {
            return Ok(Some((
                RpcResponse {
                    _jsonrpc: None,
                    id: None,
                    result: None,
                    error: None,
                },
                line_end + 1,
            )));
        }
        let response: RpcResponse = serde_json::from_slice(line)
            .map_err(|e| anyhow!("Invalid MCP JSON response: {}", e))?;
        return Ok(Some((response, line_end + 1)));
    }

    Ok(None)
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_content_length(header: &str) -> Option<usize> {
    for line in header.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            let raw = rest.trim();
            if let Ok(len) = raw.parse::<usize>() {
                return Some(len);
            }
        }
    }
    None
}
