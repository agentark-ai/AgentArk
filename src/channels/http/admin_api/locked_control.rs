use super::*;

// - Locked-Mode Server -

/// State for the locked-mode server (before master password is provided)
#[derive(Clone)]
pub(super) struct LockedState {
    config_dir: std::path::PathBuf,
    data_dir: std::path::PathBuf,
    /// Channel to send the derived key back to main.rs
    unlock_tx: Arc<
        tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<Arc<crate::crypto::KeyManager>>>>,
    >,
    /// Rate limiter: track failed attempts per IP
    attempts: Arc<RwLock<HashMap<String, (u32, Instant)>>>,
}

/// Start a minimal HTTP server that only serves the unlock page.
/// Blocks until the user provides the correct password, then returns the key.
pub async fn serve_locked(
    config_dir: &std::path::Path,
    data_dir: &std::path::Path,
) -> Result<Arc<crate::crypto::KeyManager>> {
    let (tx, rx) = tokio::sync::oneshot::channel();

    let locked_state = LockedState {
        config_dir: config_dir.to_path_buf(),
        data_dir: data_dir.to_path_buf(),
        unlock_tx: Arc::new(tokio::sync::Mutex::new(Some(tx))),
        attempts: Arc::new(RwLock::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/", get(locked_page))
        .route("/ui", get(locked_page))
        .route("/ui/", get(locked_page))
        .route("/ui/{*path}", get(locked_page))
        .route("/health", get(locked_health))
        .route("/readiness", get(locked_readiness))
        .route("/unlock", post(handle_unlock))
        .route("/logo.svg", get(serve_logo_svg_locked))
        .with_state(locked_state)
        .layer(DefaultBodyLimit::max(MAX_HTTP_BODY_BYTES));

    let bind_addr = std::env::var("AGENTARK_BIND").unwrap_or("127.0.0.1:8990".to_string());
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    let display_addr = display_addr_for_bind_addr(&bind_addr).unwrap_or_else(|| bind_addr.clone());

    println!();
    println!(" -");
    println!(" - {} is LOCKED -", crate::branding::PRODUCT_NAME);
    println!(" -");
    println!(
        " - Open http://{} to unlock in your browser -",
        display_addr
    );
    println!(" -");
    println!();

    tracing::info!("Locked-mode server listening on {}", bind_addr);

    // Run locked server until unlock succeeds
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    );

    // Race: server vs unlock signal
    tokio::select! {
           result = server => {
               result?;
               Err(anyhow::anyhow!("Locked server exited without unlock"))
           }
           key = rx => {
               let key = key.map_err(|_| anyhow::anyhow!("Unlock channel closed"))?;
    tracing::info!("Master password accepted - proceeding to full startup");
               Ok(key)
           }
       }
}

pub(super) async fn locked_page(uri: Uri) -> Html<String> {
    let requested = requested_unlock_return_path(&uri);
    Html(crate::channels::web::render_unlock_page_html(&requested))
}

pub(super) async fn locked_health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "locked",
        "message": "Master password required to unlock"
    }))
}

pub(super) async fn locked_readiness() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({
            "status": "locked",
            "ready": false,
            "message": "Master password required to unlock"
        })),
    )
        .into_response()
}

pub(super) async fn serve_logo_svg_locked() -> Response {
    let svg = include_str!("../../../../assets/logo.svg");
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "image/svg+xml")],
        svg,
    )
        .into_response()
}

pub(super) fn requested_unlock_return_path(uri: &Uri) -> String {
    let path = uri.path();
    if path == "/" || path.starts_with("/ui") {
        let mut target = path.to_string();
        if let Some(query) = uri.query() {
            target.push('?');
            target.push_str(query);
        }
        target
    } else {
        "/".to_string()
    }
}

#[derive(Deserialize)]
pub(super) struct UnlockRequest {
    password: String,
}

pub(super) async fn handle_unlock(
    State(state): State<LockedState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<UnlockRequest>,
) -> Response {
    let ip = addr.ip().to_string();

    // Rate limit: max 5 attempts per minute per IP
    {
        let attempts = state.attempts.read().await;
        if let Some((count, since)) = attempts.get(&ip) {
            if since.elapsed() < std::time::Duration::from_secs(60) && *count >= 5 {
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(
                        serde_json::json!({ "error": "Too many attempts. Try again in 1 minute." }),
                    ),
                )
                    .into_response();
            }
        }
    }

    let master_mgr =
        crate::crypto::master::MasterPasswordManager::new(&state.config_dir, &state.data_dir);

    match master_mgr.unlock(&req.password) {
        Ok(key) => {
            // Send key to main.rs via channel
            let mut tx_guard = state.unlock_tx.lock().await;
            if let Some(tx) = tx_guard.take() {
                let _ = tx.send(key);
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "message": "Unlocked successfully. Starting up..."
                })),
            )
                .into_response()
        }
        Err(_) => {
            // Track failed attempt
            let mut attempts = state.attempts.write().await;
            let entry = attempts.entry(ip).or_insert((0, Instant::now()));
            if entry.1.elapsed() >= std::time::Duration::from_secs(60) {
                *entry = (1, Instant::now());
            } else {
                entry.0 += 1;
            }
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Invalid password"
                })),
            )
                .into_response()
        }
    }
}

// - Security Endpoints -

pub(super) async fn security_status(State(state): State<AppState>) -> Response {
    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let master_mgr = crate::crypto::master::MasterPasswordManager::new(&config_dir, &data_dir);
    let is_set = master_mgr.is_password_set();
    let bootstrap_active = master_mgr.is_bootstrap_password_active().unwrap_or(false);
    let install_managed_active = master_mgr
        .is_install_managed_password_active()
        .unwrap_or(false);
    let internal_service_tokens =
        crate::clients::describe_internal_service_tokens_async(&config_dir)
            .await
            .unwrap_or_default();
    let internal_service_rotation_supported = !internal_service_tokens
        .iter()
        .any(|item| item.managed_by_env);

    let warning = if !is_set {
        Some(
            "Encryption keys are stored as plain files. Set a master password for stronger protection.",
        )
    } else if bootstrap_active {
        Some(
            "Using a per-install bootstrap password. Set a custom master password to fully own recovery and rotation.",
        )
    } else if install_managed_active {
        Some(
            "Using an install-managed encryption secret. Set a custom master password to fully own recovery and remote access login.",
        )
    } else {
        None
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "master_password_set": is_set,
            "custom_master_password_set": is_set && !bootstrap_active && !install_managed_active,
            "using_default": bootstrap_active || install_managed_active,
            "bootstrap_password_active": bootstrap_active,
            "install_managed_password_active": install_managed_active,
            "encryption_mode": if install_managed_active { "install_managed_secret" } else if is_set { "password" } else { "keyfile" },
            "security_warning": warning,
            "internal_service_rotation_supported": internal_service_rotation_supported,
            "internal_service_tokens": internal_service_tokens
        })),
    )
        .into_response()
}

pub(super) fn current_runtime_encryption_key(
    config_dir: &FsPath,
) -> anyhow::Result<std::sync::Arc<crate::crypto::KeyManager>> {
    if let Some(key) = crate::core::runtime::config::global_key_manager() {
        return Ok(key);
    }
    if crate::crypto::master::MasterPasswordManager::docker_stack_requires_install_master_secret() {
        let secret = crate::crypto::master::MasterPasswordManager::read_install_master_secret()?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Install-managed encryption secret is missing at {}",
                    crate::crypto::master::INSTALL_MASTER_SECRET_PATH
                )
            })?;
        let master_mgr = crate::crypto::master::MasterPasswordManager::new(config_dir, config_dir);
        return master_mgr.unlock(&secret);
    }
    Ok(std::sync::Arc::new(
        crate::crypto::KeyManager::load_or_create(&config_dir.join(".keyfile"))?,
    ))
}

pub(super) async fn current_storage_encryption_key(
    state: &AppState,
) -> anyhow::Result<std::sync::Arc<crate::crypto::KeyManager>> {
    let agent = state.agent.read().await;
    Ok(agent.encrypted_storage.current_key_manager())
}

pub(super) async fn rotate_application_encryption<F>(
    state: &AppState,
    config_dir: &FsPath,
    old_secrets_key: std::sync::Arc<crate::crypto::KeyManager>,
    old_storage_key: std::sync::Arc<crate::crypto::KeyManager>,
    new_key: std::sync::Arc<crate::crypto::KeyManager>,
    commit_metadata: F,
) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    let old_mgr = crate::core::runtime::config::SecureConfigManager::with_key_manager(
        config_dir,
        old_secrets_key.clone(),
    );
    let new_mgr = crate::core::runtime::config::SecureConfigManager::with_key_manager(
        config_dir,
        new_key.clone(),
    );
    let secrets = old_mgr.with_secrets_lock(|manager| manager.load_secrets_unlocked())?;
    let agent = state.agent.write().await;

    new_mgr.save_secrets_unlocked_for_rekey(&secrets)?;
    if let Err(storage_err) = agent
        .encrypted_storage
        .reencrypt_all_sensitive_data(old_storage_key.clone(), new_key.clone())
        .await
    {
        let rollback_err = old_mgr.save_secrets_unlocked(&secrets).err();
        return Err(match rollback_err {
            Some(rollback_err) => anyhow::anyhow!(
                "Encrypted storage rekey failed: {}. Settings secret rollback also failed: {}",
                storage_err,
                rollback_err
            ),
            None => anyhow::anyhow!("Encrypted storage rekey failed: {}", storage_err),
        });
    }

    if let Err(commit_err) = commit_metadata() {
        let storage_rollback_err = agent
            .encrypted_storage
            .reencrypt_all_sensitive_data(new_key.clone(), old_storage_key)
            .await
            .err();
        let secrets_rollback_err = old_mgr.save_secrets_unlocked(&secrets).err();
        return Err(match (storage_rollback_err, secrets_rollback_err) {
            (Some(storage_rollback_err), Some(secrets_rollback_err)) => anyhow::anyhow!(
                "Metadata update failed: {}. Encrypted storage rollback also failed: {}. Settings secret rollback also failed: {}",
                commit_err,
                storage_rollback_err,
                secrets_rollback_err
            ),
            (Some(storage_rollback_err), None) => anyhow::anyhow!(
                "Metadata update failed: {}. Encrypted storage rollback also failed: {}",
                commit_err,
                storage_rollback_err
            ),
            (None, Some(secrets_rollback_err)) => anyhow::anyhow!(
                "Metadata update failed: {}. Settings secret rollback also failed: {}",
                commit_err,
                secrets_rollback_err
            ),
            (None, None) => anyhow::anyhow!("Metadata update failed: {}", commit_err),
        });
    }

    crate::core::runtime::config::set_global_key_manager(new_key);
    Ok(())
}

#[derive(Deserialize)]
pub(super) struct SetPasswordRequest {
    password: String,
}

pub(super) async fn set_master_password(
    State(state): State<AppState>,
    Json(req): Json<SetPasswordRequest>,
) -> Response {
    if req.password.len() < 8 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Password must be at least 8 characters"
            })),
        )
            .into_response();
    }

    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let master_mgr = crate::crypto::master::MasterPasswordManager::new(&config_dir, &data_dir);
    let bootstrap_active = master_mgr.is_bootstrap_password_active().unwrap_or(false);
    let install_managed_active = master_mgr
        .is_install_managed_password_active()
        .unwrap_or(false);

    if master_mgr.is_password_set() && !bootstrap_active && !install_managed_active {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "Master password already set. Use change-password instead."
            })),
        )
            .into_response();
    }

    let old_secrets_key = match current_runtime_encryption_key(&config_dir) {
        Ok(key) => key,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to load current encryption key: {}", e)
                })),
            )
                .into_response();
        }
    };
    let old_storage_key = match current_storage_encryption_key(&state).await {
        Ok(key) => key,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to load current storage encryption key: {}", e)
                })),
            )
                .into_response();
        }
    };

    match master_mgr.prepare_password(&req.password) {
        Ok(prepared) => {
            let new_key = prepared.key_manager.clone();
            if let Err(e) = rotate_application_encryption(
                &state,
                &config_dir,
                old_secrets_key,
                old_storage_key,
                new_key.clone(),
                || master_mgr.commit_prepared_password(prepared),
            )
            .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!("Failed to set password safely: {}", e)
                    })),
                )
                    .into_response();
            }

            let _ = tokio::fs::remove_file(data_dir.join("encryption.key")).await;
            let _ = tokio::fs::remove_file(config_dir.join(".keyfile")).await;
            tracing::info!("Master password set and applied in-memory (no restart needed)");

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "message": "Master password set and applied.",
                    "restart_scheduled": false
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to set password: {}", e)
            })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct ChangePasswordRequest {
    current_password: String,
    new_password: String,
}

pub(super) async fn change_master_password(
    State(state): State<AppState>,
    Json(req): Json<ChangePasswordRequest>,
) -> Response {
    if req.new_password.len() < 8 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "New password must be at least 8 characters"
            })),
        )
            .into_response();
    }

    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let master_mgr = crate::crypto::master::MasterPasswordManager::new(&config_dir, &data_dir);
    let bootstrap_active = master_mgr.is_bootstrap_password_active().unwrap_or(false);
    let install_managed_active = master_mgr
        .is_install_managed_password_active()
        .unwrap_or(false);

    let current_pw = if req.current_password.is_empty() {
        if bootstrap_active {
            match master_mgr.bootstrap_password_if_active() {
                Ok(Some(pw)) => pw,
                _ => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "error": "Current password is required"
                        })),
                    )
                        .into_response();
                }
            }
        } else if install_managed_active {
            match crate::crypto::master::MasterPasswordManager::read_install_master_secret() {
                Ok(Some(pw)) => pw,
                Ok(None) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "error": "Install-managed encryption secret is missing"
                        })),
                    )
                        .into_response();
                }
                Err(error) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "error": error.to_string()
                        })),
                    )
                        .into_response();
                }
            }
        } else {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Current password is required"
                })),
            )
                .into_response();
        }
    } else {
        req.current_password.clone()
    };

    // Verify current password
    let old_secrets_key = match master_mgr.unlock(&current_pw) {
        Ok(key) => key,
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Current password is incorrect"
                })),
            )
                .into_response();
        }
    };
    let old_storage_key = match current_storage_encryption_key(&state).await {
        Ok(key) => key,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to load current storage encryption key: {}", e)
                })),
            )
                .into_response();
        }
    };

    match master_mgr.prepare_password(&req.new_password) {
        Ok(prepared) => {
            let new_key = prepared.key_manager.clone();
            if let Err(e) = rotate_application_encryption(
                &state,
                &config_dir,
                old_secrets_key,
                old_storage_key,
                new_key.clone(),
                || master_mgr.commit_prepared_password(prepared),
            )
            .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!("Failed to change password safely: {}", e)
                    })),
                )
                    .into_response();
            }

            tracing::info!("Master password changed and applied in-memory");

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "message": "Password changed and applied.",
                    "restart_scheduled": false
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to change password: {}", e)
            })),
        )
            .into_response(),
    }
}

pub(super) async fn remove_master_password(
    State(state): State<AppState>,
    Json(req): Json<SetPasswordRequest>,
) -> Response {
    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let master_mgr = crate::crypto::master::MasterPasswordManager::new(&config_dir, &data_dir);

    if !master_mgr.is_password_set() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "No master password is set"
            })),
        )
            .into_response();
    }

    // Verify password first
    let old_secrets_key = match master_mgr.unlock(&req.password) {
        Ok(key) => key,
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Password is incorrect"
                })),
            )
                .into_response();
        }
    };
    let old_storage_key = match current_storage_encryption_key(&state).await {
        Ok(key) => key,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to load current storage encryption key: {}", e)
                })),
            )
                .into_response();
        }
    };

    let prepared_install_secret =
        if crate::crypto::master::MasterPasswordManager::docker_stack_requires_install_master_secret(
        ) {
            let secret =
                match crate::crypto::master::MasterPasswordManager::read_install_master_secret() {
                    Ok(Some(secret)) => secret,
                    Ok(None) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({
                                "error": "Install-managed encryption secret is missing"
                            })),
                        )
                            .into_response();
                    }
                    Err(error) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({
                                "error": error.to_string()
                            })),
                        )
                            .into_response();
                    }
                };
            match master_mgr.prepare_install_managed_password(&secret) {
                Ok(prepared) => Some(prepared),
                Err(error) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": format!("Failed to prepare install-managed encryption secret: {}", error)
                        })),
                    )
                        .into_response();
                }
            }
        } else {
            None
        };

    let keyfile_key = if prepared_install_secret.is_none() {
        match master_mgr.prepare_keyfile_encryption() {
            Ok(key) => Some(key),
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!("Failed to remove password: {}", e)
                    })),
                )
                    .into_response();
            }
        }
    } else {
        None
    };
    let new_key = prepared_install_secret
        .as_ref()
        .map(|prepared| prepared.key_manager.clone())
        .or(keyfile_key)
        .expect("remove password target key should be prepared");
    let reverting_to_install_managed = prepared_install_secret.is_some();
    if let Err(e) = rotate_application_encryption(
        &state,
        &config_dir,
        old_secrets_key,
        old_storage_key,
        new_key.clone(),
        || match prepared_install_secret {
            Some(prepared) => master_mgr.commit_prepared_password(prepared),
            None => master_mgr.commit_password_removal(),
        },
    )
    .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to remove password safely: {}", e)
            })),
        )
            .into_response();
    }

    tracing::info!("Master password removed or reverted to install-managed encryption in-memory");
    if tunnel_auth::control_plane_tunnel_is_active(&state).await {
        tunnel::stop_tunnel_internal(&state).await;
        tracing::info!(
            "Stopped active control-plane tunnel after removing the custom master password"
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "message": if reverting_to_install_managed { "Master password reverted to install-managed encryption." } else { "Master password removed." },
            "restart_scheduled": false
        })),
    )
        .into_response()
}
