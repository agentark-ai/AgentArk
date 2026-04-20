use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::Rng;
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InternalServiceKind {
    Executor,
    Workspace,
}

impl InternalServiceKind {
    pub(crate) fn env_var(self) -> &'static str {
        match self {
            Self::Executor => "AGENTARK_EXECUTOR_TOKEN",
            Self::Workspace => "AGENTARK_WORKSPACE_TOKEN",
        }
    }

    fn token_file_name(self) -> &'static str {
        match self {
            Self::Executor => ".internal-executor-token",
            Self::Workspace => ".internal-workspace-token",
        }
    }

    fn token_prefix(self) -> &'static str {
        match self {
            Self::Executor => "ak_int_exec_",
            Self::Workspace => "ak_int_ws_",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Executor => "executor",
            Self::Workspace => "workspace",
        }
    }

    fn id(self) -> &'static str {
        self.label()
    }

    fn token_path(self, config_dir: &Path) -> PathBuf {
        config_dir.join(self.token_file_name())
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct InternalServiceTokenStatus {
    pub id: String,
    pub label: String,
    pub env_var: String,
    pub managed_by_env: bool,
    pub configured: bool,
    pub updated_at: Option<String>,
}

pub(crate) fn load_or_create_internal_service_token(
    config_dir: &Path,
    service: InternalServiceKind,
) -> Result<String> {
    if let Some(token) = read_env_token(service) {
        persist_token_if_needed(config_dir, service, &token)?;
        return Ok(token);
    }

    std::fs::create_dir_all(config_dir).with_context(|| {
        format!(
            "Failed to create config directory for {} internal auth at {}",
            service.label(),
            config_dir.display()
        )
    })?;

    let token_path = config_dir.join(service.token_file_name());
    if let Some(token) = read_token_file(&token_path)? {
        return Ok(token);
    }

    let token = generate_internal_service_token(service);
    match create_token_file(&token_path, &token) {
        Ok(()) => {
            tracing::info!(
                "Generated {} internal service token at {}",
                service.label(),
                token_path.display()
            );
            Ok(token)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => read_token_file(&token_path)?
            .ok_or_else(|| {
                anyhow!(
                    "{} token file became unreadable during creation race",
                    service.label()
                )
            }),
        Err(error) => Err(anyhow!(
            "Failed to create {} token file at {}: {}",
            service.label(),
            token_path.display(),
            error
        )),
    }
}

pub(crate) fn load_internal_service_token_from_default_config_dir(
    service: InternalServiceKind,
) -> Option<String> {
    let config_dir = resolve_default_config_dir()?;
    match load_or_create_internal_service_token(&config_dir, service) {
        Ok(token) => Some(token),
        Err(error) => {
            tracing::warn!(
                "Failed to resolve {} internal auth token from {}: {}",
                service.label(),
                config_dir.display(),
                error
            );
            None
        }
    }
}

fn resolve_default_config_dir() -> Option<PathBuf> {
    std::env::var("AGENTARK_CONFIG")
        .ok()
        .map(PathBuf::from)
        .or_else(|| crate::branding::project_dirs().map(|dirs| dirs.config_dir().to_path_buf()))
}

fn read_env_token(service: InternalServiceKind) -> Option<String> {
    std::env::var(service.env_var())
        .ok()
        .and_then(|value| normalize_token(&value))
}

fn read_token_file(path: &Path) -> Result<Option<String>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(value) => value,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(anyhow!(
                "Failed to read internal auth token file {}: {}",
                path.display(),
                error
            ));
        }
    };
    normalize_token(&raw).map(Some).ok_or_else(|| {
        anyhow!(
            "Internal auth token file {} is empty or invalid",
            path.display()
        )
    })
}

fn persist_token_if_needed(
    config_dir: &Path,
    service: InternalServiceKind,
    token: &str,
) -> Result<()> {
    std::fs::create_dir_all(config_dir).with_context(|| {
        format!(
            "Failed to create config directory for {} internal auth at {}",
            service.label(),
            config_dir.display()
        )
    })?;
    let token_path = config_dir.join(service.token_file_name());
    match read_token_file(&token_path)? {
        Some(existing) if existing == token => return Ok(()),
        _ => {}
    }
    crate::crypto::atomic_write_file(&token_path, format!("{token}\n").as_bytes())?;
    restrict_token_file_permissions(&token_path)?;
    Ok(())
}

fn create_token_file(path: &Path, token: &str) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(token.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    restrict_token_file_permissions(path)?;
    Ok(())
}

fn restrict_token_file_permissions(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn normalize_token(raw: &str) -> Option<String> {
    let token = raw.trim();
    if token.is_empty() || token.eq_ignore_ascii_case("change-me") {
        None
    } else {
        Some(token.to_string())
    }
}

pub(crate) fn read_persisted_internal_service_token(
    config_dir: &Path,
    service: InternalServiceKind,
) -> Result<Option<String>> {
    read_token_file(&service.token_path(config_dir))
}

pub(crate) fn restore_internal_service_token(
    config_dir: &Path,
    service: InternalServiceKind,
    previous: Option<&str>,
) -> Result<()> {
    let token_path = service.token_path(config_dir);
    match previous {
        Some(token) => persist_token_if_needed(config_dir, service, token),
        None => {
            if token_path.exists() {
                std::fs::remove_file(&token_path).with_context(|| {
                    format!(
                        "Failed to remove {} internal auth token file {} during rollback",
                        service.label(),
                        token_path.display()
                    )
                })?;
            }
            Ok(())
        }
    }
}

pub(crate) fn rotate_internal_service_token(
    config_dir: &Path,
    service: InternalServiceKind,
) -> Result<String> {
    if read_env_token(service).is_some() {
        anyhow::bail!(
            "{} internal credential is managed by {}. Rotate it in the deployment environment instead.",
            service.label(),
            service.env_var()
        );
    }
    let token = generate_internal_service_token(service);
    persist_token_if_needed(config_dir, service, &token)?;
    Ok(token)
}

pub(crate) fn describe_internal_service_tokens(
    config_dir: &Path,
) -> Result<Vec<InternalServiceTokenStatus>> {
    [
        InternalServiceKind::Executor,
        InternalServiceKind::Workspace,
    ]
    .into_iter()
    .map(|service| describe_internal_service_token(config_dir, service))
    .collect()
}

fn describe_internal_service_token(
    config_dir: &Path,
    service: InternalServiceKind,
) -> Result<InternalServiceTokenStatus> {
    let token_path = service.token_path(config_dir);
    let managed_by_env = read_env_token(service).is_some();
    let configured = managed_by_env || read_token_file(&token_path)?.is_some();
    let updated_at = std::fs::metadata(&token_path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .map(|time| chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339());
    Ok(InternalServiceTokenStatus {
        id: service.id().to_string(),
        label: service.label().to_string(),
        env_var: service.env_var().to_string(),
        managed_by_env,
        configured,
        updated_at,
    })
}

fn generate_internal_service_token(service: InternalServiceKind) -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    format!(
        "{}{}",
        service.token_prefix(),
        URL_SAFE_NO_PAD.encode(bytes)
    )
}

#[cfg(test)]
mod tests {
    use super::{
        load_or_create_internal_service_token, read_persisted_internal_service_token,
        rotate_internal_service_token, InternalServiceKind,
    };

    #[test]
    fn creates_and_reuses_executor_token_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first =
            load_or_create_internal_service_token(dir.path(), InternalServiceKind::Executor)
                .expect("first token");
        let second =
            load_or_create_internal_service_token(dir.path(), InternalServiceKind::Executor)
                .expect("second token");
        assert_eq!(first, second);
        assert!(first.starts_with("ak_int_exec_"));
    }

    #[test]
    fn rotates_workspace_token_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first =
            load_or_create_internal_service_token(dir.path(), InternalServiceKind::Workspace)
                .expect("first token");
        let rotated = rotate_internal_service_token(dir.path(), InternalServiceKind::Workspace)
            .expect("rotated token");
        let persisted =
            read_persisted_internal_service_token(dir.path(), InternalServiceKind::Workspace)
                .expect("persisted token")
                .expect("token file present");
        assert_ne!(first, rotated);
        assert_eq!(rotated, persisted);
        assert!(rotated.starts_with("ak_int_ws_"));
    }
}
