use super::super::*;

impl ActionRuntime {
    pub(in crate::runtime) fn resolve_optional_cli_cwd(
        &self,
        raw: Option<&str>,
    ) -> Result<Option<PathBuf>> {
        let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        let candidate = self.absolutize_tool_path(raw)?;
        let resolved = candidate.canonicalize().with_context(|| {
            format!(
                "CLI working directory '{}' does not exist",
                candidate.display()
            )
        })?;
        self.ensure_tool_path_allowed(&resolved)?;
        Ok(Some(resolved))
    }

    pub(in crate::runtime) async fn execute_cli_action(
        &self,
        action_name: &str,
        binding: CliToolBinding,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let executable = binding.executable_path.trim();
        if executable.is_empty() {
            anyhow::bail!("CLI executable path is empty");
        }

        let args = arguments
            .get("args")
            .and_then(|value| value.as_array())
            .ok_or_else(|| anyhow::anyhow!("Missing 'args' array"))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| anyhow::anyhow!("CLI args must be strings"))
            })
            .collect::<Result<Vec<_>>>()?;
        let stdin_text = arguments
            .get("stdin")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let timeout_secs = arguments
            .get("timeout_secs")
            .and_then(|value| value.as_u64())
            .unwrap_or(60)
            .clamp(1, 300);
        let cwd =
            self.resolve_optional_cli_cwd(arguments.get("cwd").and_then(|value| value.as_str()))?;
        let review = self
            .get_action_review(action_name)
            .await
            .unwrap_or_default();
        let mut injected_env = BTreeMap::new();
        if !review.required_env.is_empty() {
            let required_secret_env = review
                .required_env
                .iter()
                .filter(|env| !binding.auth_env_exports.contains_key(*env))
                .cloned()
                .collect::<Vec<_>>();
            let placeholder_map = required_secret_env
                .iter()
                .map(|env| {
                    (
                        env.clone(),
                        serde_json::Value::String(format!("{{{{env:{}}}}}", env)),
                    )
                })
                .collect::<serde_json::Map<String, serde_json::Value>>();
            if !placeholder_map.is_empty() {
                let resolved = self.resolve_secret_placeholders(
                    action_name,
                    &serde_json::Value::Object(placeholder_map),
                )?;
                if let Some(obj) = resolved.as_object() {
                    for env in &required_secret_env {
                        if let Some(value) = obj.get(env).and_then(|value| value.as_str()) {
                            injected_env.insert(env.clone(), value.to_string());
                        }
                    }
                }
            }
        }
        if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
            if binding.auth_env_exports.is_empty() {
                anyhow::bail!(
                    "CLI auth profile '{}' is bound but no auth.env_exports mapping is declared.",
                    auth_profile_id
                );
            }
            let storage = self.storage().ok_or_else(|| {
                anyhow::anyhow!("Storage is unavailable for auth profile lookups")
            })?;
            let auth_exports =
            crate::core::connectivity::auth_profiles::AuthProfileControlPlane::resolve_env_exports(
                &storage,
                auth_profile_id,
                &binding.auth_env_exports,
            )
            .await?;
            for (key, value) in auth_exports {
                injected_env.insert(key, value);
            }
        }

        let mut command = tokio::process::Command::new(executable);
        command.args(&args);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        for (key, value) in injected_env {
            command.env(key, value);
        }
        if stdin_text.is_some() {
            command.stdin(std::process::Stdio::piped());
        }
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());

        let mut child = command.spawn().with_context(|| {
            format!(
                "Failed to launch CLI executable '{}'",
                binding.executable_path
            )
        })?;

        if let Some(stdin_text) = stdin_text {
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                stdin.write_all(stdin_text.as_bytes()).await?;
            }
        }

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            child.wait_with_output(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("CLI command timed out after {}s", timeout_secs))??;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let mut combined = String::new();
        if !stdout.is_empty() {
            combined.push_str(&stdout);
        }
        if !stderr.is_empty() {
            if !combined.is_empty() {
                combined.push_str("\n\nstderr:\n");
            } else {
                combined.push_str("stderr:\n");
            }
            combined.push_str(&stderr);
        }
        if combined.is_empty() {
            combined = "(no output)".to_string();
        }

        if output.status.success() {
            if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
                if let Some(storage) = self.storage() {
                    let _ =
                    crate::core::connectivity::auth_profiles::AuthProfileControlPlane::mark_used(
                        &storage,
                        auth_profile_id,
                    )
                    .await;
                }
            }
            Ok(combined)
        } else {
            Err(anyhow::anyhow!(
                "CLI command exited with status {}. {}",
                output
                    .status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
                combined
            ))
        }
    }
}
