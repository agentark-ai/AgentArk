//! Telegram bot channel

use anyhow::Result;
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
        "start tunnel" | "/tunnel start" | "/start_tunnel" => Some(TunnelControlCommand::Start),
        "stop tunnel" | "/tunnel stop" | "/stop_tunnel" => Some(TunnelControlCommand::Stop),
        "tunnel status" | "status tunnel" | "/tunnel" | "/tunnel status" | "/tunnel_status" => {
            Some(TunnelControlCommand::Status)
        }
        _ => None,
    }
}

fn internal_api_base_url() -> String {
    let bind_addr = std::env::var("AGENTARK_BIND").unwrap_or_else(|_| "127.0.0.1:8990".to_string());
    let tls_enabled = std::env::var("AGENTARK_TLS_CERT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .is_some()
        && std::env::var("AGENTARK_TLS_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .is_some();
    let scheme = if tls_enabled { "https" } else { "http" };
    format!("{}://{}", scheme, bind_addr)
}

async fn execute_tunnel_command(agent: &SharedAgent, cmd: TunnelControlCommand) -> String {
    let api_key = { agent.read().await.api_key.clone() };
    let base_url = internal_api_base_url();
    let client = match reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
    {
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

fn parse_set_secret(text: &str) -> Option<(String, String)> {
    // Accept both:
    // - "/setsecret KEY=VALUE" (Telegram command)
    // - "set secret KEY=VALUE" (plain text)
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    let rest = if lower.starts_with("/setsecret ") || lower.starts_with("set secret ") {
        trimmed[10..].trim() // len("set secret ") == 10
    } else {
        return None;
    };
    if rest.is_empty() {
        return None;
    }

    let (key, value) = if let Some(eq) = rest.find('=') {
        let (k, v) = rest.split_at(eq);
        (k.trim(), v[1..].trim())
    } else {
        let mut parts = rest.splitn(2, char::is_whitespace);
        let k = parts.next().unwrap_or("").trim();
        let v = parts.next().unwrap_or("").trim();
        (k, v)
    };
    if key.is_empty() || value.is_empty() {
        return None;
    }
    if key.chars().any(|c| c.is_whitespace()) {
        return None;
    }
    if key.contains('\n') || key.contains('\r') {
        return None;
    }
    Some((key.to_string(), value.to_string()))
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
        "I can't map '{}' from current model settings. Available model-backed keys: {}. You can set it manually with: set secret {}=VALUE",
        key, available, key
    ))
}

/// Split a message into chunks for Telegram (max 4096 chars)
/// Tries to split at paragraph boundaries for better formatting
fn split_message_for_telegram(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut current_chunk = String::new();

    for paragraph in text.split("\n\n") {
        let paragraph_with_break = if current_chunk.is_empty() {
            paragraph.to_string()
        } else {
            format!("\n\n{}", paragraph)
        };

        if current_chunk.len() + paragraph_with_break.len() <= max_len {
            current_chunk.push_str(&paragraph_with_break);
        } else {
            // If current paragraph is too long, split it by lines
            if paragraph.len() > max_len {
                if !current_chunk.is_empty() {
                    chunks.push(current_chunk);
                    current_chunk = String::new();
                }
                // Split long paragraph by lines
                for line in paragraph.lines() {
                    let line_with_break = if current_chunk.is_empty() {
                        line.to_string()
                    } else {
                        format!("\n{}", line)
                    };

                    if current_chunk.len() + line_with_break.len() <= max_len {
                        current_chunk.push_str(&line_with_break);
                    } else {
                        if !current_chunk.is_empty() {
                            chunks.push(current_chunk);
                        }
                        current_chunk = line.to_string();
                    }
                }
            } else {
                // Start new chunk with this paragraph
                if !current_chunk.is_empty() {
                    chunks.push(current_chunk);
                }
                current_chunk = paragraph.to_string();
            }
        }
    }

    if !current_chunk.is_empty() {
        chunks.push(current_chunk);
    }

    chunks
}

/// Convert markdown to Telegram HTML format
fn markdown_to_telegram_html(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            // Escape HTML special chars
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '&' => result.push_str("&amp;"),

            // Bold: **text** or __text__
            '*' if chars.peek() == Some(&'*') => {
                chars.next(); // consume second *
                let mut bold_text = String::new();
                while let Some(&next) = chars.peek() {
                    if next == '*' {
                        chars.next();
                        if chars.peek() == Some(&'*') {
                            chars.next();
                            break;
                        }
                        bold_text.push('*');
                    } else {
                        bold_text.push(chars.next().unwrap());
                    }
                }
                result.push_str(&format!("<b>{}</b>", bold_text));
            }

            // Headers: # text -> bold
            '#' if result.ends_with('\n') || result.is_empty() => {
                // Count # symbols
                let mut level = 1;
                while chars.peek() == Some(&'#') {
                    chars.next();
                    level += 1;
                }
                // Skip space after #
                if chars.peek() == Some(&' ') {
                    chars.next();
                }
                // Collect header text until newline
                let mut header_text = String::new();
                while let Some(&next) = chars.peek() {
                    if next == '\n' {
                        break;
                    }
                    header_text.push(chars.next().unwrap());
                }
                // Add emoji prefix for different levels
                let prefix = match level {
                    1 => "📌 ",
                    2 => "▸ ",
                    3 => "• ",
                    _ => "",
                };
                result.push_str(&format!("<b>{}{}</b>", prefix, header_text));
            }

            // Links: [text](url)
            '[' => {
                let mut link_text = String::new();
                let mut found_link = false;
                while let Some(&next) = chars.peek() {
                    if next == ']' {
                        chars.next();
                        if chars.peek() == Some(&'(') {
                            chars.next();
                            let mut url = String::new();
                            while let Some(&url_char) = chars.peek() {
                                if url_char == ')' {
                                    chars.next();
                                    break;
                                }
                                url.push(chars.next().unwrap());
                            }
                            result.push_str(&format!("<a href=\"{}\">{}</a>", url, link_text));
                            found_link = true;
                        }
                        break;
                    }
                    link_text.push(chars.next().unwrap());
                }
                if !found_link {
                    result.push('[');
                    result.push_str(&link_text);
                    result.push(']');
                }
            }

            // Inline code: `code`
            '`' if chars.peek() != Some(&'`') => {
                let mut code_text = String::new();
                while let Some(&next) = chars.peek() {
                    if next == '`' {
                        chars.next();
                        break;
                    }
                    code_text.push(chars.next().unwrap());
                }
                result.push_str(&format!("<code>{}</code>", code_text));
            }

            // Code block: ```code```
            '`' if chars.peek() == Some(&'`') => {
                chars.next(); // second `
                if chars.peek() == Some(&'`') {
                    chars.next(); // third `
                                  // Skip optional language identifier
                    while let Some(&next) = chars.peek() {
                        if next == '\n' {
                            chars.next();
                            break;
                        }
                        chars.next();
                    }
                    let mut code_block = String::new();
                    let mut backtick_count = 0;
                    while let Some(&next) = chars.peek() {
                        if next == '`' {
                            backtick_count += 1;
                            chars.next();
                            if backtick_count == 3 {
                                break;
                            }
                        } else {
                            if backtick_count > 0 {
                                for _ in 0..backtick_count {
                                    code_block.push('`');
                                }
                                backtick_count = 0;
                            }
                            code_block.push(chars.next().unwrap());
                        }
                    }
                    result.push_str(&format!("<pre>{}</pre>", code_block.trim()));
                } else {
                    result.push_str("``");
                }
            }

            // Horizontal rule: --- or *** or ___
            '-' if result.ends_with('\n') || result.is_empty() => {
                let mut dash_count = 1;
                while chars.peek() == Some(&'-') {
                    chars.next();
                    dash_count += 1;
                }
                if dash_count >= 3 {
                    result.push_str("─────────────────");
                } else {
                    for _ in 0..dash_count {
                        result.push('-');
                    }
                }
            }

            // Keep everything else
            _ => result.push(c),
        }
    }

    result
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
        BotCommand::new("settings", "View current settings"),
        BotCommand::new("tunnel", "Tunnel control - /tunnel [start|stop|status]"),
        BotCommand::new("clear", "Clear conversation history"),
    ];

    match bot.set_my_commands(commands).await {
        Ok(_) => tracing::info!("Telegram commands registered successfully"),
        Err(e) => tracing::warn!("Failed to register Telegram commands: {}", e),
    }
}

/// Start the Telegram bot
pub async fn serve(agent: SharedAgent) -> Result<()> {
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

    // Register commands with Telegram (shows in / menu)
    register_commands(&bot).await;

    let agent_clone = agent.clone();

    teloxide::repl(bot, move |bot: Bot, msg: Message| {
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
                            "Usage:\n/setsecret KEY=VALUE\nExample: /setsecret OPENAI_API_KEY=sk-...\n\nOr reuse your configured model key:\nuse current llm key for OPENAI_API_KEY"
                                .to_string()
                        };

                        bot.send_message(chat_id, reply).await?;
                        return Ok(());
                    }

                    let response = handle_command(text, &agent, chat_id).await;
                    bot.send_message(chat_id, response).await?;
                } else {
                    // Allow "set secret ..." in private chat only, and only when allowlist is configured.
                    // This stores the secret encrypted and does not send it to the LLM.
                    let allow_set_secret = {
                        let a = agent.read().await;
                        a.config
                            .telegram
                            .as_ref()
                            .map(|c| !c.allowed_users.is_empty())
                            .unwrap_or(false)
                    };
                    if msg.chat.is_private() && allow_set_secret {
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
                    let _ = bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await;

                    // Keep typing indicator alive in background (Telegram typing expires after ~5s)
                    let typing_bot = bot.clone();
                    let typing_chat_id = chat_id;
                    let typing_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let typing_flag = typing_done.clone();
                    tokio::spawn(async move {
                        loop {
                            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                            if typing_flag.load(std::sync::atomic::Ordering::Relaxed) {
                                break;
                            }
                            let _ = typing_bot.send_chat_action(typing_chat_id, teloxide::types::ChatAction::Typing).await;
                        }
                    });

                    // Process with agent
                    let conversation_id = format!("telegram:{}", chat_id.0);
                    let (response, _trace_ref) = {
                        let agent = agent.read().await;
                        let r = match agent
                            .process_message(text, "telegram", Some(&conversation_id), None)
                            .await
                        {
                            Ok(r) => r,
                            Err(e) => format!("Error: {}", e),
                        };
                        (r, agent.last_trace.clone())
                    };
                    typing_done.store(true, std::sync::atomic::Ordering::Relaxed);

                    // Convert markdown to Telegram HTML
                    let html_response = markdown_to_telegram_html(&response);

                    // Split long messages (Telegram limit is 4096 chars)
                    // Try to split at paragraph boundaries for better formatting
                    let chunks = split_message_for_telegram(&html_response, 4000);
                    for chunk in chunks {
                        bot.send_message(chat_id, chunk)
                            .parse_mode(ParseMode::Html)
                            .await?;
                    }
                }
            }
            Ok(())
        }
    })
    .await;

    Ok(())
}

pub async fn send_message(agent: &Agent, text: &str) -> Result<()> {
    let Some(config) = &agent.config.telegram else {
        tracing::debug!("Telegram send_message: no telegram config, skipping");
        return Ok(());
    };

    let Some(chat_id) = resolve_chat_id(agent, config).await else {
        return Ok(());
    };

    let bot = Bot::new(&config.bot_token);
    bot.send_message(ChatId(chat_id), text).await?;
    Ok(())
}

/// Resolve the chat_id to send to: stored last_chat_id > first allowed_user > None
async fn resolve_chat_id(
    agent: &Agent,
    config: &crate::core::config::TelegramConfig,
) -> Option<i64> {
    let stored = agent
        .storage
        .get("telegram:last_chat_id")
        .await
        .ok()
        .flatten();
    if let Some(bytes) = stored {
        let id: i64 = String::from_utf8_lossy(&bytes).parse().unwrap_or_default();
        if id != 0 {
            return Some(id);
        }
    }
    // Fallback: for private chats, user_id == chat_id
    if let Some(&first) = config.allowed_users.first() {
        if first != 0 {
            tracing::info!(
                "Telegram: no last_chat_id, falling back to allowed_users[0]={}",
                first
            );
            return Some(first);
        }
    }
    tracing::warn!("Telegram: no chat_id available — user must send a message to the bot first");
    None
}

/// Send a photo (screenshot) with optional caption to the last active Telegram chat
pub async fn send_photo(agent: &Agent, image_bytes: &[u8], caption: &str) -> Result<()> {
    let Some(config) = &agent.config.telegram else {
        return Ok(());
    };
    let Some(chat_id) = resolve_chat_id(agent, config).await else {
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

/// Send a video with optional caption to the last active Telegram chat
pub async fn send_video(agent: &Agent, video_bytes: &[u8], caption: &str) -> Result<()> {
    let Some(config) = &agent.config.telegram else {
        return Ok(());
    };
    let Some(chat_id) = resolve_chat_id(agent, config).await else {
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
                "Welcome to {}! 🤖\n\n\
                📸 Media:\n\
                /image <prompt> - Generate image\n\
                /video <prompt> - Generate video\n\n\
                ⏰ Productivity:\n\
                /remind <time> <msg> - Set reminder\n\
                /todo - View todo list\n\
                /todo add <item> - Add todo\n\
                /note <text> - Save a note\n\
                /tasks - View pending tasks\n\n\
                🔍 Utilities:\n\
                /weather [location] - Get weather\n\
                /translate <text> - Translate\n\
                /search <query> - Web search\n\
                /summarize - Summarize chat\n\n\
                ⚙️ Settings:\n\
                /status - Agent status\n\
                /skills - List skills\n\
                /memory - Memory stats\n\
                /model <name> - Switch model\n\
                /settings - View settings\n\
                /install <url> - Install a skill from URL\n\
                /tunnel [start|stop|status] - Manage public UI tunnel\n\
                /run <skill> [query] - Run a custom/bundled skill\n\
                /setsecret KEY=VALUE - Store a secret encrypted (private + allowlisted only)\n\
                /clear - Clear conversation history\n\n\
                Or just chat with me!",
                agent.config.name
            )
        }

        "/status" => {
            let agent = agent.read().await;
            let status = agent.status().await;
            format!(
                "📊 Agent Status\n\n\
                🆔 DID: {}\n\
                🧠 Memory: {} entries\n\
                🛠 Skills: {} loaded\n\
                📋 Tasks: {} pending",
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
                "⚙️ Current Settings\n\n\
                🤖 Bot: {}\n\
                💬 Personality: {}\n\
                🧠 Model: {}\n\
                🔄 Fallback: {}",
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
                    .map(|s| format!("• {} - {}", s.name, s.description))
                    .collect::<Vec<_>>()
                    .join("\n");
                let more = if actions.len() > 15 {
                    format!("\n\n...and {} more", actions.len() - 15)
                } else {
                    String::new()
                };
                format!("🛠 Available Skills:\n\n{}{}", list, more)
            }
        }

        "/memory" => {
            let agent = agent.read().await;
            let status = agent.status().await;
            format!(
                "🧠 Memory Stats\n\n\
                📝 Entries: {}\n\
                🛠 Skills: {}\n\
                📋 Tasks: {}",
                status.memory_entries, status.actions_loaded, status.tasks_pending
            )
        }

        "/image" => {
            if args.is_empty() {
                "Usage: /image <prompt>\n\nExample: /image a cute robot playing guitar".to_string()
            } else {
                // Process through agent with image generation intent
                let response = {
                    let agent = agent.read().await;
                    let prompt = format!("Generate an image of: {}", args);
                    match agent
                        .process_message(&prompt, "telegram", Some(&conversation_id), None)
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => format!("❌ Error: {}", e),
                    }
                };
                response
            }
        }

        "/video" => {
            if args.is_empty() {
                "Usage: /video <prompt>\n\nExample: /video a rocket launching into space"
                    .to_string()
            } else {
                let response = {
                    let agent = agent.read().await;
                    let prompt = format!("Generate a video of: {}", args);
                    match agent
                        .process_message(&prompt, "telegram", Some(&conversation_id), None)
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => format!("❌ Error: {}", e),
                    }
                };
                response
            }
        }

        "/remind" => {
            if args.is_empty() {
                "Usage: /remind <time> <message>\n\nExamples:\n/remind 5m Check the oven\n/remind 2h Call mom\n/remind tomorrow 9am Meeting".to_string()
            } else {
                let response = {
                    let agent = agent.read().await;
                    let prompt = format!("Set a reminder: {}", args);
                    match agent
                        .process_message(&prompt, "telegram", Some(&conversation_id), None)
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => format!("❌ Error: {}", e),
                    }
                };
                response
            }
        }

        "/weather" => {
            let location = if args.is_empty() { "my location" } else { args };
            let response = {
                let agent = agent.read().await;
                let prompt = format!("What's the weather in {}?", location);
                match agent
                    .process_message(&prompt, "telegram", Some(&conversation_id), None)
                    .await
                {
                    Ok(r) => r,
                    Err(e) => format!("❌ Error: {}", e),
                }
            };
            response
        }

        "/translate" => {
            if args.is_empty() {
                "Usage: /translate <text>\n\nExample: /translate Hello, how are you? to Spanish"
                    .to_string()
            } else {
                let response = {
                    let agent = agent.read().await;
                    let prompt = format!("Translate: {}", args);
                    match agent
                        .process_message(&prompt, "telegram", Some(&conversation_id), None)
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => format!("❌ Error: {}", e),
                    }
                };
                response
            }
        }

        "/search" => {
            if args.is_empty() {
                "Usage: /search <query>\n\nExample: /search latest news about AI".to_string()
            } else {
                let response = {
                    let agent = agent.read().await;
                    let prompt = format!("Search the web for: {}", args);
                    match agent
                        .process_message(&prompt, "telegram", Some(&conversation_id), None)
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => format!("❌ Error: {}", e),
                    }
                };
                response
            }
        }

        "/install" => {
            if args.is_empty() {
                "Usage: /install <skill_url>".to_string()
            } else {
                let prompt = format!("install this skill {}", args.trim());
                let agent = agent.read().await;
                match agent
                    .process_message(&prompt, "telegram", Some(&conversation_id), None)
                    .await
                {
                    Ok(r) => r,
                    Err(e) => format!("❌ Error: {}", e),
                }
            }
        }

        "/summarize" => {
            let response = {
                let agent = agent.read().await;
                match agent
                    .process_message(
                        "Summarize our recent conversation",
                        "telegram",
                        Some(&conversation_id),
                        None,
                    )
                    .await
                {
                    Ok(r) => r,
                    Err(e) => format!("❌ Error: {}", e),
                }
            };
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
                        Err(e) => format!("❌ Error: {}", e),
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
                        Err(e) => format!("❌ Error: {}", e),
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
                        Err(e) => format!("❌ Error: {}", e),
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
                "📋 No pending tasks".to_string()
            } else {
                let list = pending
                    .iter()
                    .map(|t| {
                        let status = match t.status {
                            TaskStatus::AwaitingApproval => "⏳",
                            TaskStatus::Pending => "📌",
                            _ => "•",
                        };
                        format!("{} {}", status, t.description)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("📋 Pending Tasks:\n\n{}", list)
            }
        }

        "/clear" => {
            let agent = agent.read().await;
            agent
                .clear_conversation_by_id("telegram", &conversation_id, None)
                .await;
            "🧹 Conversation cleared! Starting fresh.".to_string()
        }

        "/model" => {
            if args.is_empty() {
                let agent = agent.read().await;
                let current = match &agent.config.llm {
                    crate::core::LlmProvider::Ollama { model, .. } => model.clone(),
                    crate::core::LlmProvider::Anthropic { model, .. } => model.clone(),
                    crate::core::LlmProvider::OpenAI { model, .. } => model.clone(),
                };
                format!("Current model: {}\n\nUsage: /model <model_name>\n\nNote: Changing models requires restart", current)
            } else {
                format!("Model change to '{}' noted.\n\n⚠️ To apply, please update via web UI settings and restart.", args)
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
                    Err(e) => format!("❌ Error: {}", e),
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
                    Ok(_) => format!("✅ Task created: {}", description),
                    Err(e) => format!("❌ Failed to create task: {}", e),
                }
            }
        }

        _ => format!(
            "Unknown command: {}\n\nType /help for all commands",
            command
        ),
    }
}
