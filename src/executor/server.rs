use crate::actions::app::{
    generate_access_key, launch_dynamic_runtime, parse_required_inputs,
    read_local_runtime_log_tail, resolve_required_env_values, runtime_preference_from_opt,
    AppRegistry, DynamicAppRegistration, DynamicRuntimeHandle, DynamicRuntimeLaunch,
    RuntimePreference,
};
use crate::executor::protocol::{
    AppActionResponse, AppDeployRequest, AppDeployResponse, AppLifecycleRequest,
    CodeExecuteRequest, CodeExecuteResponse, ExecutorStatusResponse, InternalServiceHealth,
    StackMemoryContainerStats, StackMemoryStatsResponse, StackUpdateRequest, StackUpdateResponse,
};
use crate::runtime::ActionRuntime;
use anyhow::{Context, Result};
use axum::{
    body::{to_bytes, Body},
    extract::{
        ws::{Message as AxumWsMessage, WebSocket, WebSocketUpgrade},
        FromRequestParts, Path, Request, State,
    },
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, delete, get, post},
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message as TungsteniteMessage},
};

const CONTROL_CONTAINER_NAME: &str = "agentark-control";
const EXECUTOR_CONTAINER_NAME: &str = "agentark-executor";
const WORKSPACE_CONTAINER_NAME: &str = "agentark-workspace";
const POSTGRES_CONTAINER_NAME: &str = "agentark-postgres";
const EMBEDDINGS_CONTAINER_NAME: &str = "agentark-embeddings";
const DEFAULT_STACK_UPDATER_IMAGE: &str = "docker:28-cli";
const STACK_UPDATE_SCRIPT: &str = r#"set -eu
if ! command -v git >/dev/null 2>&1; then
    apk add --no-cache git >/dev/null
fi
if [ ! -d .git ]; then
    echo "AgentArk source checkout is missing at /workspace." >&2
    exit 2
fi
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
    echo "AgentArk source checkout has tracked local changes. Resolve them before updating." >&2
    exit 2
fi
git fetch --tags --force origin
git checkout --force "$AGENTARK_RELEASE_TAG"
if [ ! -f .env ] && [ -f .env.example ]; then
    cp .env.example .env
fi
touch .env
upsert_env() {
    key="$1"
    value="$2"
    if grep -q "^${key}=" .env 2>/dev/null; then
        sed -i "s|^${key}=.*|${key}=${value}|" .env
    else
        printf "%s=%s\n" "$key" "$value" >> .env
    fi
}
upsert_env AGENTARK_IMAGE "${AGENTARK_IMAGE_REPOSITORY}:${AGENTARK_RELEASE_VERSION}"
upsert_env AGENTARK_RELEASE_REPO "$AGENTARK_RELEASE_REPO"
upsert_env AGENTARK_RELEASE_TAG "$AGENTARK_RELEASE_TAG"
docker compose pull
docker compose up -d
"#;

#[derive(Debug, Deserialize)]
struct DockerInspectContainer {
    #[serde(default, rename = "Mounts")]
    mounts: Vec<DockerInspectMount>,
}

#[derive(Debug, Deserialize)]
struct DockerInspectMount {
    #[serde(default, rename = "Source")]
    source: String,
    #[serde(default, rename = "Destination")]
    destination: String,
}

#[derive(Debug, Clone)]
pub struct ExecutorServiceConfig {
    pub bind_addr: String,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub token: Option<String>,
    pub workspace_base_url: Option<String>,
}

impl ExecutorServiceConfig {
    pub fn from_env_paths(config_dir: PathBuf, data_dir: PathBuf) -> Result<Self> {
        let token = crate::clients::load_or_create_internal_service_token(
            &config_dir,
            crate::clients::InternalServiceKind::Executor,
        )?;
        Ok(Self {
            bind_addr: std::env::var("AGENTARK_EXECUTOR_BIND")
                .unwrap_or_else(|_| "127.0.0.1:8991".to_string()),
            config_dir,
            data_dir,
            token: Some(token),
            workspace_base_url: std::env::var("AGENTARK_WORKSPACE_URL")
                .or_else(|_| std::env::var("AGENTARK_WORKSPACE_BASE_URL"))
                .ok()
                .or_else(|| Some("http://127.0.0.1:8992".to_string())),
        })
    }
}

fn validate_internal_service_token(
    token: Option<&str>,
    env_name: &str,
    service_name: &str,
) -> Result<()> {
    let Some(value) = token.map(str::trim).filter(|value| !value.is_empty()) else {
        anyhow::bail!(
            "{} requires {} to be set to a non-empty shared secret",
            service_name,
            env_name
        );
    };
    if value.eq_ignore_ascii_case("change-me") {
        anyhow::bail!(
            "{} requires {} to be changed from the insecure default placeholder",
            service_name,
            env_name
        );
    }
    Ok(())
}

async fn wait_for_executor_shutdown_signal() {
    match tokio::signal::ctrl_c().await {
        Ok(()) => tracing::info!("Executor service shutdown signal received"),
        Err(error) => tracing::warn!("Executor service shutdown signal failed: {}", error),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DockerRuntimePolicy {
    TrustedLocal,
    ProxyOnly,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DockerRuntimePolicyStatus {
    policy: DockerRuntimePolicy,
    transport: Option<String>,
    raw_socket_present: bool,
}

fn docker_runtime_policy_from_env() -> DockerRuntimePolicy {
    if let Ok(policy) = std::env::var("AGENTARK_DOCKER_ACCESS_POLICY") {
        match policy.trim().to_ascii_lowercase().as_str() {
            "disabled" | "off" | "none" => return DockerRuntimePolicy::Disabled,
            "proxy_only" | "proxy-only" | "proxy" => return DockerRuntimePolicy::ProxyOnly,
            "trusted_local" | "trusted-local" | "local" => {
                return DockerRuntimePolicy::TrustedLocal
            }
            _ => {}
        }
    }

    let deployment_mode = std::env::var("AGENTARK_DEPLOYMENT_MODE")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let runtime_isolation = std::env::var("AGENTARK_RUNTIME_ISOLATION")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if deployment_mode == "internet_facing"
        || deployment_mode == "internet-facing"
        || runtime_isolation == "hosted"
        || runtime_isolation == "shared"
    {
        DockerRuntimePolicy::ProxyOnly
    } else {
        DockerRuntimePolicy::TrustedLocal
    }
}

fn docker_host_is_raw_transport(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    normalized.starts_with("unix://") || normalized.starts_with("npipe://")
}

fn docker_host_is_proxy_transport(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    normalized.starts_with("tcp://")
        || normalized.starts_with("http://")
        || normalized.starts_with("https://")
}

fn evaluate_docker_runtime_policy(
    policy: DockerRuntimePolicy,
    docker_host: Option<&str>,
    raw_socket_present: bool,
) -> Result<DockerRuntimePolicyStatus> {
    let transport = docker_host
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let raw_transport = transport
        .as_deref()
        .is_some_and(docker_host_is_raw_transport)
        || raw_socket_present;
    match policy {
        DockerRuntimePolicy::TrustedLocal => Ok(DockerRuntimePolicyStatus {
            policy,
            transport,
            raw_socket_present,
        }),
        DockerRuntimePolicy::ProxyOnly => {
            if raw_transport {
                anyhow::bail!(
                    "Executor Docker policy forbids raw host Docker sockets in proxy-only mode"
                );
            }
            let Some(ref transport) = transport else {
                anyhow::bail!(
                    "Executor Docker policy requires DOCKER_HOST to point at a constrained Docker proxy"
                );
            };
            if !docker_host_is_proxy_transport(transport) {
                anyhow::bail!("Executor Docker policy requires a TCP/HTTP Docker proxy transport");
            }
            Ok(DockerRuntimePolicyStatus {
                policy,
                transport: Some(transport.clone()),
                raw_socket_present,
            })
        }
        DockerRuntimePolicy::Disabled => {
            if transport.is_some() || raw_socket_present {
                anyhow::bail!("Executor Docker policy is disabled but Docker access is configured");
            }
            Ok(DockerRuntimePolicyStatus {
                policy,
                transport,
                raw_socket_present,
            })
        }
    }
}

fn current_docker_runtime_policy_status() -> Result<DockerRuntimePolicyStatus> {
    let docker_host = std::env::var("DOCKER_HOST").ok();
    evaluate_docker_runtime_policy(
        docker_runtime_policy_from_env(),
        docker_host.as_deref(),
        std::path::Path::new("/var/run/docker.sock").exists(),
    )
}

fn validate_raw_docker_socket_operation(operation: &str) -> Result<()> {
    if docker_runtime_policy_from_env() != DockerRuntimePolicy::TrustedLocal {
        anyhow::bail!(
            "{} requires trusted-local Docker policy because it launches a helper with the host Docker socket",
            operation
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_docker_policy_rejects_raw_unix_socket() {
        let status = evaluate_docker_runtime_policy(
            DockerRuntimePolicy::ProxyOnly,
            Some("unix:///var/run/docker.sock"),
            true,
        );

        assert!(status.is_err());
    }

    #[test]
    fn hosted_docker_policy_accepts_proxy_without_raw_socket() {
        let status = evaluate_docker_runtime_policy(
            DockerRuntimePolicy::ProxyOnly,
            Some("tcp://docker-proxy:2375"),
            false,
        )
        .expect("proxy-only mode should accept an explicit Docker proxy");

        assert_eq!(status.policy, DockerRuntimePolicy::ProxyOnly);
        assert_eq!(status.transport.as_deref(), Some("tcp://docker-proxy:2375"));
    }

    #[test]
    fn trusted_local_docker_policy_allows_default_socket() {
        let status = evaluate_docker_runtime_policy(
            DockerRuntimePolicy::TrustedLocal,
            Some("unix:///var/run/docker.sock"),
            true,
        )
        .expect("trusted-local mode keeps local Docker available");

        assert_eq!(status.policy, DockerRuntimePolicy::TrustedLocal);
    }
}

#[derive(Clone)]
struct ExecutorState {
    config: ExecutorServiceConfig,
    registry: AppRegistry,
    runtime: Arc<ActionRuntime>,
    client: reqwest::Client,
}

#[derive(Debug, Clone)]
struct LoadedAppSpec {
    title: String,
    app_dir: PathBuf,
    access_guard_enabled: bool,
    access_key: String,
    expose_public: bool,
    is_static: bool,
    entry_command: Option<String>,
    install_command: Option<String>,
    build_command: Option<String>,
    runtime_image: Option<String>,
    runtime_preference: RuntimePreference,
    required_inputs: Vec<crate::actions::app::AppRequiredInput>,
    config_values: HashMap<String, String>,
}

pub async fn run_service(config: ExecutorServiceConfig) -> Result<()> {
    validate_internal_service_token(
        config.token.as_deref(),
        "AGENTARK_EXECUTOR_TOKEN",
        "Executor service",
    )?;
    if let Some(workspace_base_url) = config.workspace_base_url.as_deref() {
        crate::clients::validate_internal_service_base_url(
            workspace_base_url,
            "Workspace service",
        )?;
    }
    let docker_policy = current_docker_runtime_policy_status()?;
    tracing::info!(
        policy = ?docker_policy.policy,
        transport = ?docker_policy.transport,
        raw_socket_present = docker_policy.raw_socket_present,
        "Executor Docker runtime policy validated"
    );
    let registry = AppRegistry::with_paths(config.config_dir.clone(), config.data_dir.clone());
    let _boot_report = registry.reconcile_on_boot().await;
    registry.spawn_restore_from_disk(
        config.config_dir.clone(),
        config.data_dir.clone(),
        std::env::vars().collect(),
    );
    let mut runtime = ActionRuntime::new(&config.config_dir, &config.data_dir).await?;
    if let Some(storage) = crate::core::runtime::config::global_settings_storage() {
        runtime.set_storage(storage);
    }
    match crate::identity::IdentityManager::load_or_create(&config.data_dir).await {
        Ok(identity) => {
            match crate::security::ActionGuard::new(
                identity.signing_key(),
                identity.did(),
                &config.config_dir,
                &config.data_dir,
            )
            .await
            {
                Ok(guard) => {
                    tracing::info!("Executor action security guard initialized");
                    let guard =
                        match crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
                            &config.config_dir,
                            Some(&config.data_dir),
                        )
                        .and_then(|manager| manager.load())
                        {
                            Ok(agent_config) => {
                                match crate::core::LlmClient::new(&agent_config.llm) {
                                    Ok(llm) => guard.with_semantic_reviewer(llm),
                                    Err(error) => {
                                        tracing::warn!(
                                    "Executor semantic action reviewer unavailable: {} - user-added skills will remain blocked until a model is configured",
                                    error
                                );
                                        guard
                                    }
                                }
                            }
                            Err(error) => {
                                tracing::warn!(
                                "Executor semantic action reviewer unavailable: {} - user-added skills will remain blocked until settings can be loaded",
                                error
                            );
                                guard
                            }
                        };
                    runtime.set_action_guard(Arc::new(guard));
                }
                Err(error) => {
                    tracing::warn!(
                        "Failed to initialize executor action security guard: {} - user-added skills will remain blocked",
                        error
                    );
                }
            }
        }
        Err(error) => {
            tracing::warn!(
                "Failed to load executor identity: {} - user-added skills will remain blocked",
                error
            );
        }
    }
    let runtime = Arc::new(runtime);
    runtime.load_all_actions().await?;

    let state = ExecutorState {
        config: config.clone(),
        registry,
        runtime,
        client: reqwest::Client::new(),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/internal/v1/status", get(status))
        .route("/internal/v1/system/restart-stack", post(restart_stack))
        .route("/internal/v1/system/update-stack", post(update_stack))
        .route("/internal/v1/system/stack-memory", get(stack_memory))
        .route("/internal/v1/code/execute", post(code_execute))
        .route("/internal/v1/apps/deploy", post(app_deploy))
        .route("/internal/v1/apps/{app_id}/restart", post(app_restart))
        .route("/internal/v1/apps/{app_id}/stop", post(app_stop))
        .route("/internal/v1/apps/{app_id}", delete(app_delete))
        .route("/internal/v1/apps/{app_id}/status", get(app_status))
        .route("/internal/v1/apps/{app_id}/logs", get(app_logs))
        .route("/internal/v1/apps/{app_id}/proxy", any(proxy_app_root))
        .route(
            "/internal/v1/apps/{app_id}/proxy/{*path}",
            any(proxy_app_path),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.bind_addr)
        .await
        .with_context(|| format!("Failed to bind executor service at {}", config.bind_addr))?;
    tracing::info!("Executor service listening on {}", config.bind_addr);
    axum::serve(listener, app)
        .with_graceful_shutdown(wait_for_executor_shutdown_signal())
        .await
        .context("Executor service failed")
}

async fn health(State(state): State<ExecutorState>) -> impl IntoResponse {
    let restore = state.registry.restore_snapshot().await;
    let docker_policy = current_docker_runtime_policy_status()
        .map(|status| format!("{:?}", status.policy))
        .unwrap_or_else(|error| format!("error: {}", error));
    Json(InternalServiceHealth {
        service: "executor".to_string(),
        mode: "executor".to_string(),
        ok: true,
        details: BTreeMap::from([
            (
                "config_dir".to_string(),
                state.config.config_dir.display().to_string(),
            ),
            (
                "data_dir".to_string(),
                state.config.data_dir.display().to_string(),
            ),
            ("restore_active".to_string(), restore.active.to_string()),
            ("restore_pending".to_string(), restore.pending.to_string()),
            ("docker_policy".to_string(), docker_policy),
        ]),
    })
}

fn authorize_internal(headers: &HeaderMap, token: Option<&str>) -> Result<(), StatusCode> {
    let Some(expected) = token.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };
    let provided = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim);
    if provided.is_some_and(|value| {
        crate::security::constant_time_eq(value.as_bytes(), expected.as_bytes())
    }) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

async fn restart_stack(State(state): State<ExecutorState>, headers: HeaderMap) -> Response {
    if let Err(status) = authorize_internal(&headers, state.config.token.as_deref()) {
        return (
            status,
            Json(json!({
                "status": "error",
                "message": "Unauthorized"
            })),
        )
            .into_response();
    }

    if let Err(error) = validate_stack_restart_prerequisites().await {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "status": "error",
                "message": error.to_string()
            })),
        )
            .into_response();
    }

    crate::spawn_logged!("src/executor/server.rs:314", async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Err(error) = restart_agentark_stack_services().await {
            tracing::error!("Failed to restart AgentArk split-stack services: {}", error);
        }
    });

    (
        StatusCode::OK,
        Json(json!({
            "status": "restarting",
            "message": "AgentArk services are restarting.",
            "services": [
                CONTROL_CONTAINER_NAME,
                WORKSPACE_CONTAINER_NAME,
                EXECUTOR_CONTAINER_NAME
            ]
        })),
    )
        .into_response()
}

async fn update_stack(
    State(state): State<ExecutorState>,
    headers: HeaderMap,
    Json(request): Json<StackUpdateRequest>,
) -> Response {
    if let Err(status) = authorize_internal(&headers, state.config.token.as_deref()) {
        return (
            status,
            Json(StackUpdateResponse {
                status: "error".to_string(),
                message: "Unauthorized".to_string(),
                release_tag: None,
                release_version: None,
            }),
        )
            .into_response();
    }

    if let Err(error) = validate_stack_update_request(&request) {
        return (
            StatusCode::BAD_REQUEST,
            Json(StackUpdateResponse {
                status: "error".to_string(),
                message: error.to_string(),
                release_tag: None,
                release_version: None,
            }),
        )
            .into_response();
    }

    if let Err(error) = validate_stack_restart_prerequisites().await {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(StackUpdateResponse {
                status: "error".to_string(),
                message: error.to_string(),
                release_tag: None,
                release_version: None,
            }),
        )
            .into_response();
    }

    if let Err(error) = spawn_stack_update_job(&request).await {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(StackUpdateResponse {
                status: "error".to_string(),
                message: error.to_string(),
                release_tag: None,
                release_version: None,
            }),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(StackUpdateResponse {
            status: "updating".to_string(),
            message: format!(
                "Updating AgentArk to {} and restarting the stack.",
                request.release_tag
            ),
            release_tag: Some(request.release_tag),
            release_version: Some(request.release_version),
        }),
    )
        .into_response()
}

async fn validate_stack_restart_prerequisites() -> Result<()> {
    let output = tokio::process::Command::new("docker")
        .args([
            "inspect",
            CONTROL_CONTAINER_NAME,
            WORKSPACE_CONTAINER_NAME,
            EXECUTOR_CONTAINER_NAME,
        ])
        .output()
        .await
        .context("Failed to run docker inspect for AgentArk services")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    anyhow::bail!(
        "AgentArk split-stack restart is unavailable: {}",
        if stderr.is_empty() {
            "docker could not inspect the control, workspace, and executor containers".to_string()
        } else {
            stderr
        }
    );
}

fn stack_updater_image() -> String {
    std::env::var("AGENTARK_STACK_UPDATER_IMAGE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_STACK_UPDATER_IMAGE.to_string())
}

fn stack_workspace_root() -> String {
    std::env::var("AGENTARK_WORKSPACE_ROOT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/workspace/agentark".to_string())
}

fn validate_stack_update_request(request: &StackUpdateRequest) -> Result<()> {
    let release_tag = request.release_tag.trim();
    if release_tag.is_empty() {
        anyhow::bail!("Release tag is required.");
    }
    if request.release_version.trim().is_empty() {
        anyhow::bail!("Release version is required.");
    }
    if request.release_repo.trim().is_empty() || !request.release_repo.contains('/') {
        anyhow::bail!("Release repository must use owner/repo format.");
    }
    if request.image_repository.trim().is_empty() || !request.image_repository.contains('/') {
        anyhow::bail!("Image repository must be a fully qualified registry path.");
    }
    Ok(())
}

async fn resolve_stack_workspace_host_dir() -> Result<String> {
    let output = tokio::process::Command::new("docker")
        .args(["inspect", EXECUTOR_CONTAINER_NAME])
        .output()
        .await
        .context("Failed to inspect executor container mounts")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "AgentArk update is unavailable: {}",
            if stderr.is_empty() {
                "docker inspect could not read executor container metadata".to_string()
            } else {
                stderr
            }
        );
    }

    let containers: Vec<DockerInspectContainer> = serde_json::from_slice(&output.stdout)
        .context("Failed to decode executor container mount metadata")?;
    let workspace_root = stack_workspace_root();
    let mount = containers
        .into_iter()
        .flat_map(|container| container.mounts.into_iter())
        .find(|mount| mount.destination == workspace_root)
        .map(|mount| mount.source)
        .filter(|value| !value.trim().is_empty());
    match mount {
        Some(source) => Ok(source),
        None => anyhow::bail!(
            "AgentArk update is unavailable: executor does not expose a host workspace mount for {}",
            workspace_root
        ),
    }
}

async fn spawn_stack_update_job(request: &StackUpdateRequest) -> Result<()> {
    validate_raw_docker_socket_operation("AgentArk stack update")?;
    let host_workspace_dir = resolve_stack_workspace_host_dir().await?;
    let updater_name = format!(
        "agentark-stack-updater-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    );
    let output = tokio::process::Command::new("docker")
        .args(["run", "-d", "--rm", "--name", updater_name.as_str()])
        .args([
            "-e",
            &format!("AGENTARK_RELEASE_TAG={}", request.release_tag),
        ])
        .args([
            "-e",
            &format!("AGENTARK_RELEASE_VERSION={}", request.release_version),
        ])
        .args([
            "-e",
            &format!("AGENTARK_RELEASE_REPO={}", request.release_repo),
        ])
        .args([
            "-e",
            &format!("AGENTARK_IMAGE_REPOSITORY={}", request.image_repository),
        ])
        .args(["-v", "/var/run/docker.sock:/var/run/docker.sock"])
        .args(["-v", &format!("{}:/workspace", host_workspace_dir)])
        .args(["-w", "/workspace"])
        .arg(stack_updater_image())
        .args(["sh", "-lc", STACK_UPDATE_SCRIPT])
        .output()
        .await
        .context("Failed to start the AgentArk update job")?;
    if output.status.success() {
        tracing::info!(
            "Spawned AgentArk stack updater job {} for release {}",
            updater_name,
            request.release_tag
        );
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    anyhow::bail!(
        "Failed to start the AgentArk update job: {}",
        if stderr.is_empty() {
            "docker run returned an unknown error".to_string()
        } else {
            stderr
        }
    );
}

async fn restart_agentark_stack_services() -> Result<()> {
    restart_named_container(CONTROL_CONTAINER_NAME).await?;
    restart_named_container(WORKSPACE_CONTAINER_NAME).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    restart_current_executor_container().await?;
    Ok(())
}

async fn restart_named_container(container_name: &str) -> Result<()> {
    let output = tokio::process::Command::new("docker")
        .args(["restart", "-t", "10", container_name])
        .output()
        .await
        .with_context(|| format!("Failed to restart Docker container {}", container_name))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    anyhow::bail!(
        "Docker restart for {} failed: {}",
        container_name,
        if stderr.is_empty() {
            "unknown error".to_string()
        } else {
            stderr
        }
    );
}

async fn restart_current_executor_container() -> Result<()> {
    tokio::process::Command::new("docker")
        .args(["restart", "-t", "10", EXECUTOR_CONTAINER_NAME])
        .spawn()
        .with_context(|| {
            format!(
                "Failed to trigger Docker restart for {}",
                EXECUTOR_CONTAINER_NAME
            )
        })?;
    Ok(())
}

async fn status(State(state): State<ExecutorState>, headers: HeaderMap) -> Response {
    if let Err(status) = authorize_internal(&headers, state.config.token.as_deref()) {
        return (
            status,
            Json(json!({
                "service": "executor",
                "mode": "executor",
                "status": "error",
                "error": "Unauthorized"
            })),
        )
            .into_response();
    }

    Json(ExecutorStatusResponse {
        service: "executor".to_string(),
        mode: "executor".to_string(),
        config_dir: state.config.config_dir.clone(),
        data_dir: state.config.data_dir.clone(),
        workspace_base_url: state.config.workspace_base_url.clone(),
        token_configured: state.config.token.is_some(),
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
struct DockerStatsRow {
    #[serde(default, rename = "Name")]
    name: String,
    #[serde(default, rename = "MemUsage")]
    memory_usage: String,
}

async fn stack_memory(State(state): State<ExecutorState>, headers: HeaderMap) -> Response {
    if let Err(status) = authorize_internal(&headers, state.config.token.as_deref()) {
        return (
            status,
            Json(json!({
                "status": "error",
                "source": "docker_stats",
                "message": "Unauthorized"
            })),
        )
            .into_response();
    }

    match collect_stack_memory_stats().await {
        Ok(stats) => (StatusCode::OK, Json(stats)).into_response(),
        Err(error) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(StackMemoryStatsResponse {
                status: "error".to_string(),
                source: "docker_stats".to_string(),
                memory_used_bytes: 0,
                memory_total_bytes: None,
                memory_pressure_percent: None,
                container_count: 0,
                containers: Vec::new(),
                sampled_at: chrono::Utc::now().to_rfc3339(),
                message: Some(error.to_string()),
            }),
        )
            .into_response(),
    }
}

async fn collect_stack_memory_stats() -> Result<StackMemoryStatsResponse> {
    let container_names = stack_memory_container_names();
    let mut command = tokio::process::Command::new("docker");
    command.args(["stats", "--no-stream", "--format", "{{json .}}"]);

    let output = tokio::time::timeout(Duration::from_secs(5), command.output())
        .await
        .context("Timed out running docker stats for AgentArk stack memory")?
        .context("Failed to run docker stats for AgentArk stack memory")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "docker stats failed for AgentArk stack memory: {}",
            if stderr.is_empty() {
                format!("exit status {}", output.status)
            } else {
                stderr
            }
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut containers = Vec::new();
    let mut total_used = 0_u64;
    let mut memory_total = None::<u64>;
    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let row: DockerStatsRow =
            serde_json::from_str(line).context("Failed to decode docker stats JSON")?;
        if !container_names.iter().any(|name| name == &row.name) {
            continue;
        }
        let Some((used, limit)) = parse_docker_memory_usage(&row.memory_usage) else {
            continue;
        };
        total_used = total_used.saturating_add(used);
        if let Some(limit) = limit {
            memory_total = Some(memory_total.map_or(limit, |current| current.max(limit)));
        }
        containers.push(StackMemoryContainerStats {
            name: row.name,
            memory_used_bytes: used,
            memory_limit_bytes: limit,
        });
    }

    if containers.is_empty() {
        anyhow::bail!(
            "docker stats returned no running AgentArk stack containers matching {}",
            container_names.join(", ")
        );
    }

    memory_total = read_docker_daemon_memory_total()
        .await
        .ok()
        .flatten()
        .or(memory_total);
    let memory_pressure_percent = memory_total
        .filter(|total| *total > 0)
        .map(|total| round_1((total_used as f64 / total as f64 * 100.0).clamp(0.0, 100.0)));

    Ok(StackMemoryStatsResponse {
        status: "ok".to_string(),
        source: "docker_stack".to_string(),
        memory_used_bytes: total_used,
        memory_total_bytes: memory_total,
        memory_pressure_percent,
        container_count: containers.len(),
        containers,
        sampled_at: chrono::Utc::now().to_rfc3339(),
        message: None,
    })
}

async fn read_docker_daemon_memory_total() -> Result<Option<u64>> {
    let output = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::process::Command::new("docker")
            .args(["info", "--format", "{{json .MemTotal}}"])
            .output(),
    )
    .await
    .context("Timed out reading Docker daemon memory total")?
    .context("Failed to read Docker daemon memory total")?;
    if !output.status.success() {
        return Ok(None);
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let trimmed = raw.trim().trim_matches('"');
    Ok(trimmed.parse::<u64>().ok())
}

fn stack_memory_container_names() -> Vec<String> {
    if let Ok(raw) = std::env::var("AGENTARK_STACK_MEMORY_CONTAINERS") {
        let names = raw
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if !names.is_empty() {
            return names;
        }
    }

    [
        CONTROL_CONTAINER_NAME,
        EXECUTOR_CONTAINER_NAME,
        WORKSPACE_CONTAINER_NAME,
        POSTGRES_CONTAINER_NAME,
        EMBEDDINGS_CONTAINER_NAME,
    ]
    .into_iter()
    .map(ToOwned::to_owned)
    .collect()
}

fn parse_docker_memory_usage(value: &str) -> Option<(u64, Option<u64>)> {
    let mut parts = value.split('/');
    let used = parse_docker_size_bytes(parts.next()?.trim())?;
    let limit = parts
        .next()
        .and_then(|part| parse_docker_size_bytes(part.trim()));
    Some((used, limit))
}

fn parse_docker_size_bytes(value: &str) -> Option<u64> {
    let compact = value.trim().replace(',', "");
    if compact.is_empty() {
        return None;
    }
    let split_at = compact
        .find(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .unwrap_or(compact.len());
    let number = compact[..split_at].trim().parse::<f64>().ok()?;
    let unit = compact[split_at..].trim().to_ascii_lowercase();
    let multiplier = match unit.as_str() {
        "" | "b" => 1_f64,
        "kb" => 1_000_f64,
        "mb" => 1_000_000_f64,
        "gb" => 1_000_000_000_f64,
        "tb" => 1_000_000_000_000_f64,
        "kib" => 1024_f64,
        "mib" => 1024_f64.powi(2),
        "gib" => 1024_f64.powi(3),
        "tib" => 1024_f64.powi(4),
        _ => return None,
    };
    Some((number * multiplier).round().max(0.0) as u64)
}

fn round_1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn is_valid_app_id(app_id: &str) -> bool {
    !app_id.is_empty()
        && app_id.len() <= 64
        && app_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

async fn read_json(path: &FsPath) -> Value {
    let mut value = match tokio::fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({})),
        Err(_) => json!({}),
    };
    if !value.is_object() {
        value = json!({});
    }
    value
}

fn str_val(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn bool_val(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(|v| v.as_bool())
}

fn map_val(value: &Value, key: &str) -> HashMap<String, String> {
    value
        .get(key)
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| {
                    let text = match v {
                        Value::String(s) => s.clone(),
                        Value::Bool(b) => b.to_string(),
                        Value::Number(n) => n.to_string(),
                        _ => return None,
                    };
                    Some((k.clone(), text))
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn app_dir(state: &ExecutorState, app_id: &str) -> Option<PathBuf> {
    if let Some(dir) = state.registry.get_dir(app_id).await {
        return Some(dir);
    }
    let fallback = state.config.data_dir.join("apps").join(app_id);
    fallback.exists().then_some(fallback)
}

async fn app_row(state: &ExecutorState, app_id: &str) -> Option<Value> {
    state
        .registry
        .list()
        .await
        .into_iter()
        .find(|row| row.get("id").and_then(|v| v.as_str()) == Some(app_id))
}

async fn load_spec(state: &ExecutorState, app_id: &str) -> Result<LoadedAppSpec> {
    let app_dir = app_dir(state, app_id)
        .await
        .ok_or_else(|| anyhow::anyhow!("App not found"))?;
    let meta = read_json(&app_dir.join(".app_meta.json")).await;
    let row = app_row(state, app_id).await;

    let title = row
        .as_ref()
        .and_then(|v| v.get("title").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .or_else(|| str_val(&meta, "title"))
        .unwrap_or_else(|| app_id.to_string());
    let access_guard_enabled = row
        .as_ref()
        .and_then(|v| v.get("access_guard_enabled").and_then(|v| v.as_bool()))
        .or_else(|| bool_val(&meta, "access_guard_enabled"))
        .unwrap_or(false);
    let expose_public = row
        .as_ref()
        .and_then(|v| v.get("expose_public").and_then(|v| v.as_bool()))
        .or_else(|| bool_val(&meta, "expose_public"))
        .unwrap_or(false);
    let mut access_key = row
        .as_ref()
        .and_then(|v| {
            v.get("access_password")
                .or_else(|| v.get("access_key"))
                .and_then(|value| value.as_str())
        })
        .map(|s| s.to_string())
        .unwrap_or_default();
    if access_key.trim().is_empty() {
        access_key = state.registry.access_key(app_id).await.unwrap_or_default();
    }
    if access_guard_enabled && access_key.trim().is_empty() {
        access_key = generate_access_key();
    }
    let entry_command = crate::actions::app::app_meta_lifecycle_command(&meta, "entry_command");
    let install_command = crate::actions::app::app_meta_lifecycle_command(&meta, "install_command");
    let build_command = crate::actions::app::app_meta_lifecycle_command(&meta, "build_command");
    let is_static = row
        .as_ref()
        .and_then(|v| v.get("is_static").and_then(|v| v.as_bool()))
        .unwrap_or_else(|| entry_command.is_none());

    Ok(LoadedAppSpec {
        title,
        app_dir,
        access_guard_enabled,
        access_key,
        expose_public,
        is_static,
        entry_command,
        install_command,
        build_command,
        runtime_image: str_val(&meta, "runtime_image"),
        runtime_preference: runtime_preference_from_opt(
            str_val(&meta, "runtime_preference").as_deref(),
        ),
        required_inputs: parse_required_inputs(&meta),
        config_values: map_val(&meta, "config_values"),
    })
}

async fn wait_for_runtime(
    registry: &AppRegistry,
    app_id: &str,
    port: u16,
    proxy_path_mode: crate::actions::app::AppProxyPathMode,
) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if !registry.runtime_is_alive(app_id).await {
            anyhow::bail!("App {} stopped before it opened port {}", app_id, port);
        }
        if let Ok(Ok(_stream)) = tokio::time::timeout(
            Duration::from_millis(1200),
            tokio::net::TcpStream::connect(("127.0.0.1", port)),
        )
        .await
        {
            let readiness =
                crate::actions::app::runtime_http_readiness_check(port, proxy_path_mode, app_id)
                    .await;
            if readiness.ready {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!(
                    "App {} accepted TCP connections on port {} but failed HTTP readiness: {}",
                    app_id,
                    port,
                    readiness.detail
                );
            }
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "App {} did not become HTTP-ready on port {} within 30s",
                app_id,
                port
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn code_execute_exec_id(output_files: &[String]) -> Option<String> {
    output_files.iter().find_map(|path| {
        let mut parts = path.trim_matches('/').split('/');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("api"), Some("outputs"), Some(exec_id)) if !exec_id.trim().is_empty() => {
                Some(exec_id.to_string())
            }
            _ => None,
        }
    })
}

async fn start_dynamic(state: &ExecutorState, app_id: &str) -> Result<Value> {
    let spec = load_spec(state, app_id).await?;
    if spec.is_static {
        anyhow::bail!("App {} is static", app_id);
    }

    if state.registry.get_dir(app_id).await.is_some() {
        let _ = state.registry.stop_runtime(app_id).await;
    }

    let env = std::env::vars().collect::<HashMap<String, String>>();
    let (resolved_env, missing_sensitive, missing_config) = resolve_required_env_values(
        &state.config.config_dir,
        &state.config.data_dir,
        Some(app_id),
        &spec.required_inputs,
        &env,
        &spec.config_values,
    )
    .await?;
    if !missing_sensitive.is_empty() || !missing_config.is_empty() {
        anyhow::bail!(
            "Missing required inputs: {}{}",
            if missing_sensitive.is_empty() {
                String::new()
            } else {
                format!("secrets [{}]", missing_sensitive.join(", "))
            },
            if missing_config.is_empty() {
                String::new()
            } else if missing_sensitive.is_empty() {
                format!("configs [{}]", missing_config.join(", "))
            } else {
                format!(", configs [{}]", missing_config.join(", "))
            }
        );
    }

    let port = state
        .registry
        .find_available_port()
        .await
        .ok_or_else(|| anyhow::anyhow!("No available ports in range"))?;
    let runtime = launch_dynamic_runtime(DynamicRuntimeLaunch {
        app_id,
        app_dir: &spec.app_dir,
        entry_command: spec
            .entry_command
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Missing entry_command"))?,
        install_command: spec.install_command.as_deref(),
        build_command: spec.build_command.as_deref(),
        port,
        extra_env: &resolved_env,
        runtime_image: spec.runtime_image.as_deref(),
        runtime_preference: spec.runtime_preference,
        stream_tx: None,
    })
    .await?;
    let (child, container_id) = match runtime {
        DynamicRuntimeHandle::Container(container_id) => (None, Some(container_id)),
        DynamicRuntimeHandle::Process(child) => (Some(*child), None),
    };

    state
        .registry
        .register_dynamic(
            app_id.to_string(),
            DynamicAppRegistration {
                title: spec.title.clone(),
                app_dir: spec.app_dir.clone(),
                child,
                container_id,
                port,
                access_key: spec.access_key.clone(),
                access_guard_enabled: spec.access_guard_enabled,
                expose_public: spec.expose_public,
                enabled: true,
                last_accessed: None,
            },
        )
        .await;
    let _ = state.registry.set_enabled(app_id, true).await;
    let proxy_path_mode = crate::actions::app::proxy_path_mode_for_entry_command(
        spec.entry_command.as_deref(),
        &spec.app_dir,
        app_id,
    );
    if let Err(error) = wait_for_runtime(&state.registry, app_id, port, proxy_path_mode).await {
        let _ = state.registry.stop_runtime(app_id).await;
        let tail = read_local_runtime_log_tail(&spec.app_dir, 4096).await;
        if tail.is_empty() {
            anyhow::bail!(error);
        }
        anyhow::bail!("{}. Recent runtime logs:\n{}", error, tail);
    }

    Ok(json!({
        "status": "restarted",
        "mode": "dynamic",
        "app_id": app_id,
        "title": spec.title,
        "port": port,
        "runtime_mode": "isolated_container",
        "is_isolated_runtime": true,
        "url": format!("/internal/v1/apps/{}/proxy/", app_id),
        "access_url": format!("/internal/v1/apps/{}/proxy/", app_id),
        "access_key": spec.access_key.clone(),
        "access_password": spec.access_key,
        "access_guard_enabled": spec.access_guard_enabled,
        "expose_public": spec.expose_public,
        "enabled": true
    }))
}

async fn start_static(state: &ExecutorState, app_id: &str) -> Result<Value> {
    let spec = load_spec(state, app_id).await?;
    if state.registry.get_dir(app_id).await.is_some() {
        let _ = state.registry.set_enabled(app_id, true).await;
    } else {
        state
            .registry
            .register_stored(
                app_id.to_string(),
                crate::actions::app::StoredAppRegistration {
                    title: spec.title.clone(),
                    app_dir: spec.app_dir.clone(),
                    is_static: true,
                    access_key: spec.access_key.clone(),
                    access_guard_enabled: spec.access_guard_enabled,
                    expose_public: spec.expose_public,
                    enabled: true,
                    last_accessed: None,
                },
            )
            .await;
    }
    Ok(json!({
        "status": "restarted",
        "mode": "static",
        "app_id": app_id,
        "title": spec.title,
        "url": format!("/internal/v1/apps/{}/proxy/", app_id),
        "access_url": format!("/internal/v1/apps/{}/proxy/", app_id),
        "access_key": spec.access_key.clone(),
        "access_password": spec.access_key,
        "access_guard_enabled": spec.access_guard_enabled,
        "expose_public": spec.expose_public,
        "enabled": true
    }))
}

async fn code_execute(
    State(state): State<ExecutorState>,
    headers: HeaderMap,
    Json(request): Json<CodeExecuteRequest>,
) -> impl IntoResponse {
    if let Err(status) = authorize_internal(&headers, state.config.token.as_deref()) {
        return (
            status,
            Json(CodeExecuteResponse {
                status: "error".to_string(),
                message: "Unauthorized".to_string(),
                exec_id: None,
                output_files: vec![],
                output_text: None,
                raw: json!({}),
            }),
        );
    }
    let arguments = json!({
        "language": request.language,
        "code": request.code,
        "env": request.env,
        "network_access": request.network_access,
        "execution_contract": request.execution_contract,
        "file_payloads": request.file_payloads,
    });
    let auth_context = request.auth_context.unwrap_or_default();
    match state
        .runtime
        .execute_action_with_context("code_execute", &arguments, &auth_context)
        .await
    {
        Ok(raw_result) => {
            let parsed = serde_json::from_str::<Value>(&raw_result).unwrap_or_else(|_| {
                json!({
                    "output": raw_result,
                    "error": serde_json::Value::Null,
                    "exit_code": 0,
                    "files": Vec::<String>::new(),
                })
            });
            let output_files = parsed
                .get("files")
                .and_then(|value| value.as_array())
                .map(|files| {
                    files
                        .iter()
                        .filter_map(|value| value.as_str().map(|value| value.to_string()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let exit_code = parsed
                .get("exit_code")
                .and_then(|value| value.as_i64())
                .unwrap_or(0);
            let output_text = parsed
                .get("output")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string());
            let message = if exit_code == 0 {
                "Code execution completed.".to_string()
            } else {
                parsed
                    .get("error")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string())
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| {
                        format!("Code execution failed with exit code {}", exit_code)
                    })
            };
            (
                StatusCode::OK,
                Json(CodeExecuteResponse {
                    status: "ok".to_string(),
                    message,
                    exec_id: code_execute_exec_id(&output_files),
                    output_files,
                    output_text,
                    raw: parsed,
                }),
            )
        }
        Err(error) => (
            StatusCode::OK,
            Json(CodeExecuteResponse {
                status: "error".to_string(),
                message: error.to_string(),
                exec_id: None,
                output_files: vec![],
                output_text: None,
                raw: json!({
                    "output": "",
                    "error": error.to_string(),
                    "exit_code": -1,
                    "files": Vec::<String>::new(),
                }),
            }),
        ),
    }
}

async fn app_deploy(
    State(state): State<ExecutorState>,
    headers: HeaderMap,
    Json(request): Json<AppDeployRequest>,
) -> impl IntoResponse {
    if let Err(status) = authorize_internal(&headers, state.config.token.as_deref()) {
        return (
            status,
            Json(AppDeployResponse {
                status: "error".to_string(),
                message: "Unauthorized".to_string(),
                app_id: None,
                url: None,
                raw: json!({}),
            }),
        );
    }
    let mut arguments = serde_json::Map::new();
    if let Some(value) = request
        .app_id
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        arguments.insert("app_id".to_string(), json!(value));
    }
    if let Some(value) = request
        .mode
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        arguments.insert("mode".to_string(), json!(value));
    }
    if !request.files.is_empty() {
        arguments.insert(
            "files".to_string(),
            serde_json::to_value(&request.files).unwrap_or_default(),
        );
    }
    if !request.file_patches.is_empty() {
        arguments.insert(
            "file_patches".to_string(),
            serde_json::to_value(&request.file_patches).unwrap_or_default(),
        );
    }
    if !request.delete_paths.is_empty() {
        arguments.insert(
            "delete_paths".to_string(),
            serde_json::to_value(&request.delete_paths).unwrap_or_default(),
        );
    }
    if let Some(value) = request
        .repo_url
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        arguments.insert("repo_url".to_string(), json!(value));
    }
    if let Some(value) = request
        .repo_ref
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        arguments.insert("repo_ref".to_string(), json!(value));
    }
    if let Some(value) = request
        .repo_subdir
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        arguments.insert("repo_subdir".to_string(), json!(value));
    }
    if let Some(value) = request
        .service_mode
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        arguments.insert("service_mode".to_string(), json!(value));
    }
    if let Some(value) = request
        .title
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        arguments.insert("title".to_string(), json!(value));
    }
    if let Some(value) = request
        .runtime_image
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        arguments.insert("runtime_image".to_string(), json!(value));
    }
    if let Some(value) = request
        .runtime_preference
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        arguments.insert("runtime_preference".to_string(), json!(value));
    }
    if let Some(value) = request.runtime_required {
        arguments.insert("runtime_required".to_string(), json!(value));
    }
    if let Some(value) = request.expose_public {
        arguments.insert("expose_public".to_string(), json!(value));
    }
    if let Some(value) = request.access_guard {
        arguments.insert("access_guard".to_string(), json!(value));
    }
    if let Some(value) = request
        .access_password
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        arguments.insert("access_password".to_string(), json!(value));
    }
    if !request.required_inputs.is_empty() {
        arguments.insert(
            "required_inputs".to_string(),
            serde_json::to_value(&request.required_inputs).unwrap_or_else(|_| json!([])),
        );
    }
    if !request.config_values.is_empty() {
        arguments.insert(
            "config_values".to_string(),
            serde_json::to_value(&request.config_values).unwrap_or_else(|_| json!({})),
        );
    }
    if let Some(value) = request
        .install_command
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        arguments.insert("install_command".to_string(), json!(value));
    }
    if let Some(value) = request
        .build_command
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        arguments.insert("build_command".to_string(), json!(value));
    }
    if let Some(value) = request
        .entry_command
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        arguments.insert("entry_command".to_string(), json!(value));
    }
    if let Some(value) = request
        .start_command
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        arguments.insert("start_command".to_string(), json!(value));
    }
    if let Some(value) = request
        .stop_command
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        arguments.insert("stop_command".to_string(), json!(value));
    }
    if !request.commands.is_empty() {
        arguments.insert(
            "commands".to_string(),
            serde_json::to_value(&request.commands).unwrap_or_else(|_| json!({})),
        );
    }

    let llm_env = std::env::vars().collect::<HashMap<String, String>>();
    match crate::actions::app::app_deploy(
        &state.config.config_dir,
        &state.config.data_dir,
        &Value::Object(arguments),
        &state.registry,
        &llm_env,
        None,
    )
    .await
    {
        Ok(raw_result) => {
            let parsed = serde_json::from_str::<Value>(&raw_result)
                .unwrap_or_else(|_| json!({ "raw_result": raw_result }));
            (
                StatusCode::OK,
                Json(AppDeployResponse {
                    status: parsed
                        .get("status")
                        .and_then(|value| value.as_str())
                        .unwrap_or("deployed")
                        .to_string(),
                    message: parsed
                        .get("message")
                        .and_then(|value| value.as_str())
                        .unwrap_or("App deployment completed.")
                        .to_string(),
                    app_id: parsed
                        .get("app_id")
                        .and_then(|value| value.as_str())
                        .map(|value| value.to_string()),
                    url: parsed
                        .get("url")
                        .and_then(|value| value.as_str())
                        .map(|value| value.to_string()),
                    raw: parsed,
                }),
            )
        }
        Err(error) => (
            StatusCode::OK,
            Json(AppDeployResponse {
                status: "error".to_string(),
                message: error.to_string(),
                app_id: None,
                url: None,
                raw: json!({ "error": error.to_string() }),
            }),
        ),
    }
}

async fn app_restart(
    State(state): State<ExecutorState>,
    Path(app_id): Path<String>,
    headers: HeaderMap,
    Json(_request): Json<AppLifecycleRequest>,
) -> impl IntoResponse {
    if let Err(status) = authorize_internal(&headers, state.config.token.as_deref()) {
        return (
            status,
            Json(AppActionResponse {
                status: "error".to_string(),
                message: "Unauthorized".to_string(),
                raw: json!({}),
            }),
        );
    }
    if !is_valid_app_id(&app_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(AppActionResponse {
                status: "error".to_string(),
                message: "Invalid app_id".to_string(),
                raw: json!({}),
            }),
        );
    }

    match load_spec(&state, &app_id).await {
        Ok(spec) if spec.is_static => match start_static(&state, &app_id).await {
            Ok(raw) => (
                StatusCode::OK,
                Json(AppActionResponse {
                    status: "restarted".to_string(),
                    message: format!("Restarted static app {}", spec.title),
                    raw,
                }),
            ),
            Err(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(AppActionResponse {
                    status: "error".to_string(),
                    message: error.to_string(),
                    raw: json!({ "app_id": app_id }),
                }),
            ),
        },
        Ok(_) => match start_dynamic(&state, &app_id).await {
            Ok(raw) => (
                StatusCode::OK,
                Json(AppActionResponse {
                    status: "restarted".to_string(),
                    message: format!("Restarted dynamic app {}", app_id),
                    raw,
                }),
            ),
            Err(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(AppActionResponse {
                    status: "error".to_string(),
                    message: format!("Failed to restart app: {}", error),
                    raw: json!({ "app_id": app_id }),
                }),
            ),
        },
        Err(error) => (
            StatusCode::NOT_FOUND,
            Json(AppActionResponse {
                status: "error".to_string(),
                message: error.to_string(),
                raw: json!({ "app_id": app_id }),
            }),
        ),
    }
}

async fn app_stop(
    State(state): State<ExecutorState>,
    Path(app_id): Path<String>,
    headers: HeaderMap,
    Json(_request): Json<AppLifecycleRequest>,
) -> impl IntoResponse {
    if let Err(status) = authorize_internal(&headers, state.config.token.as_deref()) {
        return (
            status,
            Json(AppActionResponse {
                status: "error".to_string(),
                message: "Unauthorized".to_string(),
                raw: json!({}),
            }),
        );
    }
    if !is_valid_app_id(&app_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(AppActionResponse {
                status: "error".to_string(),
                message: "Invalid app_id".to_string(),
                raw: json!({}),
            }),
        );
    }
    let spec = match load_spec(&state, &app_id).await {
        Ok(spec) => spec,
        Err(error) => {
            return (
                StatusCode::NOT_FOUND,
                Json(AppActionResponse {
                    status: "error".to_string(),
                    message: error.to_string(),
                    raw: json!({ "app_id": app_id }),
                }),
            );
        }
    };

    if let Err(error) = state.registry.stop_runtime(&app_id).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(AppActionResponse {
                status: "error".to_string(),
                message: format!("Failed to stop runtime: {}", error),
                raw: json!({ "app_id": app_id }),
            }),
        );
    }
    let _ = state.registry.set_enabled(&app_id, false).await;
    (
        StatusCode::OK,
        Json(AppActionResponse {
            status: "disabled".to_string(),
            message: format!("Stopped app {}", spec.title),
            raw: json!({
                "app_id": app_id,
                "title": spec.title,
                "enabled": false
            }),
        }),
    )
}

async fn app_delete(
    State(state): State<ExecutorState>,
    Path(app_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize_internal(&headers, state.config.token.as_deref()) {
        return (
            status,
            Json(AppActionResponse {
                status: "error".to_string(),
                message: "Unauthorized".to_string(),
                raw: json!({}),
            }),
        );
    }
    if !is_valid_app_id(&app_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(AppActionResponse {
                status: "error".to_string(),
                message: "Invalid app_id".to_string(),
                raw: json!({}),
            }),
        );
    }

    tracing::info!("executor app_delete: starting cleanup for '{}'", app_id);

    // Resolve workspace dir but treat absence as "already gone" (idempotent).
    let dir = app_dir(&state, &app_id)
        .await
        .unwrap_or_else(|| state.config.data_dir.join("apps").join(&app_id));

    let mut warnings: Vec<String> = Vec::new();

    // Step 1: stop the runtime process/container before removing files so
    // filesystem mounts release the workspace.
    tracing::info!("executor app_delete: stopping runtime for '{}'", app_id);
    if let Err(error) = state.registry.stop_runtime(&app_id).await {
        tracing::warn!(
            "executor app_delete: stop_runtime failed for '{}': {}",
            app_id,
            error
        );
        warnings.push(format!("stop_runtime: {}", error));
    }

    // Step 2: belt-and-suspenders container removal - covers cases where the
    // registry never recorded a container_id but a stale container exists.
    let container_name = crate::actions::app::app_container_name(&app_id);
    tracing::info!(
        "executor app_delete: ensuring container '{}' removed for '{}'",
        container_name,
        app_id
    );
    crate::actions::app::cleanup_existing_container(&container_name).await;

    // Step 3: remove the workspace directory (source, node_modules,
    // .agentark/venv, captured stdout/stderr logs, dockerfiles, etc.).
    tracing::info!("executor app_delete: removing workspace {}", dir.display());
    match tokio::fs::remove_dir_all(&dir).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(
                "executor app_delete: failed to remove workspace {} for '{}': {}",
                dir.display(),
                app_id,
                error
            );
            warnings.push(format!("remove_workspace: {}", error));
        }
    }

    // Step 4: drop the registry entry (frees reserved port via stop()).
    if let Err(error) = state.registry.stop(&app_id).await {
        tracing::warn!(
            "executor app_delete: registry.stop failed for '{}': {}",
            app_id,
            error
        );
        warnings.push(format!("registry_stop: {}", error));
    }

    tracing::info!(
        "executor app_delete: completed for '{}' (warnings={})",
        app_id,
        warnings.len()
    );

    (
        StatusCode::OK,
        Json(AppActionResponse {
            status: "deleted".to_string(),
            message: format!("Deleted app {}", app_id),
            raw: json!({ "app_id": app_id, "warnings": warnings }),
        }),
    )
}

async fn app_status(
    State(state): State<ExecutorState>,
    Path(app_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = authorize_internal(&headers, state.config.token.as_deref()) {
        return (
            status,
            Json(json!({
                "status": "error",
                "message": "Unauthorized"
            })),
        )
            .into_response();
    }
    if !is_valid_app_id(&app_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "status": "error",
                "message": "Invalid app_id"
            })),
        )
            .into_response();
    }
    let Some(row) = app_row(&state, &app_id).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "status": "error",
                "message": "App not found",
                "app_id": app_id
            })),
        )
            .into_response();
    };
    Json(json!({
        "status": "ok",
        "app_id": app_id,
        "title": row.get("title").and_then(|v| v.as_str()).unwrap_or("App"),
        "running": row.get("running").and_then(|v| v.as_bool()).unwrap_or(false),
        "port": row.get("port").and_then(|v| v.as_u64()),
        "runtime_mode": row.get("runtime_mode").and_then(|v| v.as_str()).unwrap_or("stopped"),
        "is_isolated_runtime": row.get("is_isolated_runtime").and_then(|v| v.as_bool()).unwrap_or(false),
        "enabled": row.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
        "restoring": row.get("restoring").and_then(|v| v.as_bool()).unwrap_or(false),
        "restore_error": row.get("restore_error").cloned(),
        "restore_status": row.get("restore_status").and_then(|v| v.as_str()).unwrap_or("ready"),
        "url": format!("/internal/v1/apps/{}/proxy/", app_id),
        "access_url": format!("/internal/v1/apps/{}/proxy/", app_id),
        "raw": row,
    }))
    .into_response()
}

async fn app_logs(
    State(state): State<ExecutorState>,
    Path(app_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(status) = authorize_internal(&headers, state.config.token.as_deref()) {
        return (
            status,
            Json(json!({
                "status": "error",
                "message": "Unauthorized"
            })),
        )
            .into_response();
    }
    if !is_valid_app_id(&app_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "status": "error",
                "message": "Invalid app_id"
            })),
        )
            .into_response();
    }
    let Some(spec) = load_spec(&state, &app_id).await.ok() else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "status": "error",
                "message": "App not found",
                "app_id": app_id
            })),
        )
            .into_response();
    };
    let logs = read_local_runtime_log_tail(&spec.app_dir, 4096).await;
    Json(json!({
        "status": "ok",
        "app_id": app_id,
        "logs": logs,
        "message": if logs.is_empty() { "No runtime logs available" } else { "Runtime logs loaded" },
    }))
    .into_response()
}

async fn proxy_app_root(
    State(state): State<ExecutorState>,
    Path(app_id): Path<String>,
    headers: HeaderMap,
    request: Request,
) -> Response {
    proxy_app_request(state, app_id, String::new(), headers, request).await
}

async fn proxy_app_path(
    State(state): State<ExecutorState>,
    Path((app_id, path)): Path<(String, String)>,
    headers: HeaderMap,
    request: Request,
) -> Response {
    proxy_app_request(state, app_id, path, headers, request).await
}

fn is_hop_by_hop_header(name: &str) -> bool {
    matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    let has_upgrade = headers
        .get(header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false);
    let websocket = headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    has_upgrade && websocket
}

fn axum_to_tungstenite_message(msg: AxumWsMessage) -> Option<TungsteniteMessage> {
    match msg {
        AxumWsMessage::Text(text) => Some(TungsteniteMessage::Text(text.to_string().into())),
        AxumWsMessage::Binary(data) => Some(TungsteniteMessage::Binary(data)),
        AxumWsMessage::Ping(data) => Some(TungsteniteMessage::Ping(data)),
        AxumWsMessage::Pong(data) => Some(TungsteniteMessage::Pong(data)),
        AxumWsMessage::Close(_) => Some(TungsteniteMessage::Close(None)),
    }
}

fn tungstenite_to_axum_message(msg: TungsteniteMessage) -> Option<AxumWsMessage> {
    match msg {
        TungsteniteMessage::Text(text) => Some(AxumWsMessage::Text(text.to_string().into())),
        TungsteniteMessage::Binary(data) => Some(AxumWsMessage::Binary(data)),
        TungsteniteMessage::Ping(data) => Some(AxumWsMessage::Ping(data)),
        TungsteniteMessage::Pong(data) => Some(AxumWsMessage::Pong(data)),
        TungsteniteMessage::Close(_) => Some(AxumWsMessage::Close(None)),
        TungsteniteMessage::Frame(_) => None,
    }
}

async fn proxy_websocket_connection(
    client_socket: WebSocket,
    upstream_url: String,
    requested_protocols: Vec<String>,
    forward_headers: Vec<(String, String)>,
) {
    let mut upstream_request = match upstream_url.into_client_request() {
        Ok(request) => request,
        Err(error) => {
            tracing::warn!("Failed to build upstream WS request: {}", error);
            return;
        }
    };
    if !requested_protocols.is_empty() {
        let protocols = requested_protocols.join(", ");
        if let Ok(value) = HeaderValue::from_str(&protocols) {
            upstream_request
                .headers_mut()
                .insert("Sec-WebSocket-Protocol", value);
        }
    }
    for (name, value) in forward_headers {
        if let (Ok(header_name), Ok(header_value)) = (
            axum::http::HeaderName::from_bytes(name.as_bytes()),
            axum::http::HeaderValue::from_str(&value),
        ) {
            upstream_request
                .headers_mut()
                .insert(header_name, header_value);
        }
    }

    let (upstream_socket, _) = match connect_async(upstream_request).await {
        Ok(pair) => pair,
        Err(error) => {
            tracing::warn!("Failed to connect to upstream WS app: {}", error);
            return;
        }
    };

    let (mut client_sender, mut client_receiver) = client_socket.split();
    let (mut upstream_sender, mut upstream_receiver) = upstream_socket.split();

    let client_to_upstream = async {
        while let Some(result) = client_receiver.next().await {
            match result {
                Ok(message) => {
                    let Some(upstream_message) = axum_to_tungstenite_message(message) else {
                        continue;
                    };
                    if upstream_sender.send(upstream_message).await.is_err() {
                        break;
                    }
                }
                Err(error) => {
                    tracing::debug!("Client WS receive error: {}", error);
                    break;
                }
            }
        }
        let _ = upstream_sender.close().await;
    };

    let upstream_to_client = async {
        while let Some(result) = upstream_receiver.next().await {
            match result {
                Ok(message) => {
                    let Some(client_message) = tungstenite_to_axum_message(message) else {
                        continue;
                    };
                    if client_sender.send(client_message).await.is_err() {
                        break;
                    }
                }
                Err(error) => {
                    tracing::debug!("Upstream WS receive error: {}", error);
                    break;
                }
            }
        }
        let _ = client_sender.send(AxumWsMessage::Close(None)).await;
    };

    tokio::select! {
        _ = client_to_upstream => {}
        _ = upstream_to_client => {}
    }
}

async fn proxy_app_request(
    state: ExecutorState,
    app_id: String,
    path: String,
    headers: HeaderMap,
    request: Request,
) -> Response {
    if let Err(status) = authorize_internal(&headers, state.config.token.as_deref()) {
        return (status, "Unauthorized").into_response();
    }
    let (mut parts, body) = request.into_parts();
    let ws = if is_websocket_upgrade(&parts.headers) {
        WebSocketUpgrade::from_request_parts(&mut parts, &())
            .await
            .ok()
    } else {
        None
    };
    let method = parts.method.clone();
    let uri = parts.uri.clone();
    let headers = parts.headers.clone();

    if !is_valid_app_id(&app_id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if !state.registry.runtime_is_alive(&app_id).await {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Dynamic app is not running.",
        )
            .into_response();
    }
    let Some(port) = state.registry.get_port(&app_id).await else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "App runtime port is not available.",
        )
            .into_response();
    };
    state.registry.touch(&app_id).await;

    let proxy_path_mode = match app_dir(&state, &app_id).await {
        Some(dir) => crate::actions::app::proxy_path_mode_for_app_dir(&dir, &app_id).await,
        None => crate::actions::app::AppProxyPathMode::StripAppPrefix,
    };
    let target_path =
        crate::actions::app::dynamic_app_upstream_path(&app_id, &path, proxy_path_mode);
    let mut target_url = format!("http://127.0.0.1:{}{}", port, target_path);
    if let Some(query) = uri.query().filter(|q| !q.is_empty()) {
        target_url.push('?');
        target_url.push_str(query);
    }

    if is_websocket_upgrade(&headers) {
        if method != Method::GET {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        let Some(ws_upgrade) = ws else {
            return (StatusCode::BAD_REQUEST, "Invalid websocket upgrade request").into_response();
        };
        let requested_protocols = headers
            .get("Sec-WebSocket-Protocol")
            .and_then(|v| v.to_str().ok())
            .map(|raw| {
                raw.split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();
        let mut ws_forward_headers = Vec::new();
        if let Some(v) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
            ws_forward_headers.push(("origin".to_string(), v.to_string()));
        }
        if let Some(v) = headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
        {
            ws_forward_headers.push(("user-agent".to_string(), v.to_string()));
        }
        if let Some(v) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) {
            ws_forward_headers.push(("x-forwarded-host".to_string(), v.to_string()));
        }
        let forwarded_proto = headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("http");
        ws_forward_headers.push(("x-forwarded-proto".to_string(), forwarded_proto.to_string()));
        ws_forward_headers.push((
            "x-forwarded-prefix".to_string(),
            format!("/internal/v1/apps/{}/proxy", app_id),
        ));
        let ws_upgrade = if requested_protocols.is_empty() {
            ws_upgrade
        } else {
            ws_upgrade.protocols(requested_protocols.clone())
        };
        return ws_upgrade
            .on_upgrade(move |socket| async move {
                proxy_websocket_connection(
                    socket,
                    target_url,
                    requested_protocols,
                    ws_forward_headers,
                )
                .await;
            })
            .into_response();
    }

    let body_bytes = match to_bytes(body, 64 * 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "Request body too large").into_response(),
    };

    let mut upstream = state.client.request(method.clone(), &target_url);
    for (name, value) in &headers {
        let lower = name.as_str().to_ascii_lowercase();
        if is_hop_by_hop_header(&lower) || lower == "host" || lower == "content-length" {
            continue;
        }
        upstream = upstream.header(name, value);
    }
    if let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) {
        upstream = upstream.header("x-forwarded-host", host);
    }
    let forwarded_proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    upstream = upstream
        .header("x-forwarded-proto", forwarded_proto)
        .header(
            "x-forwarded-prefix",
            format!("/internal/v1/apps/{}/proxy", app_id),
        )
        .body(body_bytes);

    match upstream.send().await {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let response_headers = resp.headers().clone();
            match resp.bytes().await {
                Ok(response_body) => {
                    let mut builder = Response::builder().status(status);
                    for (name, value) in &response_headers {
                        if !is_hop_by_hop_header(name.as_str()) {
                            builder = builder.header(name, value);
                        }
                    }
                    builder
                        .body(Body::from(response_body))
                        .unwrap_or(StatusCode::BAD_GATEWAY.into_response())
                }
                Err(_) => StatusCode::BAD_GATEWAY.into_response(),
            }
        }
        Err(error) => {
            tracing::warn!(
                "Failed to proxy app {} request to {}: {}",
                app_id,
                target_url,
                error
            );
            (StatusCode::SERVICE_UNAVAILABLE, "App server not responding").into_response()
        }
    }
}
