//! Telegram bot channel
use anyhow::{anyhow, Result};
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{BotCommand, ParseMode};
use tokio::sync::RwLock;

use crate::core::{Agent, Task, TaskApproval, TaskStatus};

type SharedAgent = Arc<RwLock<Agent>>;

#[derive(Clone, Copy, Debug)]
enum TunnelControlCommand {
    Start,
    Stop,
    Status,
}

fn parse_tunnel_command(text: &str) -> Option<TunnelControlCommand> {
    let normalized = text.trim().to_ascii_lowercase().replace(['_', '-'], " ");
    let compact = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    match compact.as_str() {
        "/tunnel start" | "/start tunnel" => Some(TunnelControlCommand::Start),
        "/tunnel stop" | "/stop tunnel" => Some(TunnelControlCommand::Stop),
        "/tunnel" | "/tunnel status" => Some(TunnelControlCommand::Status),
        _ => None,
    }
}

fn internal_api_base_url() -> String {
    crate::core::net::internal_api_base_url()
}

async fn execute_tunnel_command(agent: &SharedAgent, cmd: TunnelControlCommand) -> String {
    let api_key = { agent.read().await.api_key.clone() };
    let base_url = internal_api_base_url();
    let client = match crate::core::net::build_internal_control_client(10) {
        Ok(c) => c,
        Err(e) => return format!("Tunnel command failed: {}", e),
    };
    let url = match cmd {
        TunnelControlCommand::Start => format!("{}/tunnel/start", base_url),
        TunnelControlCommand::Stop => format!("{}/tunnel/stop", base_url),
        TunnelControlCommand::Status => format!("{}/tunnel/status", base_url),
    };

    let mut request = match cmd {
        TunnelControlCommand::Status => client.get(&url),
        TunnelControlCommand::Start | TunnelControlCommand::Stop => client.post(&url),
    };
    if let Some(key) = api_key {
        request = request.bearer_auth(key);
    }

    let response = match request.send().await {
        Ok(r) => r,
        Err(e) => return format!("Failed to reach tunnel controller at {}: {}", base_url, e),
    };
    let status = response.status();
    let json: serde_json::Value = match response.json().await {
        Ok(v) => v,
        Err(e) => return format!("Tunnel command failed (invalid response): {}", e),
    };

    if !status.is_success() {
        let err = json
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return format!("Tunnel command failed: {}", err);
    }

    match cmd {
        TunnelControlCommand::Start => {
            let url = json.get("url").and_then(|v| v.as_str()).unwrap_or("");
            if !url.is_empty() {
                format!("Tunnel started.\nExternal URL: {}", url)
            } else {
                json.get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Tunnel is starting; URL pending.")
                    .to_string()
            }
        }
        TunnelControlCommand::Stop => json
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Tunnel stopped.")
            .to_string(),
        TunnelControlCommand::Status => {
            let active = json
                .get("active")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut out = format!(
                "Tunnel status: {}",
                if active { "active" } else { "inactive" }
            );
            if let Some(url) = json.get("url").and_then(|v| v.as_str()) {
                if !url.is_empty() {
                    out.push_str(&format!("\nExternal URL: {}", url));
                }
            }
            if let Some(err) = json.get("error").and_then(|v| v.as_str()) {
                if !err.is_empty() {
                    out.push_str(&format!("\nLast error: {}", err));
                }
            }
            out
        }
    }
}

async fn process_telegram_prompt(
    agent: &SharedAgent,
    prompt: &str,
    conversation_id: &str,
) -> String {
    let agent_snapshot = Agent::snapshot(agent).await;
    match agent_snapshot
        .process_message_with_meta(prompt, "telegram", Some(conversation_id), None)
        .await
    {
        Ok(processed) => Agent::render_plain_channel_response(processed),
        Err(error) => format!("Error: {}", error),
    }
}

fn parse_set_secret(text: &str) -> Option<(String, String)> {
    crate::core::secrets::parse_set_secret_command(text)
}

fn parse_use_current_llm_key(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("/usecurrentkey ") {
        let key = trimmed[15..].trim();
        if key.is_empty()
            || key.chars().any(|c| c.is_whitespace())
            || key.contains('\n')
            || key.contains('\r')
        {
            return None;
        }
        return Some(key.to_string());
    }
    crate::core::secrets::parse_use_current_llm_key_command(trimmed)
}

async fn store_secret_for_chat(agent: &SharedAgent, key: &str, value: &str) -> Result<(), String> {
    let (config_dir, data_dir) = {
        let a = agent.read().await;
        (a.config_dir.clone(), a.data_dir.clone())
    };
    let k = key.to_string();
    let v = value.to_string();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        crate::core::secrets::store_user_secret(&config_dir, Some(&data_dir), &k, &v)
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(())
}

async fn link_current_llm_key_for_chat(agent: &SharedAgent, key: &str) -> Result<String, String> {
    let llm_env = {
        let a = agent.read().await;
        a.config.llm.app_env_vars()
    };
    if let Some(value) = llm_env.get(key).cloned().filter(|v| !v.trim().is_empty()) {
        store_secret_for_chat(agent, key, &value).await?;
        return Ok(format!(
            "Linked '{}' to the current model credential (stored encrypted).",
            key
        ));
    }

    let mut available_keys: Vec<String> = llm_env
        .iter()
        .filter_map(|(k, v)| {
            if v.trim().is_empty() {
                None
            } else if k.ends_with("_API_KEY")
                || k.ends_with("_BASE_URL")
                || k == "LLM_MODEL"
                || k == "LLM_PROVIDER"
            {
                Some(k.clone())
            } else {
                None
            }
        })
        .collect();
    available_keys.sort();
    let available = if available_keys.is_empty() {
        "none".to_string()
    } else {
        available_keys.join(", ")
    };
    Err(format!(
        "I can't map '{}' from current model settings. Available model-backed keys: {}. Save a credential for this key in the secure web UI.",
        key, available
    ))
}

fn escape_telegram_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

fn markdown_inline_to_telegram_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '*' && chars.peek() == Some(&'*') {
            chars.next();
            let mut inner = String::new();
            let mut closed = false;
            while let Some(next) = chars.next() {
                if next == '*' && chars.peek() == Some(&'*') {
                    chars.next();
                    closed = true;
                    break;
                }
                inner.push(next);
            }
            if closed {
                out.push_str("<b>");
                out.push_str(&escape_telegram_html(&inner));
                out.push_str("</b>");
            } else {
                out.push_str("**");
                out.push_str(&escape_telegram_html(&inner));
            }
        } else if c == '_' && chars.peek() == Some(&'_') {
            chars.next();
            let mut inner = String::new();
            let mut closed = false;
            while let Some(next) = chars.next() {
                if next == '_' && chars.peek() == Some(&'_') {
                    chars.next();
                    closed = true;
                    break;
                }
                inner.push(next);
            }
            if closed {
                out.push_str("<b>");
                out.push_str(&escape_telegram_html(&inner));
                out.push_str("</b>");
            } else {
                out.push_str("__");
                out.push_str(&escape_telegram_html(&inner));
            }
        } else if c == '`' {
            let mut inner = String::new();
            let mut closed = false;
            for next in chars.by_ref() {
                if next == '`' {
                    closed = true;
                    break;
                }
                inner.push(next);
            }
            if closed {
                out.push_str("<code>");
                out.push_str(&escape_telegram_html(&inner));
                out.push_str("</code>");
            } else {
                out.push('`');
                out.push_str(&escape_telegram_html(&inner));
            }
        } else if c == '*' {
            let mut inner = String::new();
            let mut closed = false;
            for next in chars.by_ref() {
                if next == '*' {
                    closed = true;
                    break;
                }
                inner.push(next);
            }
            if closed && !inner.trim().is_empty() {
                out.push_str("<i>");
                out.push_str(&escape_telegram_html(&inner));
                out.push_str("</i>");
            } else {
                out.push('*');
                out.push_str(&escape_telegram_html(&inner));
            }
        } else if c == '[' {
            let mut label = String::new();
            let mut url = String::new();
            let mut found = false;
            while let Some(next) = chars.next() {
                if next == ']' && chars.peek() == Some(&'(') {
                    chars.next();
                    for url_ch in chars.by_ref() {
                        if url_ch == ')' {
                            found = true;
                            break;
                        }
                        url.push(url_ch);
                    }
                    break;
                }
                label.push(next);
            }
            if found && !label.trim().is_empty() && !url.trim().is_empty() {
                out.push_str("<a href=\"");
                out.push_str(&escape_telegram_html(url.trim()));
                out.push_str("\">");
                out.push_str(&escape_telegram_html(label.trim()));
                out.push_str("</a>");
            } else {
                out.push('[');
                out.push_str(&escape_telegram_html(&label));
            }
        } else {
            out.push_str(&escape_telegram_html(&c.to_string()));
        }
    }
    out
}

fn markdown_wrapped_bold(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    trimmed
        .strip_prefix("**")
        .and_then(|value| value.strip_suffix("**"))
        .or_else(|| {
            trimmed
                .strip_prefix("__")
                .and_then(|value| value.strip_suffix("__"))
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn is_markdown_rule(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.len() >= 3
        && trimmed
            .chars()
            .all(|c| c == '-' || c == '*' || c == '_' || c.is_whitespace())
        && trimmed
            .chars()
            .filter(|c| matches!(c, '-' | '*' | '_'))
            .count()
            >= 3
}

/// Convert common Markdown to Telegram HTML.
fn markdown_to_telegram_html(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut result = String::with_capacity(normalized.len());
    let mut in_code_block = false;
    let mut code_block = String::new();

    for raw_line in normalized.lines() {
        let line = raw_line.trim_end();
        let trimmed = line.trim();

        if trimmed.starts_with("```") {
            if in_code_block {
                result.push_str("<pre>");
                result.push_str(&escape_telegram_html(code_block.trim_end()));
                result.push_str("</pre>\n");
                code_block.clear();
                in_code_block = false;
            } else {
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            code_block.push_str(line);
            code_block.push('\n');
            continue;
        }

        if is_markdown_rule(trimmed) {
            if !result.ends_with("\n\n") && !result.is_empty() {
                result.push('\n');
            }
            continue;
        }

        if let Some(stripped) = trimmed.strip_prefix("# ") {
            result.push_str("<b>");
            result.push_str(&markdown_inline_to_telegram_html(stripped.trim()));
            result.push_str("</b>\n");
            continue;
        }
        if let Some(stripped) = trimmed.strip_prefix("## ") {
            result.push_str("<b>");
            result.push_str(&markdown_inline_to_telegram_html(stripped.trim()));
            result.push_str("</b>\n");
            continue;
        }
        if let Some(stripped) = trimmed.strip_prefix("### ") {
            result.push_str("<b>");
            result.push_str(&markdown_inline_to_telegram_html(stripped.trim()));
            result.push_str("</b>\n");
            continue;
        }

        let content = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .map(|item| {
                if let Some(heading) = markdown_wrapped_bold(item) {
                    format!("<b>{}</b>", markdown_inline_to_telegram_html(heading))
                } else {
                    format!("- {}", markdown_inline_to_telegram_html(item.trim()))
                }
            })
            .unwrap_or_else(|| markdown_inline_to_telegram_html(line));
        result.push_str(&content);
        result.push('\n');
    }

    if in_code_block && !code_block.trim().is_empty() {
        result.push_str("<pre>");
        result.push_str(&escape_telegram_html(code_block.trim_end()));
        result.push_str("</pre>\n");
    }

    result.trim().to_string()
}

/// Register bot commands with Telegram (shows in / menu)
async fn register_commands(bot: &Bot) {
    let commands = vec![
        BotCommand::new("help", "Show all commands"),
        BotCommand::new("status", "Agent status"),
        BotCommand::new("image", "Generate an image - /image <prompt>"),
        BotCommand::new("video", "Generate a video - /video <prompt>"),
        BotCommand::new("remind", "Set reminder - /remind <time> <message>"),
        BotCommand::new("weather", "Get weather - /weather [location]"),
        BotCommand::new("translate", "Translate text - /translate <text>"),
        BotCommand::new("summarize", "Summarize our conversation"),
        BotCommand::new("search", "Web search - /search <query>"),
        BotCommand::new("todo", "Manage todo list"),
        BotCommand::new("note", "Save a note - /note <text>"),
        BotCommand::new("tasks", "View pending tasks"),
        BotCommand::new("actions", "List available actions"),
        BotCommand::new("memory", "Memory stats"),
        BotCommand::new("model", "Switch LLM model - /model <name>"),
        BotCommand::new("settings", "View settings"),
        BotCommand::new("tunnel", "Tunnel control - /tunnel [start|stop|status]"),
        BotCommand::new("new", "Start a new conversation"),
        BotCommand::new("clear", "Clear conversation history"),
    ];

    match bot.set_my_commands(commands).await {
        Ok(_) => tracing::info!("Telegram commands registered successfully"),
        Err(e) => tracing::warn!("Failed to register Telegram commands: {}", e),
    }
}

/// Start the Telegram bot
pub async fn serve(
    agent: SharedAgent,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let config = {
        let agent = agent.read().await;
        agent.config.telegram.clone()
    };

    let Some(telegram_config) = config else {
        tracing::info!("Telegram not configured, skipping Telegram bot");
        return Ok(());
    };

    tracing::info!("Starting Telegram bot");
    if !telegram_config.allowed_users.is_empty() {
        tracing::info!(
            "Telegram allowed users: {} configured",
            telegram_config.allowed_users.len()
        );
    } else {
        tracing::info!("Telegram: All users allowed (no restriction)");
    }

    let bot = Bot::new(&telegram_config.bot_token);

    match bot.get_me().await {
        Ok(me) => {
            let username = me.user.username.unwrap_or_else(|| "unknown".to_string());
            tracing::info!("Telegram bot authenticated as @{}", username);
        }
        Err(e) => {
            return Err(anyhow!("Telegram bot token validation failed: {}", e));
        }
    }

    // Register commands with Telegram (shows in / menu)
    register_commands(&bot).await;

    let agent_clone = agent.clone();

    let bot_loop = teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let agent = agent_clone.clone();
        async move {
            let user_id = msg.from.as_ref().map(|u| u.id.0);
            let username = msg
                .from
                .as_ref()
                .and_then(|u| u.username.clone())
                .unwrap_or_else(|| "unknown".to_string());
            let chat_id = msg.chat.id;

            if let Some(text) = msg.text() {
                tracing::info!(
                    "Telegram message: user={}(@{}), chat={}, msg={}chars",
                    user_id.unwrap_or(0),
                    username,
                    chat_id,
                    text.len()
                );

                // Check authorization
                let authorized = {
                    let agent = agent.read().await;
                    if let Some(config) = &agent.config.telegram {
                        if config.allowed_users.is_empty() {
                            true
                        } else {
                            msg.from
                                .as_ref()
                                .map(|u| config.allowed_users.contains(&(u.id.0 as i64)))
                                .unwrap_or(false)
                        }
                    } else {
                        true
                    }
                };

                if !authorized {
                    tracing::warn!(
                        "Telegram: unauthorized user {}(@{}) rejected",
                        user_id.unwrap_or(0),
                        username
                    );
                    {
                        let agent = agent.read().await;
                        agent.security_events.record_unauthorized_channel_attempt();
                    }
                    bot.send_message(chat_id, "You are not authorized.").await?;
                    return Ok(());
                }

                // Persist last chat id for push notifications
                {
                    let agent = agent.read().await;
                    let _ = agent
                        .storage
                        .set("telegram:last_chat_id", chat_id.0.to_string().as_bytes())
                        .await;
                }

                // Handle commands
                if text.starts_with('/') {
                    // Store secrets without engaging the LLM. Only allow in private chats and only
                    // when an allowlist is configured (otherwise the bot could be public).
                    let lower_text = text.to_ascii_lowercase();
                    if lower_text.starts_with("/setsecret")
                        || lower_text.starts_with("/usecurrentkey")
                    {
                        if !crate::core::secrets::secret_command_escape_hatch_enabled() {
                            bot.send_message(
                                chat_id,
                                crate::core::secrets::setsecret_command_disabled_response(),
                            )
                            .await?;
                            return Ok(());
                        }
                        let allow_set_secret = {
                            let a = agent.read().await;
                            a.config
                                .telegram
                                .as_ref()
                                .map(|c| !c.allowed_users.is_empty())
                                .unwrap_or(false)
                        };
                        let conversation_id = format!("telegram:{}", chat_id.0);

                        let reply = if !msg.chat.is_private() {
                            "Refusing to store secrets in non-private chats. Use a direct message or the web UI."
                                .to_string()
                        } else if !allow_set_secret {
                            "Refusing to store secrets via Telegram until `telegram.allowed_users` is configured in Settings. Use the web UI instead."
                                .to_string()
                        } else if let Some((key, value)) = parse_set_secret(text) {
                            match store_secret_for_chat(&agent, &key, &value).await {
                                Ok(()) => {
                                    let followup = {
                                        let a = agent.read().await;
                                        a.on_secret_saved_followup(&conversation_id).await
                                    };
                                    let mut response = format!(
                                        "Saved secret '{}' (stored encrypted). This value was not sent to the LLM.",
                                        key
                                    );
                                    if let Some(f) = followup {
                                        response.push_str("\n\n");
                                        response.push_str(&f);
                                    }
                                    response
                                }
                                Err(e) => format!("Failed to store secret: {}", e),
                            }
                        } else if let Some(key) = parse_use_current_llm_key(text) {
                            match link_current_llm_key_for_chat(&agent, &key).await {
                                Ok(prefix) => {
                                    let followup = {
                                        let a = agent.read().await;
                                        a.on_secret_saved_followup(&conversation_id).await
                                    };
                                    let mut response = prefix;
                                    if let Some(f) = followup {
                                        response.push_str("\n\n");
                                        response.push_str(&f);
                                    }
                                    response
                                }
                                Err(e) => e,
                            }
                        } else {
                            "Use the secure credential form in the web UI to save credentials."
                                .to_string()
                        };

                        bot.send_message(chat_id, reply).await?;
                        return Ok(());
                    }

                    let response = handle_command(text, &agent, chat_id).await;
                    bot.send_message(chat_id, response).await?;
                } else {
                    // Internal escape hatch only. The product UX is the secure credential form.
                    let allow_set_secret = {
                        let a = agent.read().await;
                        a.config
                            .telegram
                            .as_ref()
                            .map(|c| !c.allowed_users.is_empty())
                            .unwrap_or(false)
                    };
                    if msg.chat.is_private()
                        && allow_set_secret
                        && crate::core::secrets::setsecret_command_escape_hatch_enabled()
                    {
                        if let Some((key, value)) = parse_set_secret(text) {
                            let conversation_id = format!("telegram:{}", chat_id.0);
                            let reply = match store_secret_for_chat(&agent, &key, &value).await {
                                Ok(()) => {
                                    let followup = {
                                        let a = agent.read().await;
                                        a.on_secret_saved_followup(&conversation_id).await
                                    };
                                    let mut response = format!(
                                        "Saved secret '{}' (stored encrypted). This value was not sent to the LLM.",
                                        key
                                    );
                                    if let Some(f) = followup {
                                        response.push_str("\n\n");
                                        response.push_str(&f);
                                    }
                                    response
                                }
                                Err(e) => format!("Failed to store secret: {}", e),
                            };
                            bot.send_message(chat_id, reply).await?;
                            return Ok(());
                        }
                        if let Some(key) = parse_use_current_llm_key(text) {
                            let conversation_id = format!("telegram:{}", chat_id.0);
                            let reply = match link_current_llm_key_for_chat(&agent, &key).await {
                                Ok(prefix) => {
                                    let followup = {
                                        let a = agent.read().await;
                                        a.on_secret_saved_followup(&conversation_id).await
                                    };
                                    let mut response = prefix;
                                    if let Some(f) = followup {
                                        response.push_str("\n\n");
                                        response.push_str(&f);
                                    }
                                    response
                                }
                                Err(e) => e,
                            };
                            bot.send_message(chat_id, reply).await?;
                            return Ok(());
                        }
                    }

                    if let Some(cmd) = parse_tunnel_command(text) {
                        let reply = execute_tunnel_command(&agent, cmd).await;
                        bot.send_message(chat_id, reply).await?;
                        return Ok(());
                    }

                    // Show "typing..." indicator while processing
                    let _ = bot
                        .send_chat_action(chat_id, teloxide::types::ChatAction::Typing)
                        .await;

                    // Keep typing indicator alive in background (Telegram typing expires after ~5s)
                    let typing_bot = bot.clone();
                    let typing_chat_id = chat_id;
                    let typing_done =
                        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let typing_flag = typing_done.clone();
                    crate::spawn_logged!("src/channels/telegram.rs:705", async move {
                        loop {
                            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                            if typing_flag.load(std::sync::atomic::Ordering::Relaxed) {
                                break;
                            }
                            let _ = typing_bot
                                .send_chat_action(
                                    typing_chat_id,
                                    teloxide::types::ChatAction::Typing,
                                )
                                .await;
                        }
                    });

                    // Process with agent
                    let conversation_id = format!("telegram:{}", chat_id.0);
                    let (response, _trace_ref) = {
                        let trace_ref = agent.read().await.last_trace.clone();
                        let response =
                            process_telegram_prompt(&agent, text, &conversation_id).await;
                        (response, trace_ref)
                    };
                    typing_done.store(true, std::sync::atomic::Ordering::Relaxed);

                    let chunks = super::outbound_split::split_for_provider_safe_channel(
                        "telegram", &response,
                    );
                    for chunk in chunks {
                        bot.send_message(chat_id, markdown_to_telegram_html(&chunk))
                            .parse_mode(ParseMode::Html)
                            .await?;
                    }
                }
            }
            Ok(())
        }
    });

    tokio::select! {
        _ = shutdown_rx.changed() => {
            tracing::info!("Telegram bot shutdown signal received");
        }
        _ = bot_loop => {}
    }

    Ok(())
}

pub async fn send_message(agent: &Agent, text: &str) -> Result<()> {
    let Some(config) = &agent.config.telegram else {
        let message = "Telegram is not configured";
        tracing::warn!("Telegram send_message: {}", message);
        return Err(anyhow!(message));
    };

    let Some(chat_id) = configured_notification_chat_id(config) else {
        let message = "Telegram proactive delivery is fail-closed until exactly one allowed user ID is configured.";
        tracing::warn!("Telegram send_message: {}", message);
        return Err(anyhow!(message));
    };

    let bot = Bot::new(&config.bot_token);
    for chunk in super::outbound_split::split_for_provider_safe_channel("telegram", text) {
        bot.send_message(ChatId(chat_id), markdown_to_telegram_html(&chunk))
            .parse_mode(ParseMode::Html)
            .await?;
    }
    Ok(())
}

/// Resolve the configured DM target for proactive Telegram notifications.
pub(crate) fn configured_notification_chat_id(
    config: &crate::core::config::TelegramConfig,
) -> Option<i64> {
    if config.allowed_users.len() != 1 {
        return None;
    }
    let chat_id = config.allowed_users.first().copied().unwrap_or_default();
    if chat_id != 0 {
        return Some(chat_id);
    }
    tracing::warn!("Telegram: no chat_id available - user must send a message to the bot first");
    None
}

/// Send a photo (screenshot) with optional caption to the configured Telegram DM target.
pub async fn send_photo(agent: &Agent, image_bytes: &[u8], caption: &str) -> Result<()> {
    let Some(config) = &agent.config.telegram else {
        return Ok(());
    };
    let Some(chat_id) = configured_notification_chat_id(config) else {
        return Ok(());
    };

    let bot = Bot::new(&config.bot_token);
    let input_file =
        teloxide::types::InputFile::memory(image_bytes.to_vec()).file_name("screenshot.png");
    bot.send_photo(ChatId(chat_id), input_file)
        .caption(caption)
        .await?;
    Ok(())
}

/// Send a video with optional caption to the configured Telegram DM target.
pub async fn send_video(agent: &Agent, video_bytes: &[u8], caption: &str) -> Result<()> {
    let Some(config) = &agent.config.telegram else {
        return Ok(());
    };
    let Some(chat_id) = configured_notification_chat_id(config) else {
        return Ok(());
    };

    let bot = Bot::new(&config.bot_token);
    let input_file =
        teloxide::types::InputFile::memory(video_bytes.to_vec()).file_name("video.mp4");
    bot.send_video(ChatId(chat_id), input_file)
        .caption(caption)
        .await?;
    Ok(())
}

async fn handle_command(text: &str, agent: &SharedAgent, chat_id: ChatId) -> String {
    let parts: Vec<&str> = text.splitn(2, ' ').collect();
    let command = parts.first().unwrap_or(&"");
    let args = parts.get(1).map(|s| s.trim()).unwrap_or("");
    let conversation_id = format!("telegram:{}", chat_id.0);

    match *command {
        "/start" | "/help" => {
            let agent = agent.read().await;
            format!(
                "Welcome to {}!\n\n\
                Media:\n\
                /image <prompt> - Generate image\n\
                /video <prompt> - Generate video\n\n\
                Productivity:\n\
                /remind <time> <msg> - Set reminder\n\
                /todo - View todo list\n\
                /todo add <item> - Add todo\n\
                /note <text> - Save a note\n\
                /tasks - View pending tasks\n\
                /approve-task <task_id> - Approve a waiting task\n\
                /reject-task <task_id> - Reject a waiting task\n\n\
                Utilities:\n\
                /weather [location] - Get weather\n\
                /translate <text> - Translate\n\
                /search <query> - Web search\n\
                /summarize - Summarize chat\n\n\
                Settings:\n\
                /status - Agent status\n\
                /skills - List skills\n\
                /memory - Memory stats\n\
                /model <name> - Switch model\n\
                /settings - View settings\n\
                /install <url> - Install a skill from URL\n\
                /tunnel [start|stop|status] - Manage remote UI access\n\
                /run <skill> [query] - Run a custom skill\n\
                /new - Start a new conversation\n\
                /clear - Clear conversation history\n\n\
                Or just chat with me!",
                agent.config.name
            )
        }

        "/status" => {
            let agent = agent.read().await;
            let status = agent.status().await;
            format!(
                "Agent status\n\n\
                DID: {}\n\
                Memory: {} entries\n\
                Skills: {} loaded\n\
                Tasks: {} pending",
                status.did, status.memory_entries, status.actions_loaded, status.tasks_pending
            )
        }

        "/settings" => {
            let agent = agent.read().await;
            let model = match &agent.config.llm {
                crate::core::LlmProvider::Ollama { model, .. } => format!("Ollama: {}", model),
                crate::core::LlmProvider::Anthropic { model, .. } => {
                    format!("Anthropic: {}", model)
                }
                crate::core::LlmProvider::OpenAI { model, .. } => format!("OpenAI: {}", model),
            };
            let fallback = agent
                .config
                .llm_fallback
                .as_ref()
                .map(|fb| match fb {
                    crate::core::LlmProvider::Ollama { model, .. } => format!("Ollama: {}", model),
                    crate::core::LlmProvider::Anthropic { model, .. } => {
                        format!("Anthropic: {}", model)
                    }
                    crate::core::LlmProvider::OpenAI { model, .. } => format!("OpenAI: {}", model),
                })
                .unwrap_or_else(|| "None".to_string());

            format!(
                "Current settings\n\n\
                Bot: {}\n\
                Personality: {}\n\
                Model: {}\n\
                Fallback: {}",
                agent.config.name, agent.config.personality, model, fallback
            )
        }

        "/tunnel" => {
            let cmd = if args.is_empty() {
                TunnelControlCommand::Status
            } else if args.eq_ignore_ascii_case("start") {
                TunnelControlCommand::Start
            } else if args.eq_ignore_ascii_case("stop") {
                TunnelControlCommand::Stop
            } else if args.eq_ignore_ascii_case("status") {
                TunnelControlCommand::Status
            } else {
                return "Usage: /tunnel [start|stop|status]\nExample: /tunnel start".to_string();
            };
            execute_tunnel_command(agent, cmd).await
        }

        "/skills" | "/skill" => {
            let agent = agent.read().await;
            let actions = agent.runtime.list_actions().await.unwrap_or_default();
            if actions.is_empty() {
                "No skills loaded".to_string()
            } else {
                let list = actions
                    .iter()
                    .take(15) // Limit to prevent too long message
                    .map(|s| format!("- {} - {}", s.name, s.description))
                    .collect::<Vec<_>>()
                    .join("\n");
                let more = if actions.len() > 15 {
                    format!("\n\n...and {} more", actions.len() - 15)
                } else {
                    String::new()
                };
                format!("Available skills:\n\n{}{}", list, more)
            }
        }

        "/memory" => {
            let agent = agent.read().await;
            let status = agent.status().await;
            format!(
                "Memory stats\n\n\
                Entries: {}\n\
                Skills: {} loaded\n\
                Tasks: {} pending",
                status.memory_entries, status.actions_loaded, status.tasks_pending
            )
        }

        "/image" => {
            if args.is_empty() {
                "Usage: /image <prompt>\n\nExample: /image a cute robot playing guitar".to_string()
            } else {
                // Process through agent with image generation intent
                let response = {
                    let prompt = format!("Generate an image of: {}", args);
                    process_telegram_prompt(agent, &prompt, &conversation_id).await
                };
                response
            }
        }

        "/video" => {
            if args.is_empty() {
                "Usage: /video <prompt>\n\nExample: /video a rocket launching into space"
                    .to_string()
            } else {
                let prompt = format!("Generate a video of: {}", args);
                let response = process_telegram_prompt(agent, &prompt, &conversation_id).await;
                response
            }
        }

        "/remind" => {
            if args.is_empty() {
                "Usage: /remind <time> <message>\n\nExamples:\n/remind 5m Check the oven\n/remind 2h Call mom\n/remind tomorrow 9am Meeting".to_string()
            } else {
                let prompt = format!("Set a reminder: {}", args);
                let response = process_telegram_prompt(agent, &prompt, &conversation_id).await;
                response
            }
        }

        "/weather" => {
            let location = if args.is_empty() { "my location" } else { args };
            let prompt = format!("What's the weather in {}?", location);
            let response = process_telegram_prompt(agent, &prompt, &conversation_id).await;
            response
        }

        "/translate" => {
            if args.is_empty() {
                "Usage: /translate <text>\n\nExample: /translate Hello, how are you? to Spanish"
                    .to_string()
            } else {
                let prompt = format!("Translate: {}", args);
                let response = process_telegram_prompt(agent, &prompt, &conversation_id).await;
                response
            }
        }

        "/search" => {
            if args.is_empty() {
                "Usage: /search <query>\n\nExample: /search latest news about AI".to_string()
            } else {
                let prompt = format!("Search the web for: {}", args);
                let response = process_telegram_prompt(agent, &prompt, &conversation_id).await;
                response
            }
        }

        "/install" => {
            if args.is_empty() {
                "Usage: /install <skill_url>".to_string()
            } else {
                let prompt = format!("install this skill {}", args.trim());
                process_telegram_prompt(agent, &prompt, &conversation_id).await
            }
        }

        "/summarize" => {
            let response = process_telegram_prompt(
                agent,
                "Summarize our recent conversation",
                &conversation_id,
            )
            .await;
            response
        }

        "/todo" => {
            if args.is_empty() {
                // Show todo list
                let response = {
                    let agent = agent.read().await;
                    match agent
                        .process_message(
                            "Show my todo list",
                            "telegram",
                            Some(&conversation_id),
                            None,
                        )
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => format!("Error: {}", e),
                    }
                };
                response
            } else if args.starts_with("add ") {
                let item = args.strip_prefix("add ").unwrap_or("").trim();
                let response = {
                    let agent = agent.read().await;
                    let prompt = format!("Add to my todo list: {}", item);
                    match agent
                        .process_message(&prompt, "telegram", Some(&conversation_id), None)
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => format!("Error: {}", e),
                    }
                };
                response
            } else {
                "Usage:\n/todo - Show list\n/todo add <item> - Add item".to_string()
            }
        }

        "/note" => {
            if args.is_empty() {
                "Usage: /note <text>\n\nExample: /note Remember to buy milk".to_string()
            } else {
                let response = {
                    let agent = agent.read().await;
                    let prompt = format!("Save this note: {}", args);
                    match agent
                        .process_message(&prompt, "telegram", Some(&conversation_id), None)
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => format!("Error: {}", e),
                    }
                };
                response
            }
        }

        "/tasks" => {
            let agent = agent.read().await;
            let tasks = agent.tasks.read().await;
            let pending: Vec<_> = tasks
                .all()
                .iter()
                .filter(|t| matches!(t.status, TaskStatus::Pending | TaskStatus::AwaitingApproval))
                .take(10)
                .collect();

            if pending.is_empty() {
                "No pending tasks".to_string()
            } else {
                let list = pending
                    .iter()
                    .map(|t| {
                        let status = match t.status {
                            TaskStatus::AwaitingApproval => "[approval]",
                            TaskStatus::Pending => "[pending]",
                            _ => "-",
                        };
                        if matches!(t.status, TaskStatus::AwaitingApproval) {
                            format!("{} {} [{}]", status, t.description, t.id)
                        } else {
                            format!("{} {}", status, t.description)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("Pending tasks:\n\n{}", list)
            }
        }

        "/approve-task" => {
            if args.is_empty() {
                "Usage: /approve-task <task_id>".to_string()
            } else {
                let Ok(task_id) = uuid::Uuid::parse_str(args) else {
                    return "Invalid task id. Use the full task UUID shown in /tasks or the approval notification.".to_string();
                };
                let agent = agent.read().await;
                match agent.approve_task_request(task_id, "telegram").await {
                    Ok(Some(task)) => format!("Approved: {}", task.description),
                    Ok(None) => "Task not found or is not awaiting approval.".to_string(),
                    Err(e) => format!("Failed to approve task: {}", e),
                }
            }
        }

        "/reject-task" => {
            if args.is_empty() {
                "Usage: /reject-task <task_id>".to_string()
            } else {
                let Ok(task_id) = uuid::Uuid::parse_str(args) else {
                    return "Invalid task id. Use the full task UUID shown in /tasks or the approval notification.".to_string();
                };
                let agent = agent.read().await;
                match agent
                    .reject_task_request(
                        task_id,
                        "telegram",
                        "Task was rejected from Telegram and will not be executed.",
                    )
                    .await
                {
                    Ok(Some(task)) => format!("Rejected: {}", task.description),
                    Ok(None) => "Task not found or is not awaiting approval.".to_string(),
                    Err(e) => format!("Failed to reject task: {}", e),
                }
            }
        }

        "/new" => {
            let agent = agent.read().await;
            match agent
                .start_new_channel_conversation("telegram", &conversation_id, None, "New Chat")
                .await
            {
                Ok(_) => "Started a new conversation. Previous history is kept.".to_string(),
                Err(e) => format!("Failed to start a new conversation: {}", e),
            }
        }

        "/clear" => {
            let agent = agent.read().await;
            match agent
                .clear_current_channel_conversation("telegram", &conversation_id, None)
                .await
            {
                Ok(_) => "Conversation cleared. Starting fresh.".to_string(),
                Err(e) => format!("Failed to clear conversation: {}", e),
            }
        }

        "/model" => {
            if args.is_empty() {
                let agent = agent.read().await;
                let current = match &agent.config.llm {
                    crate::core::LlmProvider::Ollama { model, .. } => model.clone(),
                    crate::core::LlmProvider::Anthropic { model, .. } => model.clone(),
                    crate::core::LlmProvider::OpenAI { model, .. } => model.clone(),
                };
                format!(
                    "Current model: {}\n\nUsage: /model <model_name>\n\nNote: Changing models requires restart",
                    current
                )
            } else {
                format!(
                    "Model change to '{}' noted.\n\nTo apply, please update via web UI settings and restart.",
                    args
                )
            }
        }

        "/run" => {
            let rest = args.trim();
            if rest.is_empty() {
                "Usage: /run <skill_name> [query]".to_string()
            } else {
                let mut parts = rest.splitn(2, char::is_whitespace);
                let skill_name = parts.next().unwrap_or("").trim();
                let query = parts.next().unwrap_or("").trim();
                let prompt = if query.is_empty() {
                    format!("run {}", skill_name)
                } else {
                    format!("run {} {}", skill_name, query)
                };
                let agent = agent.read().await;
                match agent
                    .process_message(&prompt, "telegram", Some(&conversation_id), None)
                    .await
                {
                    Ok(r) => r,
                    Err(e) => format!("Error: {}", e),
                }
            }
        }

        cmd if cmd.starts_with("/task ") => {
            let description = args;
            if description.is_empty() {
                "Usage: /task <description>".to_string()
            } else {
                let task = Task {
                    id: uuid::Uuid::new_v4(),
                    description: description.to_string(),
                    action: "telegram".to_string(),
                    arguments: serde_json::json!({ "description": description }),
                    approval: TaskApproval::Auto,
                    capabilities: vec!["telegram".to_string()],
                    status: TaskStatus::Pending,
                    created_at: chrono::Utc::now(),
                    scheduled_for: None,
                    cron: None,
                    result: None,
                    proof_id: None,
                    priority: None,
                    urgency: None,
                    importance: None,
                    eisenhower_quadrant: None,
                };

                let add_result = {
                    let agent = agent.read().await;
                    agent.add_task(task).await
                };

                match add_result {
                    Ok(_) => format!("Task created: {}", description),
                    Err(e) => format!("Failed to create task: {}", e),
                }
            }
        }

        _ => format!(
            "Unknown command: {}\n\nType /help for all commands",
            command
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_notification_chat_id_requires_exactly_one_allowed_user() {
        let mut config = crate::core::config::TelegramConfig {
            bot_token: "token".to_string(),
            allowed_users: vec![12345],
            dm_policy: "pairing".to_string(),
        };
        assert_eq!(configured_notification_chat_id(&config), Some(12345));

        config.allowed_users.clear();
        assert_eq!(configured_notification_chat_id(&config), None);

        config.allowed_users = vec![12345, 67890];
        assert_eq!(configured_notification_chat_id(&config), None);
    }
}
