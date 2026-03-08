use super::*;

fn action_source_label(source: &crate::actions::ActionSource) -> &'static str {
    match source {
        crate::actions::ActionSource::System => "system",
        crate::actions::ActionSource::Bundled => "bundled",
        crate::actions::ActionSource::Custom => "custom",
    }
}

fn summarize_schema_fields(schema: &serde_json::Value, key: &str, limit: usize) -> Vec<String> {
    let mut fields: Vec<String> = match schema.get(key).and_then(|v| v.as_object()) {
        Some(map) => map.keys().take(limit).cloned().collect(),
        None => Vec::new(),
    };
    fields.sort();
    fields
}

fn summarize_required_fields(schema: &serde_json::Value, limit: usize) -> Vec<String> {
    schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .take(limit)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

impl Agent {
    /// Build the generic system prompt. Request-specific action metadata is appended later.
    pub(crate) async fn build_system_prompt(
        &self,
        memories: &[crate::memory::MemoryEntry],
    ) -> Result<String> {
        let bot_name = &self.config.name;
        let personality = &self.config.personality;

        let style_desc = match personality.as_str() {
            "professional" => {
                "Communicate precisely and respectfully. Structure matters. Sound like a strong senior colleague."
            }
            "casual" => {
                "Keep the tone natural and direct. Stay helpful without sounding scripted."
            }
            "technical" => {
                "Be rigorous and concrete. Explain technical tradeoffs clearly and avoid hand-waving."
            }
            "creative" => {
                "Be expressive when it helps, but still grounded in the task and the evidence."
            }
            "concise" => "Be terse by default. Expand only when the task actually needs it.",
            _ => "Be clear, useful, and human. No filler and no performative friendliness.",
        };

        let mut prompt = format!(
            r#"You are {bot_name}.

## Identity
- Act like a pragmatic operator, not a generic chatbot.
- {style_desc}

## Core Operating Rules
- Understand the user's goal from natural language and choose the best matching action from the request-specific action catalog.
- Execute immediately when the request is actionable and required inputs are already available.
- Ask for clarification only when a required input is missing or the action would be destructive under unresolved ambiguity.
- Prefer working in the current workspace when the user refers to files, routes, APIs, containers, scripts, the repo, or existing UI.
- Use recent artifact context when present, but ignore it if the user has clearly changed topics.
- Never ask the user to provide raw JSON payloads. Map natural language to tool arguments yourself.
- Never hardcode secrets into generated code or tool arguments. Use secret storage or sensitive runtime inputs.
- Keep retries bounded. State or enforce a maximum attempt count and stop at the cap.
- Be honest about uncertainty. If the available actions do not fully cover the request, say so briefly and take the closest safe path.

## Action Selection
- The action catalog appears later in this prompt and is the source of truth for available capabilities.
- Use action names, descriptions, capabilities, and input schemas semantically. Do not rely on brittle keyword matching.
- If multiple actions look plausible, prefer the clearest semantic match with the smallest missing-input surface.
- For workspace/self-modification requests, prefer local code, file, and shell actions over spinning up new external artifacts unless the user clearly asked for a separate app/service.
- When you emit a tool call, provide complete arguments that satisfy the action schema as far as the user request allows.

## Response Behavior
- Stay concise by default.
- Do not expose internal routing, scoring, or policy mechanics unless the user asks.
- Ground claims in the provided context, memories, artifacts, and tool outputs.
"#,
            bot_name = bot_name,
            style_desc = style_desc,
        );

        prompt.push('\n');
        prompt.push_str(crate::core::prompt_policy::global_policy_v2_block());

        if !memories.is_empty() {
            prompt.push_str("\n## Relevant Memories\n");
            for mem in memories {
                prompt.push_str(&format!("- {}\n", safe_truncate(&mem.content, 200)));
            }
        }

        {
            let tasks = self.tasks.read().await;
            let now = chrono::Utc::now();
            let goals: Vec<_> = tasks
                .all()
                .iter()
                .filter(|t| {
                    t.action == "goal"
                        && matches!(
                            t.status,
                            crate::core::TaskStatus::Pending | crate::core::TaskStatus::InProgress
                        )
                })
                .collect();

            if !goals.is_empty() {
                prompt.push_str("\n## Active Goals\n");
                for g in &goals {
                    let deadline_note = if let Some(due) = g.scheduled_for {
                        let days_left = (due - now).num_days();
                        if days_left < 0 {
                            format!(" - overdue by {} day(s)", days_left.abs())
                        } else if days_left == 0 {
                            " - due today".to_string()
                        } else if days_left <= 3 {
                            format!(" - due in {} day(s)", days_left)
                        } else if days_left <= 7 {
                            format!(" - due in {} days", days_left)
                        } else {
                            format!(" - due {}", due.format("%b %d"))
                        }
                    } else {
                        String::new()
                    };
                    prompt.push_str(&format!(
                        "- {}{}\n",
                        safe_truncate(&g.description, 150),
                        deadline_note
                    ));
                }
                prompt.push_str(
                    "Use goals naturally when they help prioritize or remind the user about near-term deadlines.\n",
                );
            }
        }

        Ok(crate::security::SecurityGuard::protect_system_prompt(
            &prompt,
        ))
    }

    pub(crate) fn build_action_catalog_prompt(actions: &[crate::actions::ActionDef]) -> String {
        if actions.is_empty() {
            return "## Available Actions\nNo actions are currently available.\n".to_string();
        }

        let mut prompt = String::from(
            "## Available Actions\nUse only these actions when emitting tool calls. This catalog is scoped to the current request.\n",
        );

        for action in actions.iter().take(12) {
            let required = summarize_required_fields(&action.input_schema, 6);
            let optional = summarize_schema_fields(&action.input_schema, "properties", 8)
                .into_iter()
                .filter(|field| !required.iter().any(|req| req == field))
                .take(6)
                .collect::<Vec<_>>();

            prompt.push_str(&format!(
                "- `{}` [{}]: {}\n",
                action.name,
                action_source_label(&action.source),
                safe_truncate(action.description.trim(), 180)
            ));

            if !action.capabilities.is_empty() {
                prompt.push_str(&format!(
                    "  Capabilities: {}.\n",
                    safe_truncate(&action.capabilities.join(", "), 160)
                ));
            }
            if !required.is_empty() {
                prompt.push_str(&format!("  Required inputs: {}.\n", required.join(", ")));
            }
            if !optional.is_empty() {
                prompt.push_str(&format!("  Common inputs: {}.\n", optional.join(", ")));
            }
        }

        prompt
    }

    pub(crate) async fn persist_app_preview_screenshot(
        &self,
        app_id: &str,
        screenshot: &[u8],
    ) -> Result<String> {
        let exec_id = uuid::Uuid::new_v4().to_string();
        let safe_app_id: String = app_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        let file_name = format!(
            "app_preview_{}.png",
            if safe_app_id.is_empty() {
                "app"
            } else {
                &safe_app_id
            }
        );
        let out_dir = self.data_dir.join("outputs").join(&exec_id);
        tokio::fs::create_dir_all(&out_dir).await?;
        tokio::fs::write(out_dir.join(&file_name), screenshot).await?;
        Ok(format!("/api/outputs/{}/{}", exec_id, file_name))
    }

    pub(crate) async fn persist_output_binary(
        &self,
        prefix: &str,
        extension: &str,
        bytes: &[u8],
    ) -> Result<String> {
        let exec_id = uuid::Uuid::new_v4().to_string();
        let safe_prefix: String = prefix
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        let safe_ext: String = extension
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        let name = if safe_prefix.is_empty() {
            "asset"
        } else {
            &safe_prefix
        };
        let ext = if safe_ext.is_empty() {
            "bin"
        } else {
            &safe_ext
        };
        let file_name = format!("{}.{}", name, ext);
        let out_dir = self.data_dir.join("outputs").join(&exec_id);
        tokio::fs::create_dir_all(&out_dir).await?;
        tokio::fs::write(out_dir.join(&file_name), bytes).await?;
        Ok(format!(
            "/api/outputs/{}/{}",
            exec_id,
            urlencoding::encode(&file_name)
        ))
    }
}
