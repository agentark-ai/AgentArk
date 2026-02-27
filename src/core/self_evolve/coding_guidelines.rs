//! Baked-in coding guidelines for the self-evolve inner agent.
//!
//! These guidelines teach the LLM how to write code that conforms
//! to AgentArk's conventions, patterns, and security requirements.

/// Returns the full AgentArk coding guidelines as a static string.
/// Injected into the inner agent's system prompt so generated code
/// follows project conventions exactly.
pub fn coding_guidelines() -> &'static str {
    r#"# AgentArk Coding Guidelines

You are modifying AgentArk's own codebase. Follow these conventions EXACTLY.

## Project Structure

```
src/
  core/           # Agent brain: config, LLM client, tool dispatch, routing
    agent.rs      # Main Agent struct, process_message_internal()
    agent/
      tool_execution.rs   # Tool call handlers (handle_*_tool_call methods)
      prompt_builder.rs   # System prompt construction
      routing.rs          # Task complexity routing
    tool_handlers.rs      # ToolHandler trait + handler structs + default_tool_handlers()
    config.rs             # AgentConfig, SecureConfigManager
    llm.rs                # LlmClient, LlmProvider, ToolCall, LlmResponse
    mod.rs                # Core module declarations
  integrations/   # External service connectors (GitHub, Notion, Twilio, etc.)
    mod.rs        # Integration trait, Capability enum, IntegrationManager
    github.rs     # Example: GitHubConnector implements Integration
  runtime/        # Action execution, WASM/Docker sandbox, skill loading
    mod.rs        # ActionRuntime, ActionDef, load_builtin_actions()
    sandbox.rs    # ActionSandbox (WASM engine)
  actions/        # Built-in action implementations (ssh, app deploy, video, etc.)
  security/       # ActionGuard, safety rules, threat detection
  channels/       # Telegram, WhatsApp, email message delivery
  sentinel.rs     # Background scheduler for cron tasks and watchers
frontend/
  src/
    App.tsx               # Main app with sidebar navigation
    api/client.ts         # API client (fetch + SSE streaming)
    types.ts              # Shared TypeScript types
    components/           # React components (IntegrationsPanel, NativeWorkspace, etc.)
    store/uiStore.ts      # Zustand state management
    theme.ts              # Material-UI theme
```

## Rust Conventions

- **Error handling**: Always use `anyhow::Result`. Propagate errors with `?`. Never use `.unwrap()` or `.expect()` in non-test code.
- **Async**: Everything is `tokio` async. Use `#[async_trait]` for async trait methods.
- **Shared state**: `Arc<RwLock<T>>` for shared mutable state across tasks.
- **Logging**: Use `tracing::{info, warn, debug, error}`. Add `tracing::info!()` for new code paths.
- **Safety**: NEVER use `unsafe` blocks. NEVER hardcode credentials.
- **Imports**: Group by `std`, then external crates, then `crate::` internal.
- **Naming**: snake_case for functions/variables, CamelCase for types/traits.

## Adding a New Integration

1. Create `src/integrations/{name}.rs` with a struct implementing the `Integration` trait:

```rust
use anyhow::Result;
use async_trait::async_trait;
use crate::integrations::{Capability, Integration, IntegrationStatus};

pub struct MyConnector {
    http: reqwest::Client,
    config_dir: std::path::PathBuf,
}

impl MyConnector {
    pub fn new_with_config_dir(config_dir: std::path::PathBuf) -> Self {
        Self {
            http: reqwest::Client::new(),
            config_dir,
        }
    }

    fn load_token(&self) -> Option<String> {
        // 1. Check env var
        if let Ok(t) = std::env::var("MY_API_KEY") {
            if !t.is_empty() { return Some(t); }
        }
        // 2. Encrypted config fallback
        crate::core::config::SecureConfigManager::new(&self.config_dir)
            .ok()
            .and_then(|m| m.get_custom_secret("my_api_key").ok().flatten())
    }
}

#[async_trait]
impl Integration for MyConnector {
    fn id(&self) -> &str { "my_service" }
    fn name(&self) -> &str { "My Service" }
    fn description(&self) -> &str { "Connect to My Service API" }
    fn icon(&self) -> &str { "icon_emoji" }
    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Read, Capability::Write]
    }
    async fn status(&self) -> IntegrationStatus {
        match self.load_token() {
            Some(_) => IntegrationStatus::Connected,
            None => IntegrationStatus::NotConfigured,
        }
    }
    async fn is_connected(&self) -> bool {
        matches!(self.status().await, IntegrationStatus::Connected)
    }
    async fn execute(&self, action: &str, params: &serde_json::Value) -> Result<serde_json::Value> {
        match action {
            "list_items" => { /* implementation */ Ok(serde_json::json!({})) }
            _ => Err(anyhow::anyhow!("Unknown action: {}", action)),
        }
    }
    async fn handle_webhook(&self, _payload: &serde_json::Value) -> Result<()> { Ok(()) }
}
```

2. Register in `src/integrations/mod.rs` → `register_default_integrations()`:
```rust
let my = my_service::MyConnector::new_with_config_dir(config_dir.clone());
self.integrations.insert("my_service".to_string(), Box::new(my));
```

3. Add `pub mod my_service;` to `src/integrations/mod.rs`.

## Adding a New Tool Handler

1. Add struct to `src/core/tool_handlers.rs`:
```rust
pub struct MyToolHandler;

#[async_trait]
impl ToolHandler for MyToolHandler {
    fn id(&self) -> &'static str { "my_tool" }
    fn can_handle(&self, _agent: &Agent, call: &ToolCall, _ctx: &ToolHandlerContext<'_>) -> bool {
        call.name == "my_tool"
    }
    async fn handle(&self, agent: &Agent, call: &ToolCall, ctx: &ToolHandlerContext<'_>) -> Result<Option<String>> {
        let out = agent.handle_my_tool_call(call, ctx.stream_tx).await?;
        Ok(Some(out))
    }
}
```

2. Register in `default_tool_handlers()` — BEFORE `RuntimeToolHandler` (the catch-all).
3. Add `"my_tool"` to `IntegrationToolHandler.can_handle()` exclusion list.
4. Implement `handle_my_tool_call()` in `src/core/agent/tool_execution.rs`.
5. Add tool description in `src/core/agent/prompt_builder.rs`.
6. Register `ActionDef` in `src/runtime/mod.rs` → `load_builtin_actions()`.

## Frontend Conventions

- **Components**: React function components with TypeScript.
- **UI library**: Material-UI (`@mui/material`). Use `Box`, `Typography`, `Button`, `TextField`, `Chip`, etc.
- **State**: Zustand store in `store/uiStore.ts`.
- **API calls**: Use functions from `api/client.ts`. Types in `types.ts`.
- **Styling**: Use MUI `sx` prop or `styled()`. Follow the existing dark theme.

## Security Rules (MANDATORY)

1. **No hardcoded credentials** — Use `SecureConfigManager` or env vars.
2. **Validate all inputs** — Check types, bounds, and formats before use.
3. **No shell injection** — Use `tokio::process::Command` with argument arrays, NEVER string interpolation for commands.
4. **Timeouts on HTTP** — Always set `.timeout(Duration::from_secs(30))` on `reqwest` calls.
5. **Error propagation** — Use `?` operator, never swallow errors silently.
6. **No unsafe** — Zero tolerance for `unsafe` blocks.
7. **Secret handling** — Use `{{secret:KEY_NAME}}` placeholder pattern resolved at runtime.

## Build & Verify Commands

After making changes, run these in order:
1. `cargo check` — Fast syntax/type validation
2. `cargo clippy -- -D warnings` — Lint (deny warnings)
3. `cargo test` — Unit tests
4. `cd frontend && npm run build` — Frontend bundle (if TS/TSX changed)
"#
}
