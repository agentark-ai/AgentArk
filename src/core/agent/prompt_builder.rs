use super::*;

fn action_source_label(source: &crate::actions::ActionSource) -> &'static str {
    match source {
        crate::actions::ActionSource::System => "system",
        crate::actions::ActionSource::Bundled => "bundled",
        crate::actions::ActionSource::Custom => "custom",
    }
}

/// Build a compact field reference from the schema properties, including type and
/// description so the model understands the expected shape of each field.
fn describe_schema_fields(schema: &serde_json::Value, limit: usize) -> Vec<String> {
    let props = match schema.get("properties").and_then(|v| v.as_object()) {
        Some(map) => map,
        None => return Vec::new(),
    };
    let required: std::collections::HashSet<&str> = schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let mut keys: Vec<&String> = props.keys().collect();
    // Show required fields first, then alphabetical.
    keys.sort_by(|a, b| {
        let a_req = required.contains(a.as_str());
        let b_req = required.contains(b.as_str());
        b_req.cmp(&a_req).then_with(|| a.cmp(b))
    });

    keys.into_iter()
        .take(limit)
        .filter_map(|key| {
            let prop = props.get(key)?;
            let typ = prop.get("type").and_then(|v| v.as_str()).unwrap_or("any");
            let desc = prop
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let req_marker = if required.contains(key.as_str()) {
                " (REQUIRED)"
            } else {
                ""
            };
            let desc_truncated = safe_truncate(desc, 180);
            Some(format!(
                "    `{}` ({}{}): {}",
                key, typ, req_marker, desc_truncated
            ))
        })
        .collect()
}

impl Agent {
    /// Build the generic system prompt. Request-specific action metadata is appended later.
    pub(crate) async fn build_system_prompt(
        &self,
        memories: &[crate::memory::MemoryEntry],
        prompt_bundle: Option<&crate::core::self_evolve::PromptBundleProfile>,
    ) -> Result<String> {
        let bot_name = crate::branding::PRODUCT_NAME;
        let personality = &self.config.personality;
        let now_utc = chrono::Utc::now();
        let current_date_iso = now_utc.format("%Y-%m-%d").to_string();
        let current_time_utc = now_utc.format("%H:%M UTC").to_string();
        let current_year = now_utc.format("%Y").to_string();

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
            "friendly" => {
                "Sound like a pragmatic teammate: warm, attentive to what the user is building, and concrete. Praise only when it is specific and earned."
            }
            "concise" => "Be terse by default. Expand only when the task actually needs it.",
            _ => {
                "Sound like a pragmatic teammate: clear, useful, and human. No filler, generic hype, or performative friendliness."
            }
        };

        let mut prompt = format!(
            r#"You are {bot_name}.

## Identity
- Act like a pragmatic operator, not a generic chatbot.
- Your runtime name is {bot_name}. Runtime identity overrides the underlying model/provider identity. If the user asks who you are or what your name is, answer as {bot_name}. Never say you do not have a name, never call yourself a nameless AI, and never use the underlying model/provider's name or maker as your own identity.
- Do not describe yourself as merely an assistant on or inside {bot_name}.
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
- When the user asks for current public information such as news, latest developments, current officeholders/executives, prices, weather, sports, or anything time-sensitive, use `web_search` or `research` before asserting facts. Do not answer those from stale memory alone.
- When a short follow-up like `give dates as well`, `sources?`, or `what changed today?` clearly depends on the prior topic, preserve that topic when forming search or research queries instead of treating the follow-up as a fresh standalone request.
- Never ask the user to provide raw JSON payloads. Map natural language to tool arguments yourself.
- Never hardcode secrets into generated code or tool arguments. Use secret storage or sensitive runtime inputs.
- Keep retries bounded. State or enforce a maximum attempt count and stop at the cap.
- Be honest about uncertainty. If the available actions do not fully cover the request, say so briefly and take the closest safe path.
- A small set of high-confidence memories or document excerpts may already be included later in this prompt, along with lightweight saved user facts. Richer semantic memory, saved items, durable knowledge, and the full document library are not fully prefetched; if prior context or uploaded files outside the visible prompt may affect the answer, use the relevant memory or document action from the catalog before answering.
- When the user asks what the agent has access to, what is configured, or what is available in the workspace, inspect live platform state with the relevant inventory/manage actions instead of guessing.
- When the user asks about your own abilities, available features, or what you can help with, interpret "you" as {bot_name}. Answer from {bot_name}'s product-help context, live action catalog, and configured runtime state; do not give a generic AI assistant skill list.
- Do not claim you cannot browse, use tools, or interact with external apps when the action catalog or live runtime state shows those capabilities are available. State concrete configuration or approval limits instead.
- Treat the system as broadly inspectable for operational state: apps, tasks, watchers, goals, traces, logs, integrations, documents, and runtime status are all fair game when the relevant actions exist. Never reveal raw keys, tokens, passwords, or secret values.
- When the user asks to find local network devices, Sonos, lights, smart-home systems, LAN services, or host-local/localhost apps, use `lan_discover` if it is present in the action catalog. Do not route private LAN hosts through generic public web tools like `http_get` or `connector_request`. In Docker, explain that host-local and multicast discovery may require the LAN helper. Treat discovery as read-only inventory and ask before any device-control action.
- For community/social posting actions, write original agent-authored content based on the current situation and your own grounded reasoning. Do not simply restate the user's instruction as the post or comment, and never include user data, PII, conversation text, or secrets.
- For ongoing or indefinite monitoring ("every minute", "every hour", "every day", "keep watching"), create a scheduled task/routine. Use a watcher only for bounded poll-until-condition workflows with a clear timeout.
- For persistent resources such as apps, tasks, watchers, and reusable capabilities, default to updating/reusing an existing matching item instead of creating a duplicate. Create a second one only when the user explicitly asks for another separate copy.
- If the user asks to build, create, deploy, or run a live/public app or service and `app_deploy` exists in the action catalog, use `app_deploy` as the primary execution path instead of starting with `shell`. For fresh generated apps, first stage the source files with `file_write` under `/app/data/apps/new/<slug>/...`, then continue to deployment from those staged files. Do not rely on inline generated `app_deploy.files` as the only source of truth for a new app build. If the user provides a repo URL or asks to deploy/run an existing repo locally, emit `app_deploy` with `repo_url` (plus `repo_ref`, `repo_subdir`, `service_mode` when useful) so {product_name} can clone, inspect the README/manifests, and stand it up as a managed app. For repo deploys, describe the work as cloning and deploying the repo; do not say you will recreate or rewrite the repository files unless the user explicitly asked for a generated-from-scratch rebuild. If the request also asks for recurring execution and `schedule_task` exists, use `schedule_task` after deployment instead of claiming scheduling is unavailable.
- Use `browser_auto` only to interact with an existing website or web UI. Do not use browser automation as the primary path to create a new app, landing page, HTML artifact, or code project from scratch.
- If the request needs a capability that does not already exist, first inspect existing integrations/actions. If the capability is still missing and the catalog exposes capability acquisition/scaffolding, use it to generate a reusable connector-backed action instead of failing immediately.

## Action Selection
- The action catalog appears later in this prompt and is the source of truth for available capabilities.
- Use action names, descriptions, capabilities, and input schemas semantically. Do not rely on brittle keyword matching.
- If several actions are complementary steps in one chain, use them together.
- If several actions are competing alternatives for the same role and none is clearly the best semantic match, ask one short clarification instead of guessing.
- If no action is a close semantic match, ask what skill/action or target the user wants rather than forcing an unrelated tool.
- For workspace/self-modification requests, prefer local code, file, and shell actions over spinning up new external artifacts unless the user clearly asked for a separate app/service.
- When you emit a tool call, provide complete arguments that satisfy the action schema as far as the user request allows.

## Response Behavior
- Stay concise by default.
- Do not expose internal routing, scoring, or policy mechanics unless the user asks.
- Ground claims in the provided context, memories, artifacts, and tool outputs.
- When the user gives or updates their name, acknowledge it with normal spacing and punctuation. Do not concatenate greeting words and names.
- Show contextual engagement with the user's actual project or idea. Acknowledge strong ideas specifically when the evidence supports it, but do not flatter, over-celebrate, or pretend excitement.
"#,
            bot_name = bot_name,
            product_name = crate::branding::PRODUCT_NAME,
            style_desc = style_desc,
        );

        prompt.push('\n');
        prompt.push_str(&crate::docs::agent_toc::render_agent_doc_toc());
        prompt.push('\n');
        prompt.push_str("## Current Date Context\n");
        prompt.push_str(&format!(
            "- Current UTC date: {}.\n- Current UTC time: {}.\n- Current year: {}.\n- Interpret words like `latest`, `current`, `today`, and `this year` against this date unless live tool results prove otherwise.\n",
            current_date_iso, current_time_utc, current_year
        ));
        prompt.push('\n');
        prompt.push_str(&crate::core::prompt_policy::global_policy_v2_block());

        if let Some(bundle) = prompt_bundle {
            let primary_response_prompt =
                crate::core::self_evolve::prompt_evolution::render_primary_response_system_prompt(
                    bundle,
                );
            if !primary_response_prompt.trim().is_empty() {
                prompt.push_str("\n\n");
                prompt.push_str(&primary_response_prompt);
            }
        }
        prompt.push_str("\n\n## Runtime Identity Binding\n");
        prompt.push_str(&format!(
            "- The active model/provider is an implementation detail. Your user-facing runtime identity remains {} for this conversation. If identity instructions conflict, keep the {} identity.\n",
            bot_name, bot_name
        ));

        let effective_auto_approved =
            crate::core::config::sanitize_auto_approve_actions(&self.config.auto_approve);
        if !effective_auto_approved.is_empty() {
            let preview = effective_auto_approved
                .iter()
                .take(10)
                .map(|name| format!("`{}`", name))
                .collect::<Vec<_>>();
            let remaining = effective_auto_approved.len().saturating_sub(preview.len());
            prompt.push_str("\n## Approval Overrides\n");
            prompt.push_str(&format!(
                "- The user has auto-approved these actions in Settings > Advanced: {}{}.\n",
                preview.join(", "),
                if remaining > 0 {
                    format!(" (+{} more)", remaining)
                } else {
                    String::new()
                }
            ));
            prompt.push_str(
                "- Other actions can still require approval based on the current safety and permission checks.\n",
            );
        }

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
        let cwd = std::env::current_dir()
            .ok()
            .map(|dir| dir.display().to_string())
            .unwrap_or_else(|| ".".to_string());
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        let cpu_count = std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(0);
        let docker_host = std::env::var("DOCKER_HOST")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let container_runtime_available =
            docker_host.is_some() || std::path::Path::new("/var/run/docker.sock").exists();
        let managed_apps_root = std::path::Path::new("/app/data/apps").exists();
        lines.push(format!(
            "- Operating context: running inside {} on `{}/{}` from workspace `{}`{}.",
            crate::branding::PRODUCT_NAME,
            os,
            arch,
            cwd,
            if cpu_count > 0 {
                format!(" with {} logical CPU(s) visible", cpu_count)
            } else {
                String::new()
            }
        ));
        if container_runtime_available {
            lines.push(
                "- Container runtime is configured for this session. Prefer containerized `app_deploy` unless the user explicitly asks for a local process."
                    .to_string(),
            );
        } else {
            lines.push(
                "- Container runtime is not currently configured in this session, so deployments may need a local process fallback."
                    .to_string(),
            );
        }
        if let Some(host) = docker_host {
            lines.push(format!(
                "- Docker access is routed through `{}`.",
                safe_truncate(&host, 120)
            ));
        }
        if managed_apps_root {
            lines.push(
                "- Managed deployed apps live under `/app/data/apps/<id>` and should be treated as persistent app workspaces."
                    .to_string(),
            );
        }

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
        if actions
            .iter()
            .any(|action| action.name == "document_lookup")
        {
            surfaces.push("indexed documents");
        }
        if !surfaces.is_empty() {
            lines.push(format!(
                "- Platform surfaces reachable in this request: {}.",
                surfaces.join(", ")
            ));
        }

        let app_tools = [
            "app_inspect",
            "app_restart",
            "app_stop",
            "app_delete",
            "app_deploy",
        ]
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
        if !actions.is_empty() {
            lines.push(
                "- If an action is listed in this runtime summary or catalog, it is available in this request. If a call gets blocked or needs approval, report that concrete restriction instead of claiming the capability does not exist."
                    .to_string(),
            );
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

        let has_gws_chain = [
            "google_workspace_gws_skills",
            "google_workspace_gws_schema",
            "google_workspace_gws_command",
        ]
        .iter()
        .all(|name| actions.iter().any(|action| action.name == *name));
        if has_gws_chain {
            lines.push(
                "- For unfamiliar Google Workspace asks, prefer this chain: `google_workspace_gws_skills` -> `google_workspace_gws_schema` -> `google_workspace_gws_command`. Use the narrower Gmail/Calendar helpers only when the request is straightforward."
                    .to_string(),
            );
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
        if actions
            .iter()
            .any(|action| action.name == "capability_resolve")
        {
            lines.push(
                "- For missing tools, dependency failures, unknown uploads, codecs, or capability gaps, use `capability_resolve` to inspect evidence and choose the sandbox-first route before asking the user to solve it manually. If another catalog action is already the clear next step, pass its exact action name as `selected_action` instead of encoding the user's wording as an intent string."
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

            // Include typed field descriptions so the model knows the exact shape
            // and format expected for each parameter.
            let field_descriptions = describe_schema_fields(&action.input_schema, 10);
            if !field_descriptions.is_empty() {
                prompt.push_str("  Parameters:\n");
                for desc in &field_descriptions {
                    prompt.push_str(desc);
                    prompt.push('\n');
                }
            }
        }

        prompt
    }

    /* Obsolete planning helper removed from compilation.
    pub(crate) fn build_planning_prompt(
        user_message: &str,
        available_actions: &[crate::actions::ActionDef],
    ) -> (String, String) {
        let action_names: Vec<&str> = available_actions
            .iter()
            .take(12)
            .map(|a| a.name.as_str())
            .collect();

        let system = format!(
            "You are a task planner. Break the user's request into a concrete execution plan.\n\
            Return ONLY a JSON array of steps. Each step is an object with:\n\
            - \"title\": short action label (5-10 words)\n\
            - \"description\": what this step does (1 sentence)\n\
            - \"tool_hint\": which tool/action to use (from the list below, or null if no tool needed)\n\n\
            Available actions: {}\n\n\
            Rules:\n\
            - 2-8 steps maximum. Be concise.\n\
            - Each step should be one logical action, not a sub-plan.\n\
            - Order matters — steps execute sequentially.\n\
            - Use a tool_hint only when it exactly matches one available action name; otherwise return null.\n\
            - The last step should present/summarize the result to the user.\n\
            - Return ONLY the JSON array. No markdown fences, no explanation.",
            action_names.join(", ")
        );

        let user = format!(
            "Break this request into an execution plan:\n\n{}",
            user_message
        );

        (system, user)
    }

    */
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
