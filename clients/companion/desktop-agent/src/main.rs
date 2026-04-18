use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, ValueEnum};
use futures_util::{Sink, SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::fs;
use tokio::process::Command;
use tokio::time::{interval, timeout, MissedTickBehavior};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest,
    http::header::{HeaderName, HeaderValue, AUTHORIZATION},
    Message,
};

#[derive(Debug, Parser)]
#[command(name = "agentark-companion-desktop-agent")]
#[command(about = "AgentArk companion client for desktops, servers, and custom headless devices")]
struct Args {
    #[arg(long, default_value = "ws://127.0.0.1:8990/companion/ws")]
    ws_url: String,

    #[arg(long)]
    session_id: Option<String>,

    #[arg(long)]
    code: Option<String>,

    #[arg(long)]
    device_id: Option<String>,

    #[arg(long)]
    token: Option<String>,

    #[arg(long)]
    device_public_key: Option<String>,

    #[arg(long, default_value = "agentark-companion-identity.json")]
    identity_file: PathBuf,

    #[arg(long, value_enum, default_value = "desktop")]
    profile: Profile,

    #[arg(long = "capability")]
    capabilities: Vec<String>,

    #[arg(long, default_value = "AgentArk desktop companion")]
    name: String,

    #[arg(long = "metadata")]
    metadata: Vec<String>,

    #[arg(long)]
    root: Option<PathBuf>,

    #[arg(long)]
    allow_system_run: bool,

    #[arg(long, default_value_t = 30)]
    pulse_secs: u64,

    #[arg(long, default_value_t = 30)]
    command_timeout_secs: u64,
}

#[derive(Clone, Debug, ValueEnum)]
enum Profile {
    Desktop,
    HomeServer,
    RaspberryPi,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Identity {
    device_id: String,
    token: String,
}

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    message_type: String,
    #[serde(default)]
    result: Option<PairingResult>,
    #[serde(default)]
    command: Option<CommandRecord>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PairingResult {
    status: String,
    message: String,
    #[serde(default)]
    device: Option<DeviceRecord>,
    #[serde(default)]
    device_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceRecord {
    id: String,
}

#[derive(Debug, Deserialize)]
struct CommandRecord {
    id: String,
    capability: String,
    action: String,
    #[serde(default)]
    arguments: Value,
}

#[derive(Debug)]
struct CommandOutcome {
    success: bool,
    preview: Option<String>,
    error: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let capabilities = capabilities_for(&args)?;

    let identity = if args.session_id.is_some() || args.code.is_some() {
        pair_device(&args, &capabilities).await?
    } else {
        load_identity(&args).await?
            .ok_or_else(|| anyhow!("provide --session-id/--code to pair or --device-id/--token to authenticate"))?
    };

    run_authenticated(&args, &capabilities, identity).await
}

async fn pair_device(args: &Args, capabilities: &[String]) -> Result<Identity> {
    let session_id = args
        .session_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("--session-id is required for pairing"))?;
    let code = args
        .code
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("--code is required for pairing"))?;
    let (ws_stream, _) = connect_async(&args.ws_url)
        .await
        .with_context(|| format!("failed to connect to {}", args.ws_url))?;
    let (mut write, mut read) = ws_stream.split();
    send_pairing_claim(&mut write, args, session_id, code).await?;

    while let Some(message) = read.next().await {
        let raw = message_to_text(message?)?;
        if raw.is_empty() {
            continue;
        }
        let envelope: Envelope = serde_json::from_str(&raw)
            .with_context(|| format!("failed to decode companion message: {raw}"))?;
        match envelope.message_type.as_str() {
            "hello" => continue,
            "pairing_claim_result" => {
                let Some(result) = envelope.result else {
                    continue;
                };
                println!("{}", result.message);
                if let (Some(device), Some(token)) = (result.device, result.device_token) {
                    let identity = Identity {
                        device_id: device.id,
                        token,
                    };
                    save_identity(&args.identity_file, &identity).await?;
                    return Ok(identity);
                }
                if result.status == "claimed" || result.status == "approved" {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    send_pairing_claim(&mut write, args, session_id, code).await?;
                }
            }
            "error" => bail!(envelope.error.unwrap_or_else(|| "companion pairing failed".to_string())),
            other => println!("ignored companion message during pairing: {other}"),
        }
    }

    bail!(
        "pairing socket closed before token was issued; configured capabilities were {:?}",
        capabilities
    )
}

async fn run_authenticated(args: &Args, capabilities: &[String], identity: Identity) -> Result<()> {
    let mut request = args
        .ws_url
        .as_str()
        .into_client_request()
        .context("failed to build companion WebSocket request")?;
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", identity.token))
            .context("invalid companion token header")?,
    );
    request.headers_mut().insert(
        HeaderName::from_static("x-agentark-companion-device"),
        HeaderValue::from_str(&identity.device_id).context("invalid companion device id header")?,
    );
    let (ws_stream, _) = connect_async(request)
        .await
        .with_context(|| format!("failed to connect to {}", args.ws_url))?;
    let (mut write, mut read) = ws_stream.split();

    let mut pulse = interval(Duration::from_secs(args.pulse_secs.max(5)));
    pulse.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = pulse.tick() => {
                send_pulse(&mut write, args, capabilities).await?;
            }
            message = read.next() => {
                let Some(message) = message else {
                    bail!("companion socket closed");
                };
                let raw = message_to_text(message?)?;
                if raw.is_empty() {
                    continue;
                }
                let envelope: Envelope = serde_json::from_str(&raw)
                    .with_context(|| format!("failed to decode companion message: {raw}"))?;
                match envelope.message_type.as_str() {
                    "hello" => {}
                    "auth_ok" => {
                        send_pulse(&mut write, args, capabilities).await?;
                    }
                    "pulse_ok" => {}
                    "command_result_ok" => {
                        send_pulse(&mut write, args, capabilities).await?;
                    }
                    "command_dispatch" => {
                        if let Some(command) = envelope.command {
                            let outcome = handle_command(args, capabilities, &command).await;
                            send_json(
                                &mut write,
                                json!({
                                    "type": "command_result",
                                    "command_id": command.id,
                                    "success": outcome.success,
                                    "result_preview": outcome.preview,
                                    "error": outcome.error,
                                }),
                            )
                            .await?;
                        }
                    }
                    "auth_error" | "error" => {
                        bail!(envelope.error.unwrap_or_else(|| "companion server returned an error".to_string()));
                    }
                    other => println!("ignored companion message: {other}"),
                }
            }
        }
    }
}

async fn handle_command(args: &Args, capabilities: &[String], command: &CommandRecord) -> CommandOutcome {
    if !capabilities.iter().any(|capability| capability == &command.capability) {
        return CommandOutcome::failure("Capability is not enabled on this companion agent.");
    }

    let result = match command.capability.as_str() {
        "notifications" | "approval_prompt" => handle_notification(command).await,
        "file_read" => handle_file_read(args, command).await,
        "file_write" => handle_file_write(args, command).await,
        "system_run" => handle_system_run(args, command).await,
        _ => Err(anyhow!(
            "No local adapter is installed for capability '{}'",
            command.capability
        )),
    };

    match result {
        Ok(preview) => CommandOutcome {
            success: true,
            preview: Some(limit_preview(&preview, 1800)),
            error: None,
        },
        Err(error) => CommandOutcome::failure(error.to_string()),
    }
}

async fn handle_notification(command: &CommandRecord) -> Result<String> {
    println!(
        "AgentArk companion notification: action={} arguments={}",
        command.action, command.arguments
    );
    Ok("Notification delivered to the companion console.".to_string())
}

async fn handle_file_read(args: &Args, command: &CommandRecord) -> Result<String> {
    let path = json_string(&command.arguments, "path")?;
    let path = scoped_path(&args.root, &path)?;
    let bytes = fs::read(&path)
        .await
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

async fn handle_file_write(args: &Args, command: &CommandRecord) -> Result<String> {
    let path = json_string(&command.arguments, "path")?;
    let content = json_string(&command.arguments, "content")?;
    let path = scoped_path(&args.root, &path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(&path, content.as_bytes())
        .await
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(format!("Wrote {} bytes to {}", content.len(), path.display()))
}

async fn handle_system_run(args: &Args, command: &CommandRecord) -> Result<String> {
    if !args.allow_system_run {
        bail!("system_run adapter is disabled; start with --allow-system-run to enable it");
    }
    let program = json_string(&command.arguments, "program")?;
    let mut process = Command::new(program);
    if let Some(values) = command.arguments.get("args").and_then(Value::as_array) {
        let mut command_args = Vec::new();
        for value in values {
            command_args.push(
                value
                    .as_str()
                    .ok_or_else(|| anyhow!("system_run args must be strings"))?
                    .to_string(),
            );
        }
        process.args(command_args);
    }
    if let Some(cwd) = command.arguments.get("cwd").and_then(Value::as_str) {
        process.current_dir(scoped_path(&args.root, cwd)?);
    }
    process.stdout(Stdio::piped()).stderr(Stdio::piped());
    let timeout_secs = command
        .arguments
        .get("timeout_secs")
        .and_then(Value::as_u64)
        .unwrap_or(args.command_timeout_secs)
        .clamp(1, 120);
    let output = timeout(Duration::from_secs(timeout_secs), process.output())
        .await
        .context("system_run timed out")?
        .context("failed to execute system_run program")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(format!(
        "exit={} stdout={} stderr={}",
        output.status.code().unwrap_or(-1),
        limit_preview(&stdout, 900),
        limit_preview(&stderr, 900)
    ))
}

async fn send_pairing_claim<S>(sink: &mut S, args: &Args, session_id: &str, code: &str) -> Result<()>
where
    S: Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    send_json(
        sink,
        json!({
            "type": "pairing_claim",
            "session_id": session_id,
            "code": code,
            "device_public_key": device_public_key(args),
            "metadata": metadata_for(args),
        }),
    )
    .await
}

async fn send_pulse<S>(sink: &mut S, args: &Args, capabilities: &[String]) -> Result<()>
where
    S: Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    send_json(
        sink,
        json!({
            "type": "pulse",
            "state": "online",
            "capabilities": capabilities,
            "metadata": metadata_for(args),
        }),
    )
    .await
}

async fn send_json<S>(sink: &mut S, payload: Value) -> Result<()>
where
    S: Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    sink.send(Message::Text(payload.to_string().into()))
        .await
        .context("failed to send companion WebSocket message")
}

fn message_to_text(message: Message) -> Result<String> {
    match message {
        Message::Text(text) => Ok(text.to_string()),
        Message::Close(frame) => bail!("companion socket closed: {:?}", frame),
        _ => Ok(String::new()),
    }
}

async fn load_identity(args: &Args) -> Result<Option<Identity>> {
    if let (Some(device_id), Some(token)) = (&args.device_id, &args.token) {
        return Ok(Some(Identity {
            device_id: device_id.clone(),
            token: token.clone(),
        }));
    }
    if !args.identity_file.exists() {
        return Ok(None);
    }
    let raw = fs::read(&args.identity_file)
        .await
        .with_context(|| format!("failed to read {}", args.identity_file.display()))?;
    Ok(Some(serde_json::from_slice(&raw)?))
}

async fn save_identity(path: &Path, identity: &Identity) -> Result<()> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent).await?;
    }
    let raw = serde_json::to_vec_pretty(identity)?;
    fs::write(path, raw)
        .await
        .with_context(|| format!("failed to write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(0o600);
        let _ = fs::set_permissions(path, permissions).await;
    }
    Ok(())
}

fn capabilities_for(args: &Args) -> Result<Vec<String>> {
    let raw: Vec<&str> = if args.capabilities.is_empty() {
        match args.profile {
            Profile::Desktop => vec![
                "notifications",
                "screen_capture",
                "browser_control",
                "file_read",
                "file_write",
                "system_run",
            ],
            Profile::HomeServer => vec![
                "notifications",
                "system_run",
                "lan_access",
                "file_read",
                "file_write",
            ],
            Profile::RaspberryPi => vec!["sensor_read", "smart_home", "lan_access", "system_run"],
            Profile::Custom => vec!["custom.example"],
        }
    } else {
        args.capabilities.iter().map(String::as_str).collect()
    };
    let mut out = Vec::new();
    for value in raw {
        let normalized = normalize_capability(value)?;
        if !out.contains(&normalized) {
            out.push(normalized);
        }
    }
    Ok(out)
}

fn normalize_capability(value: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty()
        || normalized.len() > 96
        || !normalized
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
    {
        bail!("invalid capability id '{}'", value);
    }
    Ok(normalized)
}

fn metadata_for(args: &Args) -> serde_json::Map<String, Value> {
    let mut metadata = serde_json::Map::new();
    metadata.insert("client".to_string(), json!("AgentArk desktop companion"));
    metadata.insert("platform".to_string(), json!(profile_id(&args.profile)));
    metadata.insert("name".to_string(), json!(&args.name));
    for item in &args.metadata {
        if let Some((key, value)) = item.split_once('=') {
            let key = key.trim();
            if !key.is_empty() {
                metadata.insert(key.to_string(), json!(value.trim()));
            }
        }
    }
    metadata
}

fn device_public_key(args: &Args) -> String {
    args.device_public_key.clone().unwrap_or_else(|| {
        format!(
            "desktop:{}:{}",
            profile_id(&args.profile),
            args.name.trim().to_ascii_lowercase()
        )
    })
}

fn profile_id(profile: &Profile) -> &'static str {
    match profile {
        Profile::Desktop => "desktop",
        Profile::HomeServer => "home_server",
        Profile::RaspberryPi => "raspberry_pi",
        Profile::Custom => "custom",
    }
}

fn json_string(arguments: &Value, key: &str) -> Result<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("arguments.{} must be a string", key))
}

fn scoped_path(root: &Option<PathBuf>, requested: &str) -> Result<PathBuf> {
    let root = root
        .as_ref()
        .ok_or_else(|| anyhow!("--root is required for scoped file paths"))?;
    let requested_path = Path::new(requested);
    if requested_path.is_absolute() {
        bail!("scoped paths must be relative to --root");
    }
    for component in requested_path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => bail!("scoped paths cannot include parent traversal or path prefixes"),
        }
    }
    Ok(root.join(requested_path))
}

fn limit_preview(value: &str, max: usize) -> String {
    value.trim().chars().take(max).collect()
}

impl CommandOutcome {
    fn failure(error: impl Into<String>) -> Self {
        Self {
            success: false,
            preview: None,
            error: Some(error.into()),
        }
    }
}
