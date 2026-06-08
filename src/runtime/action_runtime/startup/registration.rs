use super::super::*;

impl ActionRuntime {
    pub(in crate::runtime) async fn register_builtin_action(&self, info: ActionDef) {
        let info = Self::normalize_action_definition(info);
        let builtin_handler =
            BuiltinActionHandler::for_action(&info, self.config.default_sandbox.clone());
        let supports_background = Self::action_schema_supports_background(&info);
        self.actions.write().await.insert(
            info.name.clone(),
            LoadedAction {
                info,
                builtin_handler,
                supports_background,
                wasm_module: None,
                workflow_content: None,
                cli_binding: None,
                mcp_binding: None,
                plugin_binding: None,
                custom_api_binding: None,
                extension_pack_binding: None,
            },
        );
    }

    pub(in crate::runtime) fn action_schema_supports_background(info: &ActionDef) -> bool {
        info.input_schema
            .pointer("/properties/background")
            .is_some()
    }

    /// Register an action with workflow content from SKILL.md
    pub(in crate::runtime) async fn register_workflow_action(
        &self,
        info: ActionDef,
        workflow: String,
    ) {
        let info = Self::normalize_action_definition(info);
        let builtin_handler =
            BuiltinActionHandler::for_action(&info, self.config.default_sandbox.clone());
        let supports_background = Self::action_schema_supports_background(&info);
        self.actions.write().await.insert(
            info.name.clone(),
            LoadedAction {
                info,
                builtin_handler,
                supports_background,
                wasm_module: None,
                workflow_content: Some(workflow),
                cli_binding: None,
                mcp_binding: None,
                plugin_binding: None,
                custom_api_binding: None,
                extension_pack_binding: None,
            },
        );
    }

    pub(in crate::runtime) async fn register_cli_action(
        &self,
        info: ActionDef,
        binding: CliToolBinding,
    ) {
        let info = Self::normalize_action_definition(info);
        self.actions.write().await.insert(
            info.name.clone(),
            LoadedAction {
                info,
                builtin_handler: None,
                supports_background: false,
                wasm_module: None,
                workflow_content: None,
                cli_binding: Some(binding),
                mcp_binding: None,
                plugin_binding: None,
                custom_api_binding: None,
                extension_pack_binding: None,
            },
        );
    }

    /// Register an MCP-backed action (external tool/resource)
    pub async fn register_mcp_action(&self, info: ActionDef, binding: McpBinding) {
        let info = Self::normalize_action_definition(info);
        let review = self.review_mcp_action(&info, &binding).await;
        self.actions.write().await.insert(
            info.name.clone(),
            LoadedAction {
                info,
                builtin_handler: None,
                supports_background: false,
                wasm_module: None,
                workflow_content: None,
                cli_binding: None,
                mcp_binding: Some(binding),
                plugin_binding: None,
                custom_api_binding: None,
                extension_pack_binding: None,
            },
        );
        match review {
            Ok(review) => {
                if let Err(error) = self.upsert_action_review(review).await {
                    tracing::warn!("Failed to persist MCP action review state: {}", error);
                }
            }
            Err(error) => {
                tracing::warn!("Failed to review MCP action during registration: {}", error);
            }
        }
    }

    /// Register a plugin-backed action
    pub async fn register_plugin_action(&self, info: ActionDef, binding: PluginBinding) {
        let info = Self::normalize_action_definition(info);
        let review = self.review_plugin_action(&info, &binding).await;
        self.actions.write().await.insert(
            info.name.clone(),
            LoadedAction {
                info,
                builtin_handler: None,
                supports_background: false,
                wasm_module: None,
                workflow_content: None,
                cli_binding: None,
                mcp_binding: None,
                plugin_binding: Some(binding),
                custom_api_binding: None,
                extension_pack_binding: None,
            },
        );
        match review {
            Ok(review) => {
                if let Err(error) = self.upsert_action_review(review).await {
                    tracing::warn!("Failed to persist plugin action review state: {}", error);
                }
            }
            Err(error) => {
                tracing::warn!(
                    "Failed to review plugin action during registration: {}",
                    error
                );
            }
        }
    }

    /// Register an imported custom API action.
    pub async fn register_custom_api_action(&self, info: ActionDef, binding: CustomApiBinding) {
        let info = Self::normalize_action_definition(info);
        let review = self.review_custom_api_action(&info, &binding).await;
        self.actions.write().await.insert(
            info.name.clone(),
            LoadedAction {
                info,
                builtin_handler: None,
                supports_background: false,
                wasm_module: None,
                workflow_content: None,
                cli_binding: None,
                mcp_binding: None,
                plugin_binding: None,
                custom_api_binding: Some(binding),
                extension_pack_binding: None,
            },
        );
        match review {
            Ok(review) => {
                if let Err(error) = self.upsert_action_review(review).await {
                    tracing::warn!(
                        "Failed to persist custom API action review state: {}",
                        error
                    );
                }
            }
            Err(error) => {
                tracing::warn!(
                    "Failed to review custom API action during registration: {}",
                    error
                );
            }
        }
    }

    /// Register an installed extension-pack feature as a real runtime action.
    pub async fn register_extension_pack_action(
        &self,
        info: ActionDef,
        binding: ExtensionPackActionBinding,
    ) {
        let info = Self::normalize_action_definition(info);
        let review = self.review_extension_pack_action(&info, &binding).await;
        self.actions.write().await.insert(
            info.name.clone(),
            LoadedAction {
                info,
                builtin_handler: None,
                supports_background: false,
                wasm_module: None,
                workflow_content: None,
                cli_binding: None,
                mcp_binding: None,
                plugin_binding: None,
                custom_api_binding: None,
                extension_pack_binding: Some(binding),
            },
        );
        match review {
            Ok(review) => {
                if let Err(error) = self.upsert_action_review(review).await {
                    tracing::warn!(
                        "Failed to persist extension-pack action review state: {}",
                        error
                    );
                }
            }
            Err(error) => {
                tracing::warn!(
                    "Failed to review extension-pack action during registration: {}",
                    error
                );
            }
        }
    }

    pub(in crate::runtime) fn build_cli_action_input_schema(
        action_name: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "args": {
                    "type": "array",
                    "description": format!("Argument list to pass to {}. Do not include the executable name itself.", action_name),
                    "items": { "type": "string" }
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional working directory. Must stay within allowed workspace/data roots."
                },
                "stdin": {
                    "type": "string",
                    "description": "Optional text to pipe to stdin."
                },
                "timeout_secs": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 300,
                    "description": "Optional timeout in seconds. Default 60."
                }
            },
            "required": ["args"]
        })
    }

    pub(in crate::runtime) fn build_cli_action_def(
        manifest: &InstalledCliSkillManifest,
        skill_path: &Path,
    ) -> ActionDef {
        ActionDef {
            name: manifest.name.clone(),
            description: format!(
                "{} Use this action to call the verified local CLI directly. Pass exact argv items in `args`, and use `--help` whenever syntax is unclear.",
                manifest.description.trim()
            ),
            version: manifest.version.clone(),
            input_schema: Self::build_cli_action_input_schema(&manifest.name),
            capabilities: vec!["local_cli".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::Custom,
            file_path: Some(skill_path.to_string_lossy().to_string()),
            authorization: Default::default(),
        }
    }

    pub async fn install_cli_skill_action(
        &self,
        manifest: InstalledCliSkillManifest,
        skill_markdown: &str,
    ) -> Result<()> {
        let skill_name = manifest.name.trim();
        if skill_name.is_empty() {
            anyhow::bail!("CLI skill name cannot be empty");
        }

        {
            let actions = self.actions.read().await;
            if let Some(existing) = actions.get(skill_name) {
                if existing.info.source == ActionSource::System {
                    anyhow::bail!(
                        "Cannot install CLI skill '{}': a built-in action with that name already exists",
                        skill_name
                    );
                }
                if existing.cli_binding.is_none() && existing.workflow_content.is_some() {
                    anyhow::bail!(
                        "Cannot install CLI skill '{}': a markdown workflow skill with that name already exists",
                        skill_name
                    );
                }
            }
        }

        let skill_dir = self.cli_skills_dir.join(skill_name);
        tokio::fs::create_dir_all(&skill_dir).await?;
        let skill_path = skill_dir.join("SKILL.md");
        let manifest_path = skill_dir.join("manifest.json");

        tokio::fs::write(&skill_path, skill_markdown).await?;
        tokio::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?).await?;

        let info = Self::build_cli_action_def(&manifest, &skill_path);
        if let Some(ref guard) = self.action_guard {
            if let Err(error) = guard.resign_action(&skill_dir, skill_name).await {
                tracing::warn!("Failed to sign CLI skill '{}': {}", skill_name, error);
            }
        }
        let (_parsed, workflow_content, frontmatter) = self
            .parse_action_md(&skill_path, ActionSource::Custom)
            .await?;
        let binding = CliToolBinding {
            executable_path: manifest.executable_path.clone(),
            verify_args: manifest.verify_args.clone(),
            auth_profile_id: Self::extract_auth_profile_id_from_frontmatter(&frontmatter),
            auth_env_exports: Self::extract_auth_env_exports_from_frontmatter(&frontmatter),
        };
        let review = self
            .review_cli_action(&skill_dir, &info, &workflow_content, &frontmatter, &binding)
            .await?;
        self.register_cli_action(info, binding).await;
        self.upsert_action_review(review.clone()).await?;
        tracing::info!(
            "Installed CLI skill '{}' backed by {}",
            skill_name,
            manifest.executable_path
        );
        if !review.allow_execute {
            tracing::warn!(
                "CLI skill '{}' installed in blocked/unready state: {:?}",
                skill_name,
                review.blocked_reason
            );
        }
        Ok(())
    }

    pub(in crate::runtime) async fn load_cli_skill_actions(&self) -> Result<()> {
        let mut entries = match tokio::fs::read_dir(&self.cli_skills_dir).await {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(
                    "Could not read CLI skills directory {:?}: {}",
                    self.cli_skills_dir,
                    e
                );
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
            let manifest_path = path.join("manifest.json");
            let skill_path = path.join("SKILL.md");
            let manifest_exists = tokio::fs::metadata(&manifest_path)
                .await
                .map(|meta| meta.is_file())
                .unwrap_or(false);
            let skill_exists = tokio::fs::metadata(&skill_path)
                .await
                .map(|meta| meta.is_file())
                .unwrap_or(false);
            if !manifest_exists || !skill_exists {
                continue;
            }

            let manifest = match tokio::fs::read_to_string(&manifest_path).await {
                Ok(raw) => match serde_json::from_str::<InstalledCliSkillManifest>(&raw) {
                    Ok(manifest) => manifest,
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse CLI skill manifest {:?}: {}",
                            manifest_path,
                            e
                        );
                        continue;
                    }
                },
                Err(e) => {
                    tracing::warn!(
                        "Failed to read CLI skill manifest {:?}: {}",
                        manifest_path,
                        e
                    );
                    continue;
                }
            };

            let info = Self::build_cli_action_def(&manifest, &skill_path);
            let parsed = match self
                .parse_action_md(&skill_path, ActionSource::Custom)
                .await
            {
                Ok(parsed) => parsed,
                Err(error) => {
                    tracing::warn!(
                        "Failed to parse CLI skill markdown {:?}: {}",
                        skill_path,
                        error
                    );
                    continue;
                }
            };
            let (_parsed_info, workflow_content, frontmatter) = parsed;
            let binding = CliToolBinding {
                executable_path: manifest.executable_path.clone(),
                verify_args: manifest.verify_args.clone(),
                auth_profile_id: Self::extract_auth_profile_id_from_frontmatter(&frontmatter),
                auth_env_exports: Self::extract_auth_env_exports_from_frontmatter(&frontmatter),
            };
            let review = self
                .review_cli_action_for_startup(
                    &path,
                    &info,
                    &workflow_content,
                    &frontmatter,
                    &binding,
                )
                .await?;
            self.register_cli_action(info, binding).await;
            self.upsert_action_review(review).await?;
        }

        Ok(())
    }

    /// Remove all MCP-backed actions
    pub async fn unregister_mcp_actions(&self) -> usize {
        let removed_names = {
            let mut actions = self.actions.write().await;
            let removed = actions
                .iter()
                .filter(|(_, action)| action.mcp_binding.is_some())
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            actions.retain(|_, a| a.mcp_binding.is_none());
            removed
        };
        let _ = self
            .remove_action_reviews(|name| removed_names.iter().any(|n| n == name))
            .await;
        removed_names.len()
    }

    /// Remove MCP-backed actions for a specific server
    pub async fn unregister_mcp_actions_for_server(&self, server_id: &str) -> usize {
        let removed_names = {
            let mut actions = self.actions.write().await;
            let removed = actions
                .iter()
                .filter(|(_, action)| {
                    action
                        .mcp_binding
                        .as_ref()
                        .is_some_and(|binding| binding.server_id == server_id)
                })
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            actions.retain(|_, a| {
                if let Some(binding) = &a.mcp_binding {
                    binding.server_id != server_id
                } else {
                    true
                }
            });
            removed
        };
        let _ = self
            .remove_action_reviews(|name| removed_names.iter().any(|n| n == name))
            .await;
        removed_names.len()
    }

    /// Remove all plugin-backed actions
    pub async fn unregister_plugin_actions(&self) -> usize {
        let removed_names = {
            let mut actions = self.actions.write().await;
            let removed = actions
                .iter()
                .filter(|(_, action)| action.plugin_binding.is_some())
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            actions.retain(|_, a| a.plugin_binding.is_none());
            removed
        };
        let _ = self
            .remove_action_reviews(|name| removed_names.iter().any(|n| n == name))
            .await;
        removed_names.len()
    }

    /// Remove plugin-backed actions for a specific plugin
    pub async fn unregister_plugin_actions_for_plugin(&self, plugin_id: &str) -> usize {
        let removed_names = {
            let mut actions = self.actions.write().await;
            let removed = actions
                .iter()
                .filter(|(_, action)| {
                    action
                        .plugin_binding
                        .as_ref()
                        .is_some_and(|binding| binding.plugin_id == plugin_id)
                })
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            actions.retain(|_, a| {
                if let Some(binding) = &a.plugin_binding {
                    binding.plugin_id != plugin_id
                } else {
                    true
                }
            });
            removed
        };
        let _ = self
            .remove_action_reviews(|name| removed_names.iter().any(|n| n == name))
            .await;
        removed_names.len()
    }

    /// Remove all imported custom API actions.
    pub async fn unregister_custom_api_actions(&self) -> usize {
        let removed_names = {
            let mut actions = self.actions.write().await;
            let removed = actions
                .iter()
                .filter(|(_, action)| action.custom_api_binding.is_some())
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            actions.retain(|_, a| a.custom_api_binding.is_none());
            removed
        };
        let _ = self
            .remove_action_reviews(|name| removed_names.iter().any(|n| n == name))
            .await;
        removed_names.len()
    }

    /// Remove all installed extension-pack feature actions.
    pub async fn unregister_extension_pack_actions(&self) -> usize {
        let removed_names = {
            let mut actions = self.actions.write().await;
            let removed = actions
                .iter()
                .filter(|(_, action)| action.extension_pack_binding.is_some())
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            actions.retain(|_, a| a.extension_pack_binding.is_none());
            removed
        };
        let _ = self
            .remove_action_reviews(|name| removed_names.iter().any(|n| n == name))
            .await;
        removed_names.len()
    }
}
