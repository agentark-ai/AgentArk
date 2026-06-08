pub const DEFAULT_RUNTIME_IMAGE: &str = "ghcr.io/agentark-ai/agentark:latest";

const RUNTIME_IMAGE_ENV_KEYS: &[&str] = &[
    "AGENTARK_RUNTIME_IMAGE",
    "AGENTARK_APP_IMAGE",
    "APP_DEPLOY_IMAGE",
    "AGENTARK_RUNNER_IMAGE",
    "AGENTARK_SELF_IMAGE",
];

pub fn configured_runtime_image_from_env() -> Option<String> {
    for key in RUNTIME_IMAGE_ENV_KEYS {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

pub fn default_runtime_image() -> String {
    configured_runtime_image_from_env().unwrap_or_else(|| DEFAULT_RUNTIME_IMAGE.to_string())
}
