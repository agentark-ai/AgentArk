use super::super::*;

impl ActionRuntime {
    /// Load markdown-defined actions from a directory
    /// Looks for SKILL.md files in subdirectories.
    /// These are registered as workflow actions for LLM-driven execution
    pub async fn load_markdown_actions(&self, dir: &Path, source: ActionSource) -> Result<()> {
        let dir_exists = tokio::fs::metadata(dir)
            .await
            .map(|meta| meta.is_dir())
            .unwrap_or(false);
        if !dir_exists {
            return Ok(());
        }

        // Read directory entries
        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Could not read skills directory {:?}: {}", dir, e);
                return Ok(());
            }
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let is_dir = entry
                .file_type()
                .await
                .map(|file_type| file_type.is_dir())
                .unwrap_or(false);
            if !is_dir {
                continue;
            }
            let path = entry.path();
            let md_file = path.join("SKILL.md");
            let md_exists = tokio::fs::metadata(&md_file)
                .await
                .map(|meta| meta.is_file())
                .unwrap_or(false);
            if !md_exists {
                continue;
            }

            match self.parse_action_md(&md_file, source.clone()).await {
                Ok((info, workflow_content, frontmatter)) => {
                    if source == ActionSource::Bundled {
                        let removed = self.removed_bundled_actions.read().await;
                        if removed.contains(&info.name) {
                            tracing::info!(
                                "Skipped deleted bundled action '{}' from {:?}",
                                info.name,
                                md_file
                            );
                            continue;
                        }
                        let disabled = self.disabled_actions.read().await;
                        if disabled.contains(&info.name) {
                            tracing::info!(
                                "Loaded bundled action '{}' as disabled from {:?}",
                                info.name,
                                md_file
                            );
                        }
                    }

                    let review = if source == ActionSource::Custom {
                        let fingerprint = crate::security::ActionGuard::compute_bundle_hash(&path)
                            .unwrap_or_else(|_| {
                                Self::fingerprint_text(&[&workflow_content, &frontmatter])
                            });
                        match self.get_action_review(&info.name).await {
                            Some(mut stored) if stored.fingerprint == fingerprint => {
                                if Self::has_semantic_skill_review_marker(&stored) {
                                    stored.source_kind =
                                        Self::action_source_label(&info.source).to_string();
                                    stored
                                } else {
                                    Self::build_blocked_review(
                                        &info.name,
                                        Self::action_source_label(&info.source),
                                        fingerprint,
                                        "Custom skill review predates the semantic security layer. Re-import or update the skill before it can run.",
                                    )
                                }
                            }
                            Some(_) => Self::build_blocked_review(
                                &info.name,
                                Self::action_source_label(&info.source),
                                fingerprint,
                                "Skill files changed on disk outside the reviewed API path; re-import or update the skill to run semantic review again.",
                            ),
                            None => Self::build_blocked_review(
                                &info.name,
                                Self::action_source_label(&info.source),
                                fingerprint,
                                "Custom skill has no semantic security review. Re-import or update the skill before it can run.",
                            ),
                        }
                    } else {
                        self.review_markdown_action(&path, &info, &workflow_content, &frontmatter)
                            .await?
                    };
                    for warning in &review.warnings {
                        tracing::warn!("Action '{}': {}", info.name, warning);
                    }
                    if !review.allow_execute {
                        tracing::warn!(
                            "Loaded action '{}' in blocked/unready state: {:?}",
                            info.name,
                            review.blocked_reason
                        );
                    }

                    tracing::info!("Loaded workflow action '{}' from {:?}", info.name, md_file);
                    self.register_workflow_action(info.clone(), workflow_content.clone())
                        .await;
                    self.upsert_action_review(review).await?;
                    continue;

                    /* Legacy duplicate security-evaluation path removed.
                    if let Some(ref guard) = self.action_guard {
                        match guard
                            .evaluate_action(&path, &info.name, &workflow_content, &frontmatter)
                            .await
                        {
                            Ok(verdict) => {
                                if !verdict.allow_load {
                                    tracing::warn!(
                                        "Action '{}' BLOCKED by security guard: {:?}",
                                        info.name,
                                        verdict.warnings
                                    );
                                    continue; // skip registration
                                }
                                for w in &verdict.warnings {
                                    tracing::warn!("Action '{}': {}", info.name, w);
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Security check failed for '{}': {} - loading anyway",
                                    info.name,
                                    e
                                );
                            }
                        }
                    }

                    tracing::info!("Loaded workflow action '{}' from {:?}", info.name, md_file);
                    self.register_workflow_action(info, workflow_content).await;
                    */
                }
                Err(e) => {
                    tracing::warn!("Failed to load action from {:?}: {}", md_file, e);
                }
            }
        }

        Ok(())
    }

    /// Parse a SKILL.md file to extract action information and full content.
    /// Returns (ActionDef, full_workflow_content, frontmatter_text)
    pub(in crate::runtime) async fn parse_action_md(
        &self,
        path: &Path,
        source: ActionSource,
    ) -> Result<(ActionDef, String, String)> {
        let content = tokio::fs::read_to_string(path).await?;

        // Parse YAML frontmatter (between --- markers)
        let mut name = String::new();
        let mut description = String::new();
        let mut version = "1.0.0".to_string();
        let mut frontmatter_text = String::new();

        if let Some((frontmatter, _rest)) = Self::split_frontmatter_block(&content) {
            frontmatter_text = frontmatter.to_string();
            if let Some(root) = Self::parse_frontmatter_yaml(frontmatter)
                .and_then(|value| value.as_mapping().cloned())
            {
                if let Some(value) = root
                    .get(serde_yaml::Value::String("name".to_string()))
                    .and_then(|value| value.as_str())
                {
                    name = value.trim().to_string();
                }
                if let Some(value) = root
                    .get(serde_yaml::Value::String("description".to_string()))
                    .and_then(|value| value.as_str())
                {
                    description = value.trim().to_string();
                }
                if let Some(value) = root
                    .get(serde_yaml::Value::String("version".to_string()))
                    .and_then(|value| value.as_str())
                {
                    version = value.trim().to_string();
                }
            }
        }

        // Fallback: use directory name as action name
        if name.is_empty() {
            name = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
        }

        // Extract first heading as description if not in frontmatter
        if description.is_empty() {
            for line in content.lines() {
                if let Some(stripped) = line.strip_prefix("# ") {
                    description = stripped.trim().to_string();
                    break;
                }
            }
        }
        if description.is_empty() {
            description = format!("Custom skill '{}'", name);
        }

        // Parse permissions from frontmatter
        let permissions = crate::security::ActionGuard::parse_permissions(&frontmatter_text);
        let mut capabilities: Vec<String> = permissions.iter().map(|p| p.to_string()).collect();
        if capabilities.is_empty() {
            capabilities.push("research".to_string());
        }

        let info = ActionDef {
            name,
            description,
            version,
            input_schema: Self::build_workflow_input_schema(&frontmatter_text, &content),
            capabilities,
            sandbox_mode: Some(SandboxMode::Native),
            source,
            file_path: Some(path.to_string_lossy().to_string()),
            authorization: Default::default(),
        };

        // Return the info, full content, and frontmatter for security evaluation
        Ok((info, content, frontmatter_text))
    }
}
