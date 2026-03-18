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
  - If the execution shape itself is unclear (for example chat vs app vs task vs watcher vs integration), ask one short confirmation instead of guessing.
- If the user names a concrete destination, community, page, app, account, or workspace and asks you to explore, contribute, engage, or "try something", start with the nearest safe read/inspect step and then take one concrete action if the available tools allow it. Do not bounce back with a clarification when you already have enough context to begin.
- IMPORTANT: When the user mentions the name of a deployed app listed in the runtime access summary, use `app_inspect` on that app immediately. These are YOUR deployed apps — never ask for a repo link, tech stack, or code. Inspect first, then act.
- Prefer working in the current workspace when the user refers to files, routes, APIs, containers, scripts, the repo, or existing UI.
- Use recent artifact context when present, but ignore it if the user has clearly changed topics.
- Never ask the user to provide raw JSON payloads. Map natural language to tool arguments yourself.
- Never hardcode secrets into generated code or tool arguments. Use secret storage or sensitive runtime inputs.
- Keep retries bounded. State or enforce a maximum attempt count and stop at the cap.
- Be honest about uncertainty. If the available actions do not fully cover the request, say so briefly and take the closest safe path.
- Lightweight saved user facts may already be included later in this prompt. Richer semantic memory, saved items, and durable knowledge are not prefetched; if prior context outside the visible prompt may affect the answer, use the relevant memory action from the catalog before answering.
- When the user asks what the agent has access to, what is configured, or what is available in the workspace, inspect live platform state with the relevant inventory/manage actions instead of guessing.
- Treat the system as broadly inspectable for operational state: apps, tasks, watchers, goals, traces, logs, integrations, documents, and runtime status are all fair game when the relevant actions exist. Never reveal raw keys, tokens, passwords, or secret values.
- For community/social posting actions, write original agent-authored content based on the current situation and your own grounded reasoning. Do not simply restate the user's instruction as the post or comment, and never include user data, PII, conversation text, or secrets.
- For ongoing or indefinite monitoring ("every minute", "every hour", "every day", "keep watching"), create a scheduled task/routine. Use a watcher only for bounded poll-until-condition workflows with a clear timeout.
- For persistent resources such as apps, tasks, watchers, and reusable capabilities, default to updating/reusing an existing matching item instead of creating a duplicate. Create a second one only when the user explicitly asks for another separate copy.
- If the request needs a capability that does not already exist, first inspect existing integrations/actions. If the capability is still missing and the catalog exposes capability acquisition/scaffolding, use it to generate a reusable connector-backed action instead of failing immediately.

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
                        && !matches!(
                            t.status,
                            crate::core::TaskStatus::Failed { .. }
                                | crate::core::TaskStatus::Cancelled
                        )
                })
                .collect();

            if !goals.is_empty() {
                prompt.push_str("\n## Saved Goals\n");
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

    pub(crate) fn build_runtime_access_summary(actions: &[crate::actions::ActionDef]) -> String {
        if actions.is_empty() {
            return String::new();
        }

        let system_count = actions
            .iter()
            .filter(|action| matches!(action.source, crate::actions::ActionSource::System))
            .count();
        let mut bundled = actions
            .iter()
            .filter(|action| matches!(action.source, crate::actions::ActionSource::Bundled))
            .map(|action| action.name.clone())
            .collect::<Vec<_>>();
        bundled.sort();
        let mut custom = actions
            .iter()
            .filter(|action| matches!(action.source, crate::actions::ActionSource::Custom))
            .map(|action| action.name.clone())
            .collect::<Vec<_>>();
        custom.sort();

        let mut lines = vec![format!(
            "## Runtime Access Summary\n- Scoped executable actions: {} total ({} system, {} bundled, {} user-added).",
            actions.len(),
            system_count,
            bundled.len(),
            custom.len()
        )];

        let mut surfaces = Vec::new();
        if actions.iter().any(|action| action.name == "list_tasks") {
            surfaces.push("tasks/routines");
        }
        if actions.iter().any(|action| action.name == "schedule_task") {
            surfaces.push("scheduling");
        }
        if actions
            .iter()
            .any(|action| action.name == "watch" || action.name == "list_watchers")
        {
            surfaces.push("watchers");
        }
        if actions.iter().any(|action| action.name == "goal_manage") {
            surfaces.push("goals");
        }
        if actions.iter().any(|action| action.name == "manage_actions") {
            surfaces.push("action library and user-added skills");
        }
        if actions
            .iter()
            .any(|action| action.name == "list_integrations")
        {
            surfaces.push("integration inventory");
        }
        if actions.iter().any(|action| action.name == "app_inspect") {
            surfaces.push("deployed apps");
        }
        if actions.iter().any(|action| action.name == "security_logs") {
            surfaces.push("security logs");
        }
        if actions.iter().any(|action| action.name == "memory_lookup") {
            surfaces.push("durable memory");
        }
        if !surfaces.is_empty() {
            lines.push(format!(
                "- Platform surfaces reachable in this request: {}.",
                surfaces.join(", ")
            ));
        }

        let app_tools = ["app_inspect", "app_restart", "app_deploy"]
            .into_iter()
            .filter(|name| actions.iter().any(|action| action.name == *name))
            .collect::<Vec<_>>();
        if !app_tools.is_empty() {
            lines.push(format!(
                "- App/deployment tools available now: {}.",
                app_tools
                    .iter()
                    .map(|name| format!("`{}`", name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        let dynamic_integration_tool_count = actions
            .iter()
            .filter(|action| action.description.starts_with("Integration tool '"))
            .count();
        if dynamic_integration_tool_count > 0 {
            lines.push(format!(
                "- Integration-backed tools already present in this scoped catalog: {}.",
                dynamic_integration_tool_count
            ));
        }

        if actions
            .iter()
            .any(|action| action.name == "capability_acquire")
        {
            lines.push(
                "- Missing capabilities can be scaffolded into reusable user-added actions when needed."
                    .to_string(),
            );
        }

        if !custom.is_empty() {
            let preview = custom.iter().take(6).cloned().collect::<Vec<_>>();
            let more = custom.len().saturating_sub(preview.len());
            lines.push(format!(
                "- User-added skills/actions loaded: {}{}.",
                preview
                    .iter()
                    .map(|name| format!("`{}`", name))
                    .collect::<Vec<_>>()
                    .join(", "),
                if more > 0 {
                    format!(" (+{} more)", more)
                } else {
                    String::new()
                }
            ));
        }

        if !bundled.is_empty() {
            let preview = bundled.iter().take(6).cloned().collect::<Vec<_>>();
            let more = bundled.len().saturating_sub(preview.len());
            lines.push(format!(
                "- Bundled skills/actions loaded: {}{}.",
                preview
                    .iter()
                    .map(|name| format!("`{}`", name))
                    .collect::<Vec<_>>()
                    .join(", "),
                if more > 0 {
                    format!(" (+{} more)", more)
                } else {
                    String::new()
                }
            ));
        }

        format!("{}\n", lines.join("\n"))
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
