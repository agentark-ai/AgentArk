use super::super::*;

impl ActionRuntime {
    #[cfg(test)]
    pub(in crate::runtime) fn find_project_root_from_path(start: &Path) -> Option<PathBuf> {
        let mut dir = if start.is_file() {
            start.parent()?
        } else {
            start
        };
        loop {
            if dir.join("Cargo.toml").exists() {
                return Some(dir.to_path_buf());
            }
            dir = dir.parent()?;
        }
    }

    pub(in crate::runtime) fn bundled_skill_dirs(&self) -> Vec<PathBuf> {
        // Repo-local bundled markdown skills are disabled for this install.
        Vec::new()
    }

    pub(in crate::runtime) fn is_runtime_owned_bundled_dir(path: &Path) -> bool {
        let _ = path;
        false
    }

    pub(in crate::runtime) async fn delete_runtime_owned_bundled_skill_dir(
        &self,
        name: &str,
    ) -> Result<()> {
        for bundled_dir in self.bundled_skill_dirs() {
            if !Self::is_runtime_owned_bundled_dir(&bundled_dir) {
                continue;
            }
            let action_dir = bundled_dir.join(name);
            if action_dir.exists() {
                tokio::fs::remove_dir_all(&action_dir).await?;
            }
        }
        Ok(())
    }

    pub async fn new(config_dir: &Path, data_dir: &Path) -> Result<Self> {
        let settings = SecureConfigManager::new_with_data_dir(config_dir, Some(data_dir)).ok();
        let config_path = config_dir.join("runtime.toml");
        let config: RuntimeConfig = if let Some(manager) = settings
            .as_ref()
            .filter(|manager| manager.uses_storage_backend())
        {
            match manager.load_encrypted_json::<RuntimeConfig>(
                crate::core::runtime::config::SETTINGS_RUNTIME_KEY,
            ) {
                Ok(Some(config)) => config,
                Ok(None) => {
                    let default = RuntimeConfig::default();
                    manager.save_encrypted_json(
                        crate::core::runtime::config::SETTINGS_RUNTIME_KEY,
                        &default,
                    )?;
                    default
                }
                Err(error) => {
                    tracing::warn!(
                        "Failed to load runtime config from settings storage: {}",
                        error
                    );
                    RuntimeConfig::default()
                }
            }
        } else if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            toml::from_str(&content)?
        } else {
            let default = RuntimeConfig::default();
            let content = toml::to_string_pretty(&default)?;
            std::fs::write(&config_path, content)?;
            default
        };

        // User-owned skills go in the data dir and survive release updates.
        let actions_dir = data_dir.join("skills");
        std::fs::create_dir_all(&actions_dir)?;
        let cli_skills_dir = data_dir.join("cli_skills");
        std::fs::create_dir_all(&cli_skills_dir)?;
        let disabled_actions_file = data_dir.join("disabled_actions.json");
        let disabled_actions =
            Self::load_disabled_actions(&disabled_actions_file, settings.as_ref());
        let action_reviews_file = data_dir.join("action_reviews.json");
        let action_reviews = Self::load_action_reviews(&action_reviews_file, settings.as_ref());
        let removed_bundled_actions_file = data_dir.join("removed_bundled_actions.json");
        let removed_bundled_actions =
            Self::load_removed_bundled_actions(&removed_bundled_actions_file, settings.as_ref());

        let snapshot_dir = data_dir.join(&config.snapshot_dir);
        std::fs::create_dir_all(&snapshot_dir)?;

        let transactions = TransactionManager::new(snapshot_dir);

        let runtime = Self {
            config,
            transactions: tokio::sync::Mutex::new(transactions),
            actions: tokio::sync::RwLock::new(HashMap::new()),
            disabled_actions: tokio::sync::RwLock::new(disabled_actions),
            disabled_actions_file,
            action_reviews: tokio::sync::RwLock::new(action_reviews),
            action_reviews_file,
            capability_run_contexts: tokio::sync::RwLock::new(HashMap::new()),
            removed_bundled_actions: tokio::sync::RwLock::new(removed_bundled_actions),
            removed_bundled_actions_file,
            actions_dir: actions_dir.clone(),
            cli_skills_dir,
            config_dir: config_dir.to_path_buf(),
            task_queue: None,
            action_guard: None,
            safety_engine: None,
            auto_approved_actions: std::sync::RwLock::new(HashSet::new()),
            tool_args_guard_config: std::sync::RwLock::new(Default::default()),
            storage: None,
            embedding_client: None,
            current_user_id: None,
            mcp_registry: None,
            plugin_registry: None,
            extension_pack_registry: None,
            #[cfg(feature = "docker")]
            active_sandbox_containers: tokio::sync::RwLock::new(HashSet::new()),
            #[cfg(feature = "docker")]
            container_reaper_status: tokio::sync::RwLock::new(ContainerReaperStatus::default()),
        };

        Ok(runtime)
    }

    /// Set shared task queue reference (called from Agent::init)
    pub fn set_task_queue(
        &mut self,
        tasks: std::sync::Arc<tokio::sync::RwLock<crate::core::TaskQueue>>,
    ) {
        self.task_queue = Some(tasks);
    }

    /// Set action security guard (called from Agent::init before load_all_actions)
    pub fn set_action_guard(&mut self, guard: std::sync::Arc<crate::security::ActionGuard>) {
        self.action_guard = Some(guard);
    }

    /// Set safety engine for dynamic integration action registration.
    pub fn set_safety_engine(&mut self, safety: std::sync::Arc<crate::safety::SafetyEngine>) {
        self.safety_engine = Some(safety);
    }

    /// Update the effective action-name overrides that can skip approval prompts.
    pub fn set_auto_approved_actions(&self, actions: &[String]) {
        let approved = crate::core::runtime::config::sanitize_auto_approve_actions(actions)
            .into_iter()
            .collect::<HashSet<_>>();
        if let Ok(mut set) = self.auto_approved_actions.write() {
            *set = approved;
        }
    }

    pub fn set_tool_args_guard_config(
        &self,
        config: crate::security::tool_args_guard::ToolArgsGuardConfig,
    ) {
        if let Ok(mut current) = self.tool_args_guard_config.write() {
            *current = config;
        }
    }

    pub(in crate::runtime) fn tool_args_guard_config(
        &self,
    ) -> crate::security::tool_args_guard::ToolArgsGuardConfig {
        self.tool_args_guard_config
            .read()
            .map(|config| config.clone())
            .unwrap_or_default()
    }

    /// Set shared storage reference for expense/entity operations (called from Agent::init)
    pub fn set_storage(&mut self, storage: crate::storage::Storage) {
        self.storage = Some(storage);
    }

    pub fn set_embedding_client(
        &mut self,
        embedding_client: Option<std::sync::Arc<crate::core::EmbeddingClient>>,
    ) {
        self.embedding_client = embedding_client;
    }

    pub fn storage(&self) -> Option<crate::storage::Storage> {
        self.storage.clone()
    }

    /// Set the active user identifier (DID). Called from `Agent::init` after
    /// the identity is loaded so per-user actions (e.g. ArkOrbit) can resolve
    /// scope without it being threaded through every tool argument.
    pub fn set_current_user_id(&mut self, user_id: impl Into<String>) {
        let value = user_id.into();
        self.current_user_id = if value.trim().is_empty() {
            None
        } else {
            Some(value)
        };
    }

    pub(in crate::runtime) fn current_user_id(&self) -> Result<&str> {
        self.current_user_id.as_deref().ok_or_else(|| {
            anyhow::anyhow!("Active user identity is not configured for the runtime")
        })
    }

    pub(in crate::runtime) fn arkorbit_service(
        &self,
    ) -> Result<crate::core::arkorbit::ArkOrbitService> {
        let storage = self
            .storage
            .clone()
            .ok_or_else(|| anyhow::anyhow!("ArkOrbit requires storage to be configured"))?;
        Ok(crate::core::arkorbit::ArkOrbitService::with_filesystem(
            storage,
            self.data_dir(),
        ))
    }

    /// Set MCP registry (called from Agent::init)
    pub fn set_mcp_registry(
        &mut self,
        registry: std::sync::Arc<tokio::sync::RwLock<crate::mcp::registry::McpRegistry>>,
    ) {
        self.mcp_registry = Some(registry);
    }

    /// Set plugin registry (called from Agent::init)
    pub fn set_plugin_registry(
        &mut self,
        registry: std::sync::Arc<tokio::sync::RwLock<crate::plugins::registry::PluginRegistry>>,
    ) {
        self.plugin_registry = Some(registry);
    }

    /// Set extension-pack registry (called from Agent::init)
    pub fn set_extension_pack_registry(
        &mut self,
        registry: std::sync::Arc<
            tokio::sync::RwLock<crate::extension_packs::ExtensionPackRegistry>,
        >,
    ) {
        self.extension_pack_registry = Some(registry);
    }

    pub(in crate::runtime) async fn unapproved_permissions_for_action(
        &self,
        action: &ActionDef,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Vec<crate::security::action_guard::Permission> {
        let _ = (arguments, auth_context);
        let _ = action;
        Vec::new()
    }

    pub(in crate::runtime) fn build_permission_requirement_error(
        action_name: &str,
        permissions: &[crate::security::action_guard::Permission],
    ) -> String {
        let perm_names = permissions
            .iter()
            .map(Self::permission_display_label)
            .collect::<Vec<_>>()
            .join(", ");
        let guidance = if crate::core::runtime::config::AUTO_APPROVE_BLOCKED.contains(&action_name)
        {
            "This permission always requires an explicit approval for the current run."
        } else {
            "Approve the prompt to continue this run."
        };
        format!(
            "Action '{}' requires approval before execution because it needs unapproved permissions: {}. {}",
            action_name, perm_names, guidance
        )
    }

    pub(in crate::runtime) fn permission_display_label(
        permission: &crate::security::action_guard::Permission,
    ) -> String {
        permission
            .to_string()
            .split(['_', '-'])
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Load all actions (builtin + user). Call AFTER set_action_guard.
    pub async fn load_all_actions(&self) -> Result<()> {
        // Load built-in actions
        self.load_builtin_actions().await?;

        // Load user-added skills from data dir
        let has_actions_dir = tokio::fs::metadata(&self.actions_dir)
            .await
            .map(|meta| meta.is_dir())
            .unwrap_or(false);
        if has_actions_dir {
            tracing::info!("Loading user skills from {:?}", self.actions_dir);
            self.load_markdown_actions(&self.actions_dir, ActionSource::Custom)
                .await?;
        }

        let has_cli_skills_dir = tokio::fs::metadata(&self.cli_skills_dir)
            .await
            .map(|meta| meta.is_dir())
            .unwrap_or(false);
        if has_cli_skills_dir {
            tracing::info!(
                "Loading installed CLI skills from {:?}",
                self.cli_skills_dir
            );
            self.load_cli_skill_actions().await?;
        }

        Ok(())
    }

    pub(in crate::runtime) fn research_report_composer_workflow() -> &'static str {
        r#"---
name: research_report_compose
description: Turn gathered research evidence into a polished, citation-backed report with clean sections, tables, and chart blocks when useful.
version: "1.0.0"
---

# Deep Research Report Composer

search: none

Use this workflow after evidence has already been gathered by deep research, document review, or user-provided source notes. Do not invent facts, citations, dates, statistics, source titles, URLs, chart data, or confidence levels. If the evidence is thin or contradictory, say so plainly and preserve the uncertainty.

## Inputs Required
- `evidence`: source notes, excerpts, prior research output, or structured evidence to synthesize.

Optional inputs:
- `audience`: who will read the report.
- `report_type`: policy brief, market landscape, technical comparison, implementation plan, investment memo, literature review, or another user-implied type.
- `output_format`: markdown, report, or brief.
- `include_charts`: whether to include `agentark-chart` JSON fences when the evidence supports useful charts.

## Workflow
1. Infer the user's underlying decision or knowledge need from the evidence and optional fields.
2. Choose report sections by meaning, not by anticipated wording. Adapt to the domain instead of using a fixed template.
3. Group related evidence into a small number of themes. Remove duplicate points.
4. Make the report decision-grade: compare positions, implications, constraints, counterarguments, practical options, uncertainty, and what would change the conclusion.
5. Use Markdown tables for comparisons, phased plans, risk allocations, scoring matrices, or evidence summaries when they make the report easier to scan.
6. Include chart fences only when the evidence contains concrete comparable values, such as counts, percentages, dates, ratings, ranges, or grouped categories. Use `agentark-chart` JSON with `title`, `type`, `x`, `series`, `data`, and `height`. Do not chart qualitative claims without numeric support.
7. Cite material claims with the source numbers, names, or identifiers present in the evidence. Do not fabricate citations.
8. End with evidence gaps or verification needs when support is incomplete.

## Output Format
# Research: [clear report title]

[Executive summary in 1-3 short paragraphs.]

## 1. [Section based on the evidence]

[Synthesis with citations.]

[Table or chart only when it clarifies the report.]

## 2. [Next meaningful section]

[Continue with concise, cited analysis.]

## Recommendations or Next Steps

[Actionable options or decisions, grounded in evidence.]

## Evidence Gaps

[Missing, uncertain, or conflicting evidence.]
"#
    }
}
