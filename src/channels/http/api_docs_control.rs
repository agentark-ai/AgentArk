use super::*;

pub(super) async fn docs_blocked_for_tunnel(state: &AppState, headers: &HeaderMap) -> bool {
    let tunnel_url = { state.tunnel.read().await.url.clone() };
    request_matches_active_tunnel(headers, tunnel_url.as_deref())
}

pub(super) async fn docs_is_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let expected_key = match auth::sync_http_api_key_state(state, false).await {
        Ok((info, _rotated)) => info.map(|k| k.key),
        Err(_) => return false,
    };
    let Some(expected_key) = expected_key else {
        return true;
    };

    if auth::has_valid_bearer_api_key(headers, Some(expected_key.as_str())) {
        return true;
    }

    if let Some(auth_value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(basic) = auth_value
            .strip_prefix("Basic ")
            .or_else(|| auth_value.strip_prefix("basic "))
        {
            if let Ok(decoded) = base64::engine::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                basic.trim(),
            ) {
                if let Ok(creds) = String::from_utf8(decoded) {
                    if let Some((username, password)) = creds.split_once(':') {
                        if crate::security::constant_time_eq(
                            password.as_bytes(),
                            expected_key.as_bytes(),
                        ) || crate::security::constant_time_eq(
                            username.as_bytes(),
                            expected_key.as_bytes(),
                        ) {
                            return true;
                        }
                    }
                }
            }
        }
    }

    if auth::has_valid_ui_session_cookie(state, headers).await {
        return true;
    }
    false
}

pub(super) fn docs_auth_required_response() -> Response {
    let mut response = (
        StatusCode::UNAUTHORIZED,
        Html(
            "Documentation is protected. Enter your API key as the password in the browser prompt.",
        ),
    )
        .into_response();
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static(crate::branding::DOCS_BASIC_AUTH_REALM),
    );
    response
}

pub(super) fn build_openapi_paths() -> serde_json::Map<String, serde_json::Value> {
    let mut paths = serde_json::Map::new();
    let mut add = |path: &str, method: &str, summary: &str, tag: &str| {
        let method_lc = method.to_ascii_lowercase();
        let entry = paths
            .entry(path.to_string())
            .or_insert_with(|| serde_json::json!({}));
        if let Some(obj) = entry.as_object_mut() {
            obj.insert(
                method_lc,
                serde_json::json!({
                    "tags": [tag],
                    "summary": summary,
                    "responses": {
                        "200": { "description": "OK" }
                    }
                }),
            );
        }
    };

    // --- Chat & Status ---
    add("/status", "GET", "Agent status", "Status");
    add("/health", "GET", "Health check", "Status");
    add("/readiness", "GET", "Readiness check", "Status");
    add("/chat", "POST", "Chat completion", "Chat");
    add("/chat/stream", "POST", "Streaming chat completion", "Chat");
    add("/chat/clear", "POST", "Clear current chat context", "Chat");
    add(
        "/chat/credential-prompt",
        "GET",
        "Get pending chat credential prompt",
        "Chat",
    );
    add(
        "/gateway/channels",
        "GET",
        "List channel gateway status",
        "Gateway",
    );
    add(
        "/gateway/ops",
        "GET",
        "Get gateway operational overview",
        "Gateway",
    );
    add(
        "/gateway/routing",
        "GET",
        "Get gateway routing status",
        "Gateway",
    );
    add(
        "/trace",
        "GET",
        "Get latest execution trace summary",
        "Trace",
    );
    add("/trace/{id}", "GET", "Get execution trace detail", "Trace");
    add("/runs/{id}", "GET", "Get live run detail", "Trace");
    add(
        "/runs/{id}/stream",
        "GET",
        "Stream live run events",
        "Trace",
    );

    // --- Skills ---
    add("/skills", "GET", "List skills", "Skills");
    add("/skills", "POST", "Create skill", "Skills");
    add("/skills/{name}", "GET", "Get skill content", "Skills");
    add("/skills/{name}", "POST", "Update skill content", "Skills");
    add("/skills/{name}", "DELETE", "Delete skill", "Skills");
    add(
        "/skills/{name}/enabled",
        "POST",
        "Enable/disable skill",
        "Skills",
    );
    add(
        "/skills/{name}/secrets",
        "GET",
        "Get skill secrets",
        "Skills",
    );
    add(
        "/skills/{name}/secrets",
        "POST",
        "Set skill secrets",
        "Skills",
    );
    add("/skills/{name}/test", "POST", "Test skill", "Skills");
    add(
        "/skills/test-runs/{run_id}/cancel",
        "POST",
        "Cancel active skill test",
        "Skills",
    );
    add(
        "/skills/import",
        "POST",
        "Import skill(s) from URL",
        "Skills",
    );

    // --- Tasks ---
    add("/tasks", "GET", "List tasks", "Tasks");
    add("/tasks", "POST", "Create task", "Tasks");
    add("/tasks/plan", "POST", "Plan task", "Tasks");
    add("/tasks/{id}", "POST", "Update task", "Tasks");
    add("/tasks/{id}", "DELETE", "Delete task", "Tasks");
    add(
        "/tasks/{id}/resume-chat/stream",
        "POST",
        "Resume cancelled or failed web chat task in chat",
        "Tasks",
    );
    add("/tasks/{id}/retry", "POST", "Retry failed task", "Tasks");
    add("/tasks/{id}/approve", "POST", "Approve task", "Tasks");
    add("/tasks/{id}/reject", "POST", "Reject task", "Tasks");
    add(
        "/background-sessions",
        "GET",
        "List background sessions",
        "Automation",
    );
    add(
        "/background-sessions",
        "POST",
        "Create background session",
        "Automation",
    );
    add(
        "/background-sessions/{id}",
        "GET",
        "Get background session detail",
        "Automation",
    );
    add(
        "/background-sessions/{id}",
        "POST",
        "Update background session",
        "Automation",
    );
    add(
        "/background-sessions/{id}",
        "DELETE",
        "Delete background session",
        "Automation",
    );
    add(
        "/background-sessions/{id}/attach",
        "POST",
        "Attach tasks or watchers to a background session",
        "Automation",
    );
    add(
        "/background-sessions/{id}/detach",
        "POST",
        "Detach tasks or watchers from a background session",
        "Automation",
    );
    add(
        "/background-sessions/{id}/pause",
        "POST",
        "Pause background session work",
        "Automation",
    );
    add(
        "/background-sessions/{id}/resume",
        "POST",
        "Resume background session work",
        "Automation",
    );
    add(
        "/background-sessions/{id}/cancel",
        "POST",
        "Stop background session work",
        "Automation",
    );
    add(
        "/automation/objects",
        "GET",
        "Unified automation inventory",
        "Automation",
    );
    add(
        "/automation/runs",
        "GET",
        "Recent automation run history",
        "Automation",
    );
    add("/watchers", "GET", "List watchers", "Watchers");

    // --- Goals ---
    add("/goals", "GET", "List goals", "Goals");
    add("/goals", "POST", "Create goal", "Goals");
    add("/goals/{id}", "DELETE", "Delete goal", "Goals");

    // --- Autonomy ---
    add(
        "/autonomy/settings",
        "GET",
        "Get autonomy settings",
        "Autonomy",
    );
    add(
        "/autonomy/settings",
        "POST",
        "Update autonomy settings",
        "Autonomy",
    );
    add(
        "/autonomy/briefing",
        "GET",
        "Get autonomy briefing",
        "Autonomy",
    );
    add(
        "/autonomy/suggestions/{id}",
        "GET",
        "Get a suggested automation detail",
        "Autonomy",
    );
    add(
        "/autonomy/suggestions/{id}/accept",
        "POST",
        "Accept a suggested automation draft",
        "Autonomy",
    );
    add(
        "/autonomy/suggestions/{id}/dismiss",
        "POST",
        "Dismiss a suggested automation draft",
        "Autonomy",
    );
    add(
        "/autonomy/incidents/live",
        "GET",
        "List live incidents",
        "Autonomy",
    );
    add(
        "/autonomy/timeline",
        "GET",
        "Get autonomy timeline",
        "Autonomy",
    );
    add(
        "/autonomy/timeline/rollback",
        "POST",
        "Rollback timeline event",
        "Autonomy",
    );
    // --- Settings & Models ---
    add("/settings", "GET", "Get settings", "Settings");
    add("/settings", "POST", "Update settings", "Settings");
    add("/profile", "GET", "Get onboarding profile", "Settings");
    add(
        "/profile/onboarding",
        "POST",
        "Save first-run personalization answers",
        "Settings",
    );
    add(
        "/profile/onboarding/dismiss",
        "POST",
        "Dismiss first-run personalization prompt",
        "Settings",
    );
    add(
        "/settings/google-workspace/oauth-client",
        "GET",
        "Get global Google OAuth client settings",
        "Settings",
    );
    add(
        "/settings/google-workspace/oauth-client",
        "POST",
        "Update global Google OAuth client settings",
        "Settings",
    );
    add(
        "/settings/observability/logs",
        "GET",
        "List observability export delivery logs",
        "Settings",
    );
    add(
        "/settings/observability/test",
        "POST",
        "Send a test observability trace",
        "Settings",
    );
    add(
        "/settings/evolution",
        "GET",
        "Get evolution control center status",
        "Settings",
    );
    add(
        "/settings/evolution",
        "POST",
        "Update evolution minimal settings",
        "Settings",
    );
    add(
        "/settings/evolution/dev",
        "GET",
        "Get evolution developer metrics",
        "Settings",
    );
    add(
        "/settings/evolution/dev/action",
        "POST",
        "Run evolution developer action",
        "Settings",
    );
    add(
        "/settings/api-key",
        "GET",
        "Get API key metadata",
        "Settings",
    );
    add(
        "/settings/api-key/regenerate",
        "POST",
        "Regenerate API key",
        "Settings",
    );
    add("/models", "GET", "List models", "Models");
    add("/models", "POST", "Add model", "Models");
    add("/models/test", "POST", "Test model connection", "Models");
    add("/models/{id}", "PUT", "Update model", "Models");
    add("/models/{id}", "DELETE", "Delete model", "Models");
    add(
        "/models/discover/{provider}",
        "GET",
        "Discover available models for a provider",
        "Models",
    );
    add(
        "/models/openai-subscription/oauth/start",
        "POST",
        "Start OpenAI Subscription browser OAuth",
        "Models",
    );
    add(
        "/models/openai-subscription/oauth/status",
        "GET",
        "Check OpenAI Subscription OAuth status",
        "Models",
    );
    add(
        "/analytics/llm",
        "GET",
        "Get LLM usage, token, cost, model, channel, and purpose analytics",
        "Analytics",
    );
    add(
        "/reflect",
        "GET",
        "Get cached ArkReflect clusters and daily digest status across chat, ArkOrbit, apps, goals, watchers, Sentinel, ArkPulse, ArkEvolve, usage, memory, and workflows",
        "Analytics",
    );
    add(
        "/reflect/refresh",
        "POST",
        "Queue a guarded ArkReflect background refresh for a selected time range",
        "Analytics",
    );

    // --- Integrations ---
    add("/integrations", "GET", "List integrations", "Integrations");
    add(
        "/gmail/status",
        "GET",
        "Get Gmail integration status",
        "Integrations",
    );
    add(
        "/gmail/test",
        "GET",
        "Test Gmail integration",
        "Integrations",
    );
    add(
        "/calendar/status",
        "GET",
        "Get calendar integration status",
        "Integrations",
    );
    add(
        "/calendar/test",
        "GET",
        "Test calendar integration",
        "Integrations",
    );
    add(
        "/integrations/{id}/auth",
        "GET",
        "Integration auth URL",
        "Integrations",
    );
    add(
        "/integrations/{id}/configure",
        "POST",
        "Configure integration",
        "Integrations",
    );
    add(
        "/integrations/{id}/enable",
        "POST",
        "Enable integration",
        "Integrations",
    );
    add(
        "/integrations/{id}/disable",
        "POST",
        "Disable integration",
        "Integrations",
    );
    add(
        "/integrations/{id}/test",
        "POST",
        "Test integration",
        "Integrations",
    );
    add(
        "/integrations/{id}/disconnect",
        "POST",
        "Disconnect integration",
        "Integrations",
    );

    // --- Plugin SDK ---
    add("/plugins", "GET", "List plugin SDK integrations", "Plugins");
    add(
        "/plugins",
        "POST",
        "Install plugin SDK integration",
        "Plugins",
    );
    add(
        "/plugins/logs",
        "GET",
        "List plugin SDK delivery logs",
        "Plugins",
    );
    add("/hooks", "GET", "List hooks", "Hooks");
    add("/hooks/runs", "GET", "List hook runs", "Hooks");
    add(
        "/webhooks/events",
        "GET",
        "List received webhook events",
        "Hooks",
    );
    add("/ssh/connections", "GET", "List SSH connections", "SSH");
    add("/ssh/keys", "GET", "List SSH keys", "SSH");
    add(
        "/plugins/{id}",
        "PUT",
        "Update plugin SDK integration",
        "Plugins",
    );
    add(
        "/plugins/{id}",
        "DELETE",
        "Delete plugin SDK integration",
        "Plugins",
    );
    add(
        "/plugins/{id}/refresh",
        "POST",
        "Refresh plugin manifest",
        "Plugins",
    );
    add("/plugins/{id}/test", "POST", "Ping plugin", "Plugins");
    add(
        "/sender-verification",
        "GET",
        "List sender verification policies and approval state",
        "Security",
    );
    add(
        "/sender-verification/settings",
        "POST",
        "Update sender verification policies",
        "Security",
    );
    add(
        "/sender-verification/approve",
        "POST",
        "Approve a pending sender",
        "Security",
    );
    add(
        "/sender-verification/revoke",
        "POST",
        "Revoke an approved sender",
        "Security",
    );

    // --- Documents ---
    add("/documents", "GET", "List documents", "Documents");
    add("/documents/upload", "POST", "Upload document", "Documents");
    add(
        "/documents/upload-file",
        "POST",
        "Upload file document",
        "Documents",
    );
    add("/documents/{id}", "DELETE", "Delete document", "Documents");
    add(
        "/documents/{id}/search",
        "GET",
        "Search document",
        "Documents",
    );

    // --- Memory ---
    add(
        "/memory/stats",
        "GET",
        "Memory statistics by domain",
        "Memory",
    );
    add("/memory/facts", "GET", "List learned facts", "Memory");
    add(
        "/channels/available",
        "GET",
        "List messaging channels from every registry source (bundled + custom + extension packs)",
        "Channels",
    );
    add(
        "/custom-messaging-channels",
        "GET",
        "List user-added custom messaging channels",
        "Channels",
    );
    add(
        "/custom-messaging-channels",
        "POST",
        "Create a user-added custom messaging channel",
        "Channels",
    );
    add(
        "/custom-messaging-channels/{id}",
        "PUT",
        "Update a user-added custom messaging channel",
        "Channels",
    );
    add(
        "/custom-messaging-channels/{id}",
        "DELETE",
        "Delete a user-added custom messaging channel and its stored credentials",
        "Channels",
    );
    add(
        "/custom-messaging-channels/{id}/credentials",
        "POST",
        "Store encrypted credentials for a custom messaging channel",
        "Channels",
    );
    add(
        "/custom-messaging-channels/{id}/test",
        "POST",
        "Send a test notification through a custom messaging channel",
        "Channels",
    );
    add(
        "/memory/preferences",
        "GET",
        "List user preferences",
        "Memory",
    );
    add(
        "/memory/preferences",
        "POST",
        "Create or update user preference",
        "Memory",
    );
    add(
        "/memory/preferences/{key}",
        "DELETE",
        "Delete user preference",
        "Memory",
    );
    add("/memory/user-data", "GET", "List user data items", "Memory");
    add(
        "/memory/user-data",
        "POST",
        "Create user data item",
        "Memory",
    );
    add(
        "/memory/user-data/{id}",
        "DELETE",
        "Delete user data item",
        "Memory",
    );
    add(
        "/memory/knowledge",
        "GET",
        "List knowledge base items",
        "Memory",
    );
    add(
        "/memory/knowledge",
        "POST",
        "Create knowledge base item",
        "Memory",
    );
    add(
        "/memory/knowledge/sync-product-docs",
        "POST",
        &format!(
            "Sync bundled {} product-help knowledge",
            crate::branding::PRODUCT_NAME
        ),
        "Memory",
    );
    add(
        "/memory/knowledge/{id}",
        "DELETE",
        "Delete knowledge base item",
        "Memory",
    );
    for (path, method, description) in [
        ("/arkmemory/summary", "GET", "ArkMemory operations summary"),
        ("/arkmemory/queue", "GET", "List staged memory candidates"),
        (
            "/arkmemory/queue/{id}/approve",
            "POST",
            "Approve a staged memory candidate",
        ),
        (
            "/arkmemory/queue/{id}/reject",
            "POST",
            "Reject a staged memory candidate",
        ),
        ("/arkmemory/ledger", "GET", "List memory ledger events"),
        (
            "/arkmemory/ledger/{id}/rollback",
            "POST",
            "Rollback a reversible memory ledger event",
        ),
        ("/arkmemory/health", "GET", "List memory health findings"),
        (
            "/arkmemory/health/{id}/apply",
            "POST",
            "Apply or acknowledge a memory health finding",
        ),
        (
            "/arkmemory/sources/{memory_id}",
            "GET",
            "Show memory source and provenance records",
        ),
        ("/arkmemory/tests", "GET", "List memory checks"),
        ("/arkmemory/tests/run", "POST", "Refresh memory checks"),
        ("/arkmemory/cleanup", "GET", "List memory cleanup findings"),
        (
            "/arkmemory/cleanup/apply",
            "POST",
            "Apply or acknowledge memory cleanup findings",
        ),
    ] {
        add(path, method, description, "ArkMemory");
    }

    // --- Notifications ---
    add(
        "/notifications",
        "GET",
        "List notifications",
        "Notifications",
    );
    add(
        "/notifications/count",
        "GET",
        "Notification count",
        "Notifications",
    );
    add(
        "/notifications/stream",
        "GET",
        "Live notification stream (SSE)",
        "Notifications",
    );
    add(
        "/notifications/read-all",
        "POST",
        "Mark all notifications read",
        "Notifications",
    );
    add(
        "/notifications/{id}/read",
        "POST",
        "Mark notification read",
        "Notifications",
    );

    // --- Conversations ---
    add(
        "/conversations",
        "GET",
        "List conversations",
        "Conversations",
    );
    add(
        "/conversations",
        "POST",
        "Create conversation",
        "Conversations",
    );
    add(
        "/conversations/{id}",
        "GET",
        "Get conversation",
        "Conversations",
    );
    add(
        "/conversations/{id}",
        "PATCH",
        "Update conversation",
        "Conversations",
    );
    add(
        "/conversations/{id}",
        "DELETE",
        "Delete conversation",
        "Conversations",
    );
    add(
        "/conversations/{id}/messages",
        "GET",
        "List conversation messages",
        "Conversations",
    );
    add(
        "/conversations/{id}/latest-run",
        "GET",
        "Get latest run for a conversation",
        "Conversations",
    );

    // --- MCP ---
    add("/mcp", "POST", "MCP request", "MCP");
    add("/mcp/tools", "GET", "List MCP tools", "MCP");
    add("/mcp/servers", "GET", "List MCP servers", "MCP");
    add("/mcp/servers", "POST", "Create MCP server", "MCP");
    add("/mcp/servers/{id}", "GET", "Get MCP server", "MCP");
    add("/mcp/servers/{id}", "PUT", "Update MCP server", "MCP");
    add("/mcp/servers/{id}", "DELETE", "Delete MCP server", "MCP");
    add(
        "/mcp/servers/{id}/refresh",
        "POST",
        "Refresh MCP server",
        "MCP",
    );

    // --- Security ---
    add("/security/status", "GET", "Security status", "Security");
    add("/security/logs", "GET", "Security logs", "Security");
    add(
        "/security/abuse-reviews",
        "GET",
        "List paused or pending abuse-review sources",
        "Security",
    );
    add(
        "/security/abuse-reviews/{source_key_hash}/approve",
        "POST",
        "Resume an abuse-review source",
        "Security",
    );
    add(
        "/security/abuse-reviews/{source_key_hash}/reject",
        "POST",
        "Pause an abuse-review source",
        "Security",
    );
    add(
        "/security/internal-service-tokens/rotate",
        "POST",
        "Rotate internal executor and workspace credentials",
        "Security",
    );
    add(
        "/security/set-password",
        "POST",
        "Set master password",
        "Security",
    );
    add(
        "/security/change-password",
        "POST",
        "Change master password",
        "Security",
    );
    add(
        "/security/remove-password",
        "POST",
        "Remove master password",
        "Security",
    );

    // --- Tunnel ---
    add("/tunnel/status", "GET", "Tunnel status", "Tunnel");
    add(
        "/tunnel/providers",
        "GET",
        "List tunnel providers",
        "Tunnel",
    );
    add(
        "/tunnel/configure",
        "POST",
        "Save tunnel provider settings",
        "Tunnel",
    );
    add(
        "/tunnel/test",
        "POST",
        "Test selected tunnel provider",
        "Tunnel",
    );
    add("/tunnel/start", "POST", "Start tunnel", "Tunnel");
    add("/tunnel/stop", "POST", "Stop tunnel", "Tunnel");

    // --- Swarm ---
    add("/swarm/status", "GET", "Swarm status", "Swarm");
    add("/swarm/agents", "GET", "List swarm agents", "Swarm");
    add(
        "/swarm/agents/builder/options",
        "GET",
        "List attachable swarm agent resources",
        "Swarm",
    );
    add(
        "/swarm/agents/access-plan",
        "POST",
        "Plan elevated access for a drafted swarm agent",
        "Swarm",
    );
    add(
        "/swarm/agents/draft",
        "POST",
        "Generate a swarm agent draft from a description",
        "Swarm",
    );
    add("/swarm/agents", "POST", "Add swarm agent", "Swarm");
    add("/swarm/agents/{id}", "POST", "Update swarm agent", "Swarm");
    add(
        "/swarm/agents/{id}",
        "DELETE",
        "Remove swarm agent",
        "Swarm",
    );
    add("/swarm/config", "GET", "Get swarm config", "Swarm");
    add("/swarm/config", "POST", "Update swarm config", "Swarm");
    add(
        "/swarm/delegations",
        "GET",
        "List swarm delegations",
        "Swarm",
    );

    // --- Apps ---
    add("/api/apps", "GET", "List deployed apps", "Apps");
    add(
        "/api/uploads/{upload_id}",
        "GET",
        "Get uploaded chat file",
        "Apps",
    );
    add("/api/apps/{app_id}/stop", "POST", "Stop app", "Apps");
    add("/api/apps/{app_id}/restart", "POST", "Restart app", "Apps");
    add("/api/apps/{app_id}", "DELETE", "Delete app", "Apps");
    add(
        "/api/applications",
        "GET",
        "List built-in Ollama application launchers",
        "Apps",
    );
    add(
        "/api/applications/{app_id}/launch",
        "POST",
        "Launch a built-in application through Ollama Launch",
        "Apps",
    );
    add(
        "/api/applications/{app_id}/stop",
        "POST",
        "Stop a running built-in application launch",
        "Apps",
    );
    add(
        "/api/arkorbit/orbits",
        "GET",
        "List ArkOrbit workspaces",
        "ArkOrbit",
    );
    add(
        "/api/arkorbit/orbits/{id}",
        "GET",
        "Get ArkOrbit workspace",
        "ArkOrbit",
    );
    add(
        "/api/arkorbit/orbits/{id}/index",
        "GET",
        "Get ArkOrbit workspace index",
        "ArkOrbit",
    );
    add(
        "/api/arkorbit/orbits/{id}/messages",
        "GET",
        "List ArkOrbit workspace messages",
        "ArkOrbit",
    );
    add(
        "/api/arkorbit/orbits/{id}/files",
        "GET",
        "List ArkOrbit workspace files",
        "ArkOrbit",
    );
    add(
        "/api/arkorbit/orbits/{id}/files/{*path}",
        "GET",
        "Get ArkOrbit workspace file",
        "ArkOrbit",
    );
    add(
        "/browser/sessions",
        "GET",
        "List browser sessions",
        "Browser",
    );
    add(
        "/browser/sessions/{id}",
        "GET",
        "Get browser session detail",
        "Browser",
    );
    add(
        "/browser/sessions/{id}/status",
        "GET",
        "Get browser session status",
        "Browser",
    );
    add(
        "/api/whatsapp-bridge/status",
        "GET",
        "Get WhatsApp bridge status",
        "Channels",
    );
    add(
        "/api/telegram/status",
        "GET",
        "Get Telegram channel status",
        "Channels",
    );

    if let Some(operation) = paths
        .get_mut("/analytics/llm")
        .and_then(|path| path.get_mut("get"))
        .and_then(|operation| operation.as_object_mut())
    {
        operation.insert(
            "parameters".to_string(),
            serde_json::json!([
                {
                    "name": "range",
                    "in": "query",
                    "description": "Relative time window such as 24h, 7d, 30d, or all.",
                    "schema": { "type": "string" }
                },
                {
                    "name": "bucket",
                    "in": "query",
                    "description": "Aggregation bucket for time series.",
                    "schema": { "type": "string", "enum": ["hour", "day", "week"] }
                },
                {
                    "name": "from",
                    "in": "query",
                    "description": "Optional inclusive start timestamp.",
                    "schema": { "type": "string" }
                },
                {
                    "name": "to",
                    "in": "query",
                    "description": "Optional exclusive end timestamp.",
                    "schema": { "type": "string" }
                }
            ]),
        );
    }

    paths
}

pub(super) async fn openapi_spec(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if docs_blocked_for_tunnel(&state, &headers).await {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !docs_is_authorized(&state, &headers).await {
        return docs_auth_required_response();
    }

    let security = serde_json::json!([
        { "BearerAuth": [] }
    ]);

    let spec = serde_json::json!({
        "openapi": "3.0.3",
        "info": {
            "title": format!("{} API", crate::branding::PRODUCT_NAME),
            "version": "1.0.0",
            "description": format!(
                "Interactive API reference for {}. Endpoints listed here require API key authentication.",
                crate::branding::PRODUCT_NAME
            )
        },
        "servers": [
            { "url": "/" }
        ],
        "tags": [
            { "name": "Status", "description": "Agent health and status" },
            { "name": "Chat", "description": "Send messages and stream responses" },
            { "name": "Gateway", "description": "Messaging gateway status and routing" },
            { "name": "Skills", "description": "Manage agent skills and actions" },
            { "name": "Tasks", "description": "Create, schedule, and manage tasks" },
            { "name": "Watchers", "description": "Background watcher inventory" },
            { "name": "Goals", "description": "Long-term goal tracking" },
            { "name": "Autonomy", "description": "Autonomous operation settings, briefings, and incidents" },
            { "name": "Settings", "description": "Application settings and API keys" },
            { "name": "Models", "description": "LLM model configuration" },
            { "name": "Analytics", "description": "Usage, cost, and operational analytics" },
            { "name": "Integrations", "description": "Third-party service connections" },
            { "name": "Hooks", "description": "Hooks and webhook event history" },
            { "name": "SSH", "description": "SSH connection inventory" },
            { "name": "Documents", "description": "Document storage and semantic search" },
            { "name": "Memory", "description": "Learned facts, user preferences, user data, and knowledge base" },
            { "name": "Notifications", "description": "Notification inbox and read status" },
            { "name": "Conversations", "description": "Conversation history and messages" },
            { "name": "MCP", "description": "Model Context Protocol servers and tools" },
            { "name": "Security", "description": "Security logs and master password" },
            { "name": "Tunnel", "description": "Remote access providers and status" },
            { "name": "Swarm", "description": "Multi-agent swarm coordination" },
            { "name": "Apps", "description": "Deployed app management" },
            { "name": "ArkOrbit", "description": "ArkOrbit workspace inventory" },
            { "name": "Browser", "description": "Interactive browser session inventory" },
            { "name": "Channels", "description": "Messaging channel status" },
            { "name": "Trace", "description": "Execution trace and run inspection" }
        ],
        "paths": build_openapi_paths(),
        "components": {
            "securitySchemes": {
                "BearerAuth": {
                    "type": "http",
                    "scheme": "bearer",
                    "bearerFormat": "API Key"
                }
            }
        },
        "security": security
    });
    (StatusCode::OK, Json(spec)).into_response()
}

pub(super) async fn api_docs_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if docs_blocked_for_tunnel(&state, &headers).await {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !docs_is_authorized(&state, &headers).await {
        return docs_auth_required_response();
    }

    let html = r#"<!doctype html>
<html>
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>__PRODUCT_NAME__ API Docs</title>
  <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css" />
  <style>
 /* - __PRODUCT_NAME__ theme - exact match to app palette - */
    /* bg.default=#030711  bg.paper=#091527  primary=#2fd4ff  secondary=#14f195
       text.primary=#ecf5ff  text.secondary=#9bb4d6  border=rgba(106,150,198,0.22)
       card=linear-gradient(140deg,rgba(9,21,39,0.92),rgba(9,21,39,0.72))
       font='Space Grotesk','IBM Plex Sans','Segoe UI',sans-serif */

    body {
      margin: 0;
      background: #030711;
      color: #ecf5ff;
      font-family: 'Space Grotesk', 'IBM Plex Sans', 'Segoe UI', sans-serif;
    }
    .swagger-ui,
    .swagger-ui .wrapper { background: #030711; font-family: inherit; }
    .swagger-ui .topbar { display: none; }

    /* Info */
    .swagger-ui .info .title,
    .swagger-ui .info h1,
    .swagger-ui .info h2,
    .swagger-ui .info h3 { color: #ecf5ff; font-family: inherit; }
    .swagger-ui .info p,
    .swagger-ui .info li,
    .swagger-ui .info .markdown p { color: #9bb4d6; }
    .swagger-ui .info a { color: #2fd4ff; }

    /* Tag groups */
    .swagger-ui .opblock-tag {
      color: #ecf5ff !important;
      font-family: inherit !important;
      border-bottom: 1px solid rgba(106,150,198,0.22) !important;
    }
    .swagger-ui .opblock-tag:hover { background: rgba(47,212,255,0.04) !important; }
    .swagger-ui .opblock-tag small { color: #9bb4d6 !important; }
    .swagger-ui .opblock-tag svg { fill: #9bb4d6 !important; }

 /* Operation blocks - card-style matching MuiCard */
    .swagger-ui .opblock {
      background: linear-gradient(140deg, rgba(9,21,39,0.92), rgba(9,21,39,0.72)) !important;
      border: 1px solid rgba(106,150,198,0.22) !important;
      border-radius: 14px !important;
      backdrop-filter: blur(6px);
      box-shadow: none !important;
      margin-bottom: 8px;
    }
    .swagger-ui .opblock .opblock-summary {
      border-bottom: 1px solid rgba(106,150,198,0.15);
      border-radius: 14px 14px 0 0;
    }
    .swagger-ui .opblock .opblock-summary-method { font-weight: 700; border-radius: 6px; }

    /* GET = primary cyan */
    .swagger-ui .opblock.opblock-get { border-color: rgba(47,212,255,0.22) !important; }
    .swagger-ui .opblock.opblock-get .opblock-summary-method { background: #2fd4ff; color: #030711; }
    .swagger-ui .opblock.opblock-get .opblock-summary { background: rgba(47,212,255,0.05); }

    /* POST = secondary green */
    .swagger-ui .opblock.opblock-post { border-color: rgba(20,241,149,0.22) !important; }
    .swagger-ui .opblock.opblock-post .opblock-summary-method { background: #14f195; color: #030711; }
    .swagger-ui .opblock.opblock-post .opblock-summary { background: rgba(20,241,149,0.05); }

    /* PUT = amber */
    .swagger-ui .opblock.opblock-put { border-color: rgba(252,161,48,0.22) !important; }
    .swagger-ui .opblock.opblock-put .opblock-summary-method { background: #fca130; color: #030711; }
    .swagger-ui .opblock.opblock-put .opblock-summary { background: rgba(252,161,48,0.04); }

    /* DELETE = red */
    .swagger-ui .opblock.opblock-delete { border-color: rgba(249,62,62,0.22) !important; }
    .swagger-ui .opblock.opblock-delete .opblock-summary-method { background: #f93e3e; color: #fff; }
    .swagger-ui .opblock.opblock-delete .opblock-summary { background: rgba(249,62,62,0.04); }

    /* PATCH = teal */
    .swagger-ui .opblock.opblock-patch { border-color: rgba(20,241,149,0.16) !important; }
    .swagger-ui .opblock.opblock-patch .opblock-summary-method { background: #50e3c2; color: #030711; }
    .swagger-ui .opblock.opblock-patch .opblock-summary { background: rgba(80,227,194,0.04); }

    .swagger-ui .opblock .opblock-summary-path,
    .swagger-ui .opblock .opblock-summary-path__deprecated,
    .swagger-ui .opblock .opblock-summary-description { color: #ecf5ff !important; }

    /* Expanded operation body */
    .swagger-ui .opblock-body { background: rgba(3,7,17,0.6) !important; }
    .swagger-ui .opblock-body pre,
    .swagger-ui .opblock-body pre.example {
      background: #030711 !important;
      color: #9bb4d6 !important;
      border: 1px solid rgba(106,150,198,0.22) !important;
      border-radius: 10px !important;
    }
    .swagger-ui .opblock-section-header {
      background: #091527 !important;
      border-bottom: 1px solid rgba(106,150,198,0.22) !important;
    }
    .swagger-ui .opblock-section-header h4 { color: #ecf5ff !important; }

    /* Tables */
    .swagger-ui table thead tr th,
    .swagger-ui table thead tr td { color: #9bb4d6 !important; border-bottom: 1px solid rgba(106,150,198,0.22) !important; }
    .swagger-ui .parameter__name,
    .swagger-ui .parameter__type { color: #ecf5ff !important; }
    .swagger-ui .parameter__name.required::after { color: #f93e3e !important; }
    .swagger-ui table tbody tr td { color: #9bb4d6 !important; border-bottom: 1px solid rgba(106,150,198,0.10) !important; }

    /* Models */
    .swagger-ui section.models { border: 1px solid rgba(106,150,198,0.22) !important; border-radius: 14px !important; }
    .swagger-ui section.models h4 { color: #ecf5ff !important; }
    .swagger-ui .model-container { background: #091527 !important; }
    .swagger-ui .model { color: #9bb4d6 !important; }

 /* Buttons - match MuiButton */
    .swagger-ui .btn {
      color: #ecf5ff;
      border-color: rgba(106,150,198,0.22);
      background: transparent;
      text-transform: none;
      font-weight: 600;
      border-radius: 10px;
      font-family: inherit;
    }
    .swagger-ui .btn:hover { background: rgba(47,212,255,0.08); }
    .swagger-ui .btn.authorize { color: #14f195; border-color: #14f195; }
    .swagger-ui .btn.authorize svg { fill: #14f195; }
    .swagger-ui .btn.execute { background: #2fd4ff; border-color: #2fd4ff; color: #030711; font-weight: 700; }

    /* Auth modal */
    .swagger-ui .dialog-ux .modal-ux {
      background: #091527 !important;
      border: 1px solid rgba(106,150,198,0.22);
      border-radius: 14px;
    }
    .swagger-ui .dialog-ux .modal-ux-header h3 { color: #ecf5ff; font-family: inherit; }
    .swagger-ui .dialog-ux .modal-ux-content p,
    .swagger-ui .dialog-ux .modal-ux-content label { color: #9bb4d6; }

    /* Inputs */
    .swagger-ui input[type=text],
    .swagger-ui textarea,
    .swagger-ui select {
      background: #030711 !important;
      color: #ecf5ff !important;
      border: 1px solid rgba(106,150,198,0.22) !important;
      border-radius: 10px !important;
      font-family: inherit !important;
    }
    .swagger-ui input[type=text]:focus,
    .swagger-ui textarea:focus { border-color: #2fd4ff !important; outline: none; }

    /* Responses */
    .swagger-ui .responses-inner { background: transparent !important; }
    .swagger-ui .response-col_status { color: #ecf5ff !important; }
    .swagger-ui .response-col_description { color: #9bb4d6 !important; }

    /* Scrollbar */
    ::-webkit-scrollbar { width: 8px; height: 8px; }
    ::-webkit-scrollbar-track { background: #030711; }
    ::-webkit-scrollbar-thumb { background: rgba(106,150,198,0.28); border-radius: 4px; }
    ::-webkit-scrollbar-thumb:hover { background: rgba(106,150,198,0.40); }

    /* Scheme container / server selector */
    .swagger-ui .scheme-container {
      background: #091527 !important;
      border-bottom: 1px solid rgba(106,150,198,0.22);
      box-shadow: none;
    }
    .swagger-ui .scheme-container .schemes > label { color: #9bb4d6; }

    /* Loading */
    .swagger-ui .loading-container .loading::after { color: #2fd4ff; }
    .swagger-ui .wrapper { padding: 0 20px; }
    .swagger-ui .info { margin: 30px 0 20px 0; }

    /* Links everywhere */
    .swagger-ui a { color: #2fd4ff; }

    /* Custom header bar */
    .ark-header {
      padding: 16px 24px;
      border-bottom: 1px solid rgba(106,150,198,0.22);
      font-family: 'Space Grotesk', 'IBM Plex Sans', 'Segoe UI', sans-serif;
      background: linear-gradient(140deg, rgba(9,21,39,0.92), rgba(9,21,39,0.72));
      backdrop-filter: blur(6px);
      display: flex; align-items: center; gap: 14px;
    }
    .ark-header img {
      width: 36px; height: 36px;
      filter: drop-shadow(0 0 10px rgba(47,212,255,0.28));
    }
    .ark-header strong {
      color: #ecf5ff;
      font-size: 16px;
      font-weight: 700;
      letter-spacing: 0.8px;
      text-transform: uppercase;
      font-family: 'Orbitron', 'Space Grotesk', 'Segoe UI', sans-serif;
      text-shadow: 0 0 14px rgba(47,212,255,0.28);
    }
    .ark-header small { color: #9bb4d6; font-size: 12px; margin-left: 4px; }
  </style>
</head>
<body>
  <div class="ark-header">
    <img src="/logo.svg" alt="" />
    <div>
      <strong>__PRODUCT_NAME__</strong>
      <small>API Docs &middot; /openapi.json</small>
    </div>
  </div>
  <div id="swagger-ui"></div>
  <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
  <script>
    window.ui = SwaggerUIBundle({
      url: '/openapi.json',
      dom_id: '#swagger-ui',
      deepLinking: true,
      persistAuthorization: true,
      docExpansion: 'list',
      defaultModelsExpandDepth: -1,
      syntaxHighlight: { theme: 'monokai' }
    });
  </script>
</body>
</html>"#;
    let html = crate::branding::render_template(html);
    (StatusCode::OK, Html(html)).into_response()
}
