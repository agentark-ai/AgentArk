use super::super::*;

impl ActionRuntime {
    /// Execute an action in WASM sandbox
    pub(in crate::runtime) async fn execute_wasm(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Result<String> {
        // For built-in actions, fall back to native with some wrapping
        match action_name {
            "http_get" => {
                let url = arguments["url"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing url"))?;
                let parsed_url = self
                    .resolve_http_get_url_for_context(url, auth_context)
                    .await?;
                let expected_mime = runtime_url_expected_mime(&parsed_url);
                let expected_non_text_resource = runtime_url_expects_non_text_resource(&parsed_url);
                let chat_override = Self::direct_trusted_chat_tool_override(auth_context);

                // Fast-path: try Lightpanda for external URLs (returns clean markdown)
                let has_custom_headers = arguments
                    .get("headers")
                    .and_then(|v| v.as_object())
                    .map(|h| !h.is_empty())
                    .unwrap_or(false);
                if !expected_non_text_resource
                    && !Self::http_get_url_is_privateish(&parsed_url)
                    && !has_custom_headers
                {
                    match crate::integrations::lightpanda::fetch_markdown(parsed_url.as_str()).await
                    {
                        Ok(markdown) => return Ok(markdown),
                        Err(e) => {
                            tracing::debug!("Lightpanda fast-path skipped for {}: {}", url, e);
                        }
                    }
                }

                let client = reqwest::Client::builder()
                    .user_agent(crate::branding::user_agent_with_suffix(
                        "(AI Agent Browser)",
                    ))
                    .timeout(std::time::Duration::from_secs(HTTP_GET_TIMEOUT_SECS))
                    .redirect(reqwest::redirect::Policy::limited(5))
                    .build()?;
                let mut req = client.get(parsed_url.clone());
                if let Some(headers) = arguments.get("headers").and_then(|v| v.as_object()) {
                    for (k, v) in headers {
                        let blocked = matches!(
                            k.to_ascii_lowercase().as_str(),
                            "host"
                                | "connection"
                                | "content-length"
                                | "transfer-encoding"
                                | "proxy-authorization"
                                | "x-forwarded-for"
                                | "x-forwarded-host"
                                | "x-real-ip"
                        );
                        if blocked && !chat_override {
                            anyhow::bail!("Header '{}' is not allowed for http_get", k);
                        }
                        if let Some(s) = v.as_str() {
                            req = req.header(k, s);
                        }
                    }
                }
                let response = req.send().await?;
                let status = response.status();
                if !status.is_success() {
                    return Err(crate::actions::structured_action_error(
                        ActionErrorDomain::Search,
                        ActionErrorReason::Failed,
                        format!("HTTP GET returned status {}", status.as_u16()),
                    ));
                }
                let content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let body_bytes = response.bytes().await?;
                if !runtime_response_matches_expected_url_mime(
                    expected_mime,
                    &content_type,
                    body_bytes.as_ref(),
                ) {
                    return Err(crate::actions::structured_action_error(
                        ActionErrorDomain::Search,
                        ActionErrorReason::Failed,
                        runtime_expected_mime_mismatch_message(
                            "HTTP GET",
                            expected_mime,
                            &content_type,
                        ),
                    ));
                }
                if expected_non_text_resource
                    || runtime_response_body_is_probably_binary(&content_type, body_bytes.as_ref())
                {
                    let payload = self
                        .persist_tool_payload_if_needed(
                            ToolPayload::Bytes {
                                mime: Some(content_type.clone())
                                    .filter(|value| !value.trim().is_empty()),
                                body: body_bytes.as_ref().to_vec(),
                                suggested_name: runtime_url_suggested_filename(&parsed_url),
                            },
                            PersistHints {
                                mime: Some(content_type.clone())
                                    .filter(|value| !value.trim().is_empty()),
                                source_action: Some("http_get".to_string()),
                                force_resource: expected_non_text_resource,
                                ..PersistHints::default()
                            },
                        )
                        .await?;
                    return Ok(Self::render_tool_payload_for_legacy("http_get", payload));
                }
                let body = if body_bytes.len() > HTTP_GET_MAX_BODY_BYTES {
                    format!(
                        "{}\n\n(response truncated at {} bytes)",
                        String::from_utf8_lossy(&body_bytes[..HTTP_GET_MAX_BODY_BYTES]),
                        HTTP_GET_MAX_BODY_BYTES
                    )
                } else {
                    String::from_utf8_lossy(&body_bytes).to_string()
                };

                Ok(body)
            }
            "manage_actions" => self.execute_manage_actions(arguments).await,
            "capability_acquire" => self.execute_capability_acquire(arguments).await,
            _ => {
                // Check if we have a WASM module for this action
                let actions = self.actions.read().await;
                if let Some(action) = actions.get(action_name) {
                    if action.wasm_module.is_some() {
                        anyhow::bail!(
                            "Imported WASM module execution is disabled for action '{}'",
                            action_name
                        );
                    }
                }
                drop(actions); // Release lock before async call
                               // Fall back to native
                self.execute_native(action_name, arguments).await
            }
        }
    }

    #[cfg(feature = "docker")]
    pub(in crate::runtime) fn docker_host_uses_socket_transport(host: &str) -> bool {
        let trimmed = host.trim();
        trimmed.starts_with("unix://") || trimmed.starts_with("npipe://")
    }

    /// Connect to Docker, honoring DOCKER_HOST transport instead of forcing HTTP.
    #[cfg(feature = "docker")]
    pub(in crate::runtime) fn connect_docker() -> Result<bollard::Docker> {
        if let Ok(host) = std::env::var("DOCKER_HOST") {
            let trimmed = host.trim();
            if !trimmed.is_empty() {
                let transport = if Self::docker_host_uses_socket_transport(trimmed) {
                    "socket"
                } else {
                    "network"
                };
                tracing::debug!(
                    "Connecting to Docker via DOCKER_HOST={} ({})",
                    trimmed,
                    transport
                );
                return bollard::Docker::connect_with_defaults().map_err(|e| {
                    anyhow::anyhow!("Failed to connect to Docker at {}: {}", trimmed, e)
                });
            }
            bollard::Docker::connect_with_local_defaults()
                .map_err(|e| anyhow::anyhow!("Failed to connect to Docker: {}", e))
        } else {
            bollard::Docker::connect_with_local_defaults()
                .map_err(|e| anyhow::anyhow!("Failed to connect to Docker: {}", e))
        }
    }

    pub(in crate::runtime) fn should_manage_local_sandbox_containers_for(
        role: Option<&str>,
        has_local_docker_endpoint: bool,
    ) -> bool {
        let is_control_plane = role
            .map(|value| value.trim().to_ascii_lowercase())
            .is_some_and(|value| matches!(value.as_str(), "control-plane" | "control"));
        !is_control_plane || has_local_docker_endpoint
    }

    pub(crate) fn should_manage_local_sandbox_containers() -> bool {
        #[cfg(feature = "docker")]
        {
            let role = std::env::var("AGENTARK_STACK_ROLE").ok();
            let has_local_docker_endpoint = std::env::var("DOCKER_HOST")
                .ok()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false)
                || Path::new("/var/run/docker.sock").exists();
            Self::should_manage_local_sandbox_containers_for(
                role.as_deref(),
                has_local_docker_endpoint,
            )
        }
        #[cfg(not(feature = "docker"))]
        {
            false
        }
    }

    pub async fn docker_available(&self) -> bool {
        #[cfg(feature = "docker")]
        {
            match Self::connect_docker() {
                Ok(docker) => docker.ping().await.is_ok(),
                Err(_) => false,
            }
        }
        #[cfg(not(feature = "docker"))]
        {
            let _ = self;
            true
        }
    }

    #[cfg(feature = "docker")]
    pub(in crate::runtime) fn docker_security_opts() -> Vec<String> {
        let mut opts = vec!["no-new-privileges:true".to_string()];
        if let Ok(profile) = std::env::var("AGENTARK_DOCKER_SECCOMP_PROFILE") {
            let trimmed = profile.trim();
            if !trimmed.is_empty() {
                opts.push(format!("seccomp={}", trimmed));
            }
        }
        if let Ok(profile) = std::env::var("AGENTARK_DOCKER_APPARMOR_PROFILE") {
            let trimmed = profile.trim();
            if !trimmed.is_empty() {
                opts.push(format!("apparmor={}", trimmed));
            }
        }
        opts
    }

    #[cfg(feature = "docker")]
    pub(in crate::runtime) fn sandbox_container_labels(
        action_name: &str,
        isolation: ContainerIsolation,
    ) -> HashMap<String, String> {
        HashMap::from([
            (
                AGENTARK_SANDBOX_LABEL_KEY.to_string(),
                AGENTARK_SANDBOX_LABEL_VALUE.to_string(),
            ),
            ("agentark.action".to_string(), action_name.to_string()),
            (
                "agentark.isolation".to_string(),
                isolation.label().to_string(),
            ),
            (
                "agentark.network_access".to_string(),
                if isolation.network_access() {
                    "enabled"
                } else {
                    "disabled"
                }
                .to_string(),
            ),
            (
                "agentark.created_at".to_string(),
                chrono::Utc::now().to_rfc3339(),
            ),
        ])
    }

    #[cfg(feature = "docker")]
    pub(in crate::runtime) async fn remember_active_container(&self, id: &str) {
        let mut active = self.active_sandbox_containers.write().await;
        active.insert(id.to_string());
        crate::metrics::set_active_containers(active.len());
    }

    #[cfg(feature = "docker")]
    pub(in crate::runtime) async fn forget_active_container(&self, id: &str) {
        let mut active = self.active_sandbox_containers.write().await;
        active.remove(id);
        crate::metrics::set_active_containers(active.len());
    }

    #[cfg(feature = "docker")]
    pub(in crate::runtime) async fn update_container_reaper_status(
        &self,
        removed: u64,
        error: Option<String>,
    ) {
        let mut status = self.container_reaper_status.write().await;
        status.last_run_at = Some(chrono::Utc::now().to_rfc3339());
        status.last_removed_count = removed;
        status.total_removed_count = status.total_removed_count.saturating_add(removed);
        status.last_error = error;
    }

    pub async fn active_container_count(&self) -> usize {
        #[cfg(feature = "docker")]
        {
            return self.active_sandbox_containers.read().await.len();
        }
        #[cfg(not(feature = "docker"))]
        {
            0
        }
    }

    pub async fn container_reaper_status(&self) -> ContainerReaperStatus {
        #[cfg(feature = "docker")]
        {
            return self.container_reaper_status.read().await.clone();
        }
        #[cfg(not(feature = "docker"))]
        {
            ContainerReaperStatus::default()
        }
    }

    pub async fn reconcile_orphan_containers(&self) -> Result<u64> {
        #[cfg(feature = "docker")]
        {
            if !Self::should_manage_local_sandbox_containers() {
                tracing::debug!(
                    "Skipping local sandbox container reconciliation on control plane without a local Docker endpoint"
                );
                self.update_container_reaper_status(0, None).await;
                crate::metrics::record_container_sweeper_run("skipped", 0);
                return Ok(0);
            }

            let docker = match Self::connect_docker() {
                Ok(docker) => docker,
                Err(error) => {
                    let message = error.to_string();
                    self.update_container_reaper_status(0, Some(message.clone()))
                        .await;
                    crate::metrics::record_container_sweeper_run("error", 0);
                    return Err(error);
                }
            };

            let filters = HashMap::from([(
                "label".to_string(),
                vec![format!(
                    "{}={}",
                    AGENTARK_SANDBOX_LABEL_KEY, AGENTARK_SANDBOX_LABEL_VALUE
                )],
            )]);
            let containers = docker
                .list_containers(Some(bollard::query_parameters::ListContainersOptions {
                    all: true,
                    filters: Some(filters),
                    ..Default::default()
                }))
                .await?;
            let active = self.active_sandbox_containers.read().await.clone();
            let mut removed = 0u64;
            for container in containers {
                let Some(id) = container.id.as_deref() else {
                    continue;
                };
                if active.contains(id) {
                    continue;
                }
                Self::force_remove_container(&docker, id).await;
                removed = removed.saturating_add(1);
            }
            if let Err(error) = self.prune_stale_code_execute_artifacts().await {
                tracing::warn!(
                    "Failed to prune stale code execution artifacts during runtime reconciliation: {}",
                    error
                );
            }
            self.update_container_reaper_status(removed, None).await;
            crate::metrics::record_container_sweeper_run("ok", removed);
            Ok(removed)
        }
        #[cfg(not(feature = "docker"))]
        {
            Ok(0)
        }
    }

    pub(in crate::runtime) async fn prune_stale_path_entries(
        &self,
        root: &Path,
        max_age_secs: u64,
    ) -> Result<u64> {
        let mut removed = 0u64;
        let cutoff = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(max_age_secs))
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let mut entries = match tokio::fs::read_dir(root).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error.into()),
        };

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let metadata = match entry.metadata().await {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            let modified = match metadata.modified() {
                Ok(modified) => modified,
                Err(_) => continue,
            };
            if modified > cutoff {
                continue;
            }
            if metadata.is_dir() {
                tokio::fs::remove_dir_all(&path).await?;
                removed = removed.saturating_add(1);
            } else {
                tokio::fs::remove_file(&path).await?;
                removed = removed.saturating_add(1);
            }
        }

        Ok(removed)
    }

    pub(in crate::runtime) async fn prune_stale_native_code_execute_temp_dirs(
        &self,
    ) -> Result<u64> {
        let temp_root = std::env::temp_dir();
        let mut removed = 0u64;
        let cutoff = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(
                CODE_EXECUTE_NATIVE_TEMP_RETENTION_SECS,
            ))
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let mut entries = match tokio::fs::read_dir(&temp_root).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error.into()),
        };
        while let Some(entry) = entries.next_entry().await? {
            let filename = entry.file_name();
            let filename = filename.to_string_lossy();
            if !filename.starts_with("agentark-exec-") {
                continue;
            }
            let path = entry.path();
            let metadata = match entry.metadata().await {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if !metadata.is_dir() {
                continue;
            }
            let modified = match metadata.modified() {
                Ok(modified) => modified,
                Err(_) => continue,
            };
            if modified > cutoff {
                continue;
            }
            tokio::fs::remove_dir_all(&path).await?;
            removed = removed.saturating_add(1);
        }
        Ok(removed)
    }

    pub(in crate::runtime) async fn prune_stale_code_execute_artifacts(&self) -> Result<u64> {
        let mut removed = 0u64;
        removed = removed.saturating_add(
            self.prune_stale_path_entries(
                &self.data_dir().join("outputs"),
                CODE_EXECUTE_OUTPUT_RETENTION_SECS,
            )
            .await?,
        );
        removed = removed.saturating_add(self.prune_stale_native_code_execute_temp_dirs().await?);
        Ok(removed)
    }

    pub(in crate::runtime) fn docker_required_error(action_name: &str) -> anyhow::Error {
        anyhow::anyhow!(
            "Docker is required for '{}' execution but is not available",
            action_name
        )
    }

    pub(in crate::runtime) fn parse_runtime_backend(value: &str) -> Result<RuntimeBackend> {
        match value.trim().to_ascii_lowercase().as_str() {
            "docker" => Ok(RuntimeBackend::Docker),
            "native" => Ok(RuntimeBackend::Native),
            "executor_server" | "remote_executor" => Ok(RuntimeBackend::RemoteExecutor),
            "wasm" => Ok(RuntimeBackend::Wasm),
            other => anyhow::bail!(
                "Unsupported code_execute backend '{}'. Use auto, docker, native, or executor_server.",
                other
            ),
        }
    }

    pub(in crate::runtime) fn parse_backend_fallback_policy(
        value: Option<&str>,
    ) -> Result<BackendFallbackPolicy> {
        match value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("auto_degrade")
            .to_ascii_lowercase()
            .as_str()
        {
            "auto_degrade" | "auto" => Ok(BackendFallbackPolicy::AutoDegrade),
            "require_exact" | "exact" => Ok(BackendFallbackPolicy::RequireExact),
            "ask_user" | "ask" => Ok(BackendFallbackPolicy::AskUser),
            other => anyhow::bail!("Unsupported backend fallback policy '{}'", other),
        }
    }

    pub(in crate::runtime) fn code_execute_backend_preference(
        arguments: &serde_json::Value,
    ) -> Result<BackendPreference> {
        if let Some(preference) = arguments
            .get("backend_preference")
            .and_then(|value| value.as_object())
        {
            let preferred = preference
                .get("preferred")
                .and_then(|value| value.as_array())
                .ok_or_else(|| anyhow::anyhow!("backend_preference.preferred must be an array"))?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .ok_or_else(|| {
                            anyhow::anyhow!("backend_preference entries must be strings")
                        })
                        .and_then(Self::parse_runtime_backend)
                })
                .collect::<Result<Vec<_>>>()?;
            if preferred.is_empty() {
                anyhow::bail!("backend_preference.preferred cannot be empty");
            }
            let fallback_policy = Self::parse_backend_fallback_policy(
                preference
                    .get("fallback_policy")
                    .and_then(|value| value.as_str()),
            )?;
            return Ok(BackendPreference {
                preferred,
                fallback_policy,
            });
        }

        let backend = arguments
            .get("backend")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("auto")
            .to_ascii_lowercase();
        if backend == "auto" {
            return Ok(BackendPreference {
                preferred: vec![RuntimeBackend::Docker, RuntimeBackend::Native],
                fallback_policy: BackendFallbackPolicy::AutoDegrade,
            });
        }
        Ok(BackendPreference {
            preferred: vec![Self::parse_runtime_backend(&backend)?],
            fallback_policy: BackendFallbackPolicy::RequireExact,
        })
    }

    /// Execute an action in Docker sandbox
    pub(in crate::runtime) async fn execute_docker(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Result<String> {
        let code_execute_backend_preference = if action_name == "code_execute" {
            let preference = Self::code_execute_backend_preference(arguments)?;
            for backend in &preference.preferred {
                match backend {
                    RuntimeBackend::Native => return self.execute_code_native(arguments).await,
                    RuntimeBackend::RemoteExecutor => {
                        if Self::control_plane_executor_client().is_some() {
                            return self.execute_code_remote(arguments, auth_context).await;
                        }
                        if preference.fallback_policy == BackendFallbackPolicy::RequireExact {
                            anyhow::bail!(
                                "code_execute backend 'executor_server' was requested, but no executor server is configured"
                            );
                        }
                    }
                    RuntimeBackend::Wasm => {
                        if preference.fallback_policy == BackendFallbackPolicy::RequireExact {
                            anyhow::bail!(
                                "code_execute backend 'wasm' is not wired for code execution. Use auto, docker, native, or executor_server."
                            );
                        }
                    }
                    RuntimeBackend::Docker => break,
                }
            }
            if !preference.preferred.contains(&RuntimeBackend::Docker) {
                anyhow::bail!(
                    "No available code_execute backend matched the requested backend preference."
                );
            }
            Some(preference)
        } else {
            None
        };
        if let Some(_executor) = Self::control_plane_executor_client() {
            if action_name == "shell" {
                let command = arguments["command"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing command"))?;
                let shell_arguments = serde_json::json!({
                    "language": "bash",
                    "code": command,
                    "timeout_secs": arguments
                        .get("timeout_secs")
                        .and_then(|value| value.as_i64())
                        .unwrap_or(30),
                    "network_access": false,
                });
                return self
                    .execute_code_remote(&shell_arguments, auth_context)
                    .await;
            }
        }
        #[cfg(feature = "docker")]
        {
            // Check Docker availability first - fall back to native if unavailable
            let docker_available = Self::connect_docker().is_ok();

            if !docker_available {
                if let Some(preference) = &code_execute_backend_preference {
                    if preference.fallback_policy == BackendFallbackPolicy::AutoDegrade
                        && preference.preferred.contains(&RuntimeBackend::Native)
                    {
                        tracing::warn!(
                            "Docker not available for code_execute backend preference; using native fallback"
                        );
                        return self.execute_code_native(arguments).await;
                    }
                }
                if action_name == "code_execute" {
                    tracing::warn!(
                        "Docker not available for code_execute and no permitted fallback backend was available"
                    );
                }
                tracing::warn!(
                    "Docker not available for '{}'; refusing unsandboxed fallback execution",
                    action_name
                );
                return Err(Self::docker_required_error(action_name));
            }

            match action_name {
                "shell" => {
                    const PUBLIC_SHELL_SANDBOX_IMAGE: &str = "alpine:3.20";
                    self.run_isolated_container(
                        action_name,
                        PUBLIC_SHELL_SANDBOX_IMAGE,
                        vec![
                            "sh".to_string(),
                            "-c".to_string(),
                            arguments["command"]
                                .as_str()
                                .ok_or_else(|| anyhow::anyhow!("Missing command"))?
                                .to_string(),
                        ],
                        None,
                        30,
                        ContainerIsolation::Strict,
                    )
                    .await
                }
                "code_execute" => self.execute_code_docker(arguments, auth_context).await,
                _ => Err(anyhow::anyhow!("Unknown docker action: {}", action_name)),
            }
        }

        #[cfg(not(feature = "docker"))]
        {
            if let Some(preference) = &code_execute_backend_preference {
                if preference.fallback_policy == BackendFallbackPolicy::AutoDegrade
                    && preference.preferred.contains(&RuntimeBackend::Native)
                {
                    tracing::warn!(
                        "Docker feature unavailable for code_execute backend preference; using native fallback"
                    );
                    return self.execute_code_native(arguments).await;
                }
            }
            let _ = arguments;
            Err(Self::docker_required_error(action_name))
        }
    }

    /// Force-remove a Docker container (stop + remove), ignoring errors.
    /// Guaranteed to not leave containers behind.
    #[cfg(feature = "docker")]
    pub(in crate::runtime) async fn force_remove_container(docker: &bollard::Docker, id: &str) {
        // Kill first (faster than stop for stuck containers)
        let _ = docker.kill_container(id, None).await;
        // Stop as fallback (handles already-stopped containers)
        let _ = docker
            .stop_container(
                id,
                Some(bollard::query_parameters::StopContainerOptions {
                    t: Some(0),
                    ..Default::default()
                }),
            )
            .await;
        // Force remove - deletes container, volumes, and anonymous volumes
        let _ = docker
            .remove_container(
                id,
                Some(bollard::query_parameters::RemoveContainerOptions {
                    force: true,
                    v: true, // Remove anonymous volumes attached to the container
                    ..Default::default()
                }),
            )
            .await;
        tracing::debug!("Cleaned up container {}", &id[..12.min(id.len())]);
    }

    /// Ensure a Docker image is available locally, pulling it if necessary.
    #[cfg(feature = "docker")]
    pub(in crate::runtime) async fn ensure_image(
        docker: &bollard::Docker,
        image: &str,
    ) -> Result<()> {
        // Check if image exists locally
        if docker.inspect_image(image).await.is_ok() {
            return Ok(());
        }

        tracing::info!("Pulling Docker image '{}' (first-time download)...", image);

        let mut stream = docker.create_image(
            Some(bollard::query_parameters::CreateImageOptions {
                from_image: Some(image.to_string()),
                ..Default::default()
            }),
            None,
            None,
        );

        use futures::StreamExt;
        while let Some(result) = stream.next().await {
            match result {
                Ok(info) => {
                    if let Some(status) = &info.status {
                        tracing::debug!("Pull {}: {}", image, status);
                    }
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Failed to pull image '{}': {}", image, e));
                }
            }
        }

        tracing::info!("Image '{}' pulled successfully", image);
        Ok(())
    }

    /// Run a command in a fully isolated, ephemeral Docker container.
    /// Automatically pulls the image if not available locally.
    /// Container is ALWAYS destroyed after execution - no leftovers.
    #[cfg(feature = "docker")]
    pub(in crate::runtime) async fn run_isolated_container(
        &self,
        action_name: &str,
        image: &str,
        cmd: Vec<String>,
        env: Option<Vec<String>>,
        timeout_secs: u64,
        isolation: ContainerIsolation,
    ) -> Result<String> {
        let docker = Self::connect_docker()?;
        let isolation_label = isolation.label();
        let network_access = isolation.network_access();

        // Auto-pull image if not available
        Self::ensure_image(&docker, image).await?;

        let security_opt = Self::docker_security_opts();
        let host_config = match isolation {
            ContainerIsolation::Strict => bollard::models::HostConfig {
                memory: Some(256 * 1024 * 1024),
                memory_swap: Some(256 * 1024 * 1024),
                cpu_period: Some(100_000),
                cpu_quota: Some(50_000),
                pids_limit: Some(64),
                network_mode: Some("none".to_string()),
                readonly_rootfs: Some(true),
                tmpfs: Some(HashMap::from([(
                    "/tmp".to_string(),
                    "size=64M,noexec".to_string(),
                )])),
                cap_drop: Some(vec!["ALL".to_string()]),
                security_opt: Some(security_opt.clone()),
                auto_remove: Some(false),
                ..Default::default()
            },
            ContainerIsolation::Standard => bollard::models::HostConfig {
                memory: Some(512 * 1024 * 1024),
                memory_swap: Some(512 * 1024 * 1024),
                cpu_period: Some(100_000),
                cpu_quota: Some(50_000),
                pids_limit: Some(128),
                network_mode: Some("none".to_string()),
                cap_drop: Some(vec!["ALL".to_string()]),
                security_opt: Some(security_opt.clone()),
                auto_remove: Some(false),
                ..Default::default()
            },
            ContainerIsolation::StandardWithNetwork => bollard::models::HostConfig {
                memory: Some(512 * 1024 * 1024),
                memory_swap: Some(512 * 1024 * 1024),
                cpu_period: Some(100_000),
                cpu_quota: Some(50_000),
                pids_limit: Some(128),
                cap_drop: Some(vec!["ALL".to_string()]),
                security_opt: Some(security_opt),
                auto_remove: Some(false),
                ..Default::default()
            },
        };

        let network_disabled = !network_access;

        let container_config = bollard::models::ContainerCreateBody {
            image: Some(image.to_string()),
            cmd: Some(cmd),
            env,
            labels: Some(Self::sandbox_container_labels(action_name, isolation)),
            host_config: Some(host_config),
            network_disabled: Some(network_disabled),
            working_dir: Some(
                if matches!(
                    isolation,
                    ContainerIsolation::Standard | ContainerIsolation::StandardWithNetwork
                ) {
                    CODE_EXECUTE_SANDBOX_DIR
                } else {
                    "/tmp"
                }
                .to_string(),
            ),
            ..Default::default()
        };

        let create_started = std::time::Instant::now();
        let container = docker.create_container(None, container_config).await;
        let container = match container {
            Ok(container) => {
                crate::metrics::observe_container_lifecycle(
                    action_name,
                    "create",
                    isolation_label,
                    network_access,
                    "ok",
                    create_started.elapsed(),
                );
                container
            }
            Err(e) => {
                crate::metrics::observe_container_lifecycle(
                    action_name,
                    "create",
                    isolation_label,
                    network_access,
                    "error",
                    create_started.elapsed(),
                );
                crate::metrics::observe_container_run(
                    action_name,
                    isolation_label,
                    network_access,
                    "error",
                );
                return Err(anyhow::anyhow!("Failed to create container: {}", e));
            }
        };

        let container_id = container.id.clone();
        self.remember_active_container(&container_id).await;
        tracing::info!(
            "Created isolated container {} for {}",
            &container_id[..12.min(container_id.len())],
            action_name
        );

        // Start container - if this fails, clean up immediately
        let start_started = std::time::Instant::now();
        if let Err(e) = docker.start_container(&container_id, None).await {
            crate::metrics::observe_container_lifecycle(
                action_name,
                "start",
                isolation_label,
                network_access,
                "error",
                start_started.elapsed(),
            );
            let cleanup_started = std::time::Instant::now();
            Self::force_remove_container(&docker, &container_id).await;
            crate::metrics::observe_container_lifecycle(
                action_name,
                "cleanup",
                isolation_label,
                network_access,
                "ok",
                cleanup_started.elapsed(),
            );
            self.forget_active_container(&container_id).await;
            crate::metrics::observe_container_run(
                action_name,
                isolation_label,
                network_access,
                "error",
            );
            return Err(anyhow::anyhow!("Failed to start container: {}", e));
        }
        crate::metrics::observe_container_lifecycle(
            action_name,
            "start",
            isolation_label,
            network_access,
            "ok",
            start_started.elapsed(),
        );

        let wait_started = std::time::Instant::now();
        let exit_code = match tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            docker
                .wait_container(
                    &container_id,
                    None::<bollard::query_parameters::WaitContainerOptions>,
                )
                .try_collect::<Vec<_>>(),
        )
        .await
        {
            Ok(Ok(statuses)) => {
                crate::metrics::observe_container_lifecycle(
                    action_name,
                    "wait",
                    isolation_label,
                    network_access,
                    "ok",
                    wait_started.elapsed(),
                );
                let code = statuses
                    .last()
                    .map(|status| status.status_code)
                    .unwrap_or(0);
                tracing::debug!(
                    "Container {} exited with code {}",
                    &container_id[..12.min(container_id.len())],
                    code
                );
                code
            }
            Ok(Err(bollard::errors::Error::DockerContainerWaitError { code, error })) => {
                crate::metrics::observe_container_lifecycle(
                    action_name,
                    "wait",
                    isolation_label,
                    network_access,
                    "error",
                    wait_started.elapsed(),
                );
                if !error.trim().is_empty() {
                    tracing::debug!(
                        "Container {} exited with wait error {}: {}",
                        &container_id[..12.min(container_id.len())],
                        code,
                        error
                    );
                }
                code
            }
            Ok(Err(e)) => {
                crate::metrics::observe_container_lifecycle(
                    action_name,
                    "wait",
                    isolation_label,
                    network_access,
                    "error",
                    wait_started.elapsed(),
                );
                let cleanup_started = std::time::Instant::now();
                Self::force_remove_container(&docker, &container_id).await;
                crate::metrics::observe_container_lifecycle(
                    action_name,
                    "cleanup",
                    isolation_label,
                    network_access,
                    "ok",
                    cleanup_started.elapsed(),
                );
                self.forget_active_container(&container_id).await;
                crate::metrics::observe_container_run(
                    action_name,
                    isolation_label,
                    network_access,
                    "error",
                );
                return Err(anyhow::anyhow!("Container wait failed: {}", e));
            }
            Err(_) => {
                crate::metrics::observe_container_lifecycle(
                    action_name,
                    "wait",
                    isolation_label,
                    network_access,
                    "timeout",
                    wait_started.elapsed(),
                );
                let cleanup_started = std::time::Instant::now();
                Self::force_remove_container(&docker, &container_id).await;
                crate::metrics::observe_container_lifecycle(
                    action_name,
                    "cleanup",
                    isolation_label,
                    network_access,
                    "ok",
                    cleanup_started.elapsed(),
                );
                self.forget_active_container(&container_id).await;
                crate::metrics::observe_container_run(
                    action_name,
                    isolation_label,
                    network_access,
                    "timeout",
                );
                return Err(anyhow::anyhow!(
                    "Code execution timed out after {} seconds",
                    timeout_secs
                ));
            }
        };

        // Collect stdout and stderr before cleanup
        let logs_started = std::time::Instant::now();
        let logs = docker
            .logs(
                &container_id,
                Some(bollard::query_parameters::LogsOptions {
                    stdout: true,
                    stderr: true,
                    ..Default::default()
                }),
            )
            .try_collect::<Vec<_>>()
            .await
            .unwrap_or_default();
        crate::metrics::observe_container_lifecycle(
            action_name,
            "logs",
            isolation_label,
            network_access,
            "ok",
            logs_started.elapsed(),
        );

        // Always destroy the container - no leftovers
        let cleanup_started = std::time::Instant::now();
        Self::force_remove_container(&docker, &container_id).await;
        crate::metrics::observe_container_lifecycle(
            action_name,
            "cleanup",
            isolation_label,
            network_access,
            "ok",
            cleanup_started.elapsed(),
        );
        self.forget_active_container(&container_id).await;

        let mut stdout = String::new();
        let mut stderr = String::new();
        for log in &logs {
            match log {
                bollard::container::LogOutput::StdOut { message } => {
                    stdout.push_str(&String::from_utf8_lossy(message));
                }
                bollard::container::LogOutput::StdErr { message } => {
                    stderr.push_str(&String::from_utf8_lossy(message));
                }
                _ => {}
            }
        }

        let result = serde_json::json!({
            "output": stdout,
            "error": if stderr.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(stderr.clone()) },
            "exit_code": exit_code,
        });
        crate::metrics::observe_container_run(
            action_name,
            isolation_label,
            network_access,
            if exit_code == 0 { "ok" } else { "error" },
        );

        Ok(serde_json::to_string(&result)?)
    }
}
