//! Durable ownership contract for release updates and runtime migrations.
//!
//! AgentArk uses a two-layer data model:
//! - user-owned data persists across image/runtime upgrades;
//! - system-owned files may be replaced by release artifacts.
//!
//! Runtime code that migrates user-owned data must do so only as an explicit
//! user action or as a versioned migration that preserves a backup path.

pub(crate) const USER_OWNED_SURFACES: &[&str] = &[
    "/app/data/**",
    "/app/config/bootstrap.toml",
    "encrypted settings:* KV",
    "memory/profile/preferences",
    "tasks",
    "/app/data/skills/**",
    "/app/data/cli_skills/**",
];

pub(crate) const SYSTEM_OWNED_SURFACES: &[&str] = &[
    "built-in prompt bundles",
    "frontend/runtime image files",
    "default extension packs",
];

pub(crate) const RELEASE_UPDATE_RULE: &str = "Release updates may replace system-owned files, but must not mutate user-owned data except through explicit user actions or future versioned migrations with backups.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_keeps_user_and_system_layers_separate() {
        assert!(USER_OWNED_SURFACES.contains(&"/app/data/**"));
        assert!(USER_OWNED_SURFACES.contains(&"/app/config/bootstrap.toml"));
        assert!(USER_OWNED_SURFACES.contains(&"encrypted settings:* KV"));
        assert!(USER_OWNED_SURFACES.contains(&"/app/data/skills/**"));
        assert!(USER_OWNED_SURFACES.contains(&"/app/data/cli_skills/**"));
        assert!(RELEASE_UPDATE_RULE.contains("migrations with backups"));
    }
}
