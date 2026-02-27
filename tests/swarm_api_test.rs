//! HTTP API integration tests for the Agent Swarm
//!
//! These tests run against a live AgentArk server at localhost:8990.
//! Start the server before running: docker compose up -d --build
//!
//! Auth: Set AGENTARK_TEST_API_KEY to a valid API key, or start the server
//! with AGENTARK_INSECURE_NO_AUTH=true to bypass authentication.
//!
//! Run with: cargo test --test swarm_api_test

const BASE_URL: &str = "http://localhost:8990";

fn unique_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    ms.to_string()
}

fn api_key() -> Option<String> {
    std::env::var("AGENTARK_TEST_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
}

fn authed_client() -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(key) = api_key() {
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", key).parse().unwrap(),
        );
    }
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap()
}

async fn server_available() -> bool {
    reqwest::Client::new()
        .get(&format!("{}/health", BASE_URL))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Returns true if the server accepts unauthenticated requests on protected routes
/// (i.e. AGENTARK_INSECURE_NO_AUTH=true on the server side).
async fn server_allows_no_auth() -> bool {
    reqwest::Client::new()
        .get(&format!("{}/status", BASE_URL))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

macro_rules! skip_if_no_server {
    () => {
        if !server_available().await {
            eprintln!("SKIP: Server not running at {}", BASE_URL);
            return;
        }
    };
}

/// Skip if the server requires auth but no AGENTARK_TEST_API_KEY is set.
macro_rules! skip_if_no_auth {
    () => {
        if api_key().is_none() && !server_allows_no_auth().await {
            eprintln!(
                "SKIP: Server requires auth but AGENTARK_TEST_API_KEY is not set. \
                 Set the env var or start the server with AGENTARK_INSECURE_NO_AUTH=true."
            );
            return;
        }
    };
}

#[tokio::test]
async fn test_api_health() {
    skip_if_no_server!();
    let resp = reqwest::get(&format!("{}/health", BASE_URL)).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "OK");
}

#[tokio::test]
async fn test_api_swarm_status() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/swarm/status", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.get("enabled").is_some(), "Should have 'enabled' field");
    assert!(
        body.get("total_agents").is_some(),
        "Should have 'total_agents' field"
    );
    assert!(
        body.get("active_agents").is_some(),
        "Should have 'active_agents' field"
    );
    assert!(body["agents"].is_array(), "agents should be an array");
}

#[tokio::test]
async fn test_api_swarm_config_get() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/swarm/config", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["enabled"].is_boolean(), "enabled should be a boolean");
    assert!(
        body["max_specialists"].is_number(),
        "max_specialists should be a number"
    );
    assert!(
        body["default_timeout_secs"].is_number(),
        "default_timeout_secs should be a number"
    );
}

#[tokio::test]
async fn test_api_swarm_agents_list() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/swarm/agents", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["agents"].is_array(), "agents should be an array");
}

#[tokio::test]
async fn test_api_swarm_delegations_list() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/swarm/delegations", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["delegations"].is_array(),
        "delegations should be an array"
    );
}

#[tokio::test]
async fn test_api_swarm_config_update() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();

    // Get original config
    let original: serde_json::Value = client
        .get(&format!("{}/swarm/config", BASE_URL))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let was_enabled = original["enabled"].as_bool().unwrap();

    // Toggle
    let resp = client
        .post(&format!("{}/swarm/config", BASE_URL))
        .json(&serde_json::json!({ "enabled": !was_enabled }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Verify
    let updated: serde_json::Value = client
        .get(&format!("{}/swarm/config", BASE_URL))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["enabled"].as_bool().unwrap(), !was_enabled);

    // Restore
    let _ = client
        .post(&format!("{}/swarm/config", BASE_URL))
        .json(&serde_json::json!({ "enabled": was_enabled }))
        .send()
        .await;
}

#[tokio::test]
async fn test_api_swarm_agent_crud() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();

    // 1. Add agent
    let add_resp = client
        .post(&format!("{}/swarm/agents", BASE_URL))
        .json(&serde_json::json!({
            "name": "TestBot-CRUD",
            "agent_type": "Researcher",
            "llm_provider": "ollama",
            "llm_model": "llama3.2",
            "llm_base_url": "http://localhost:11434",
            "capabilities": ["web search", "data analysis"],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(add_resp.status(), 200);
    let add_body: serde_json::Value = add_resp.json().await.unwrap();
    assert_eq!(add_body["status"], "ok");
    let agent_id = add_body["agent_id"].as_str().unwrap().to_string();

    // 2. Verify in list
    let list_resp = client
        .get(&format!("{}/swarm/agents", BASE_URL))
        .send()
        .await
        .unwrap();
    let list_body: serde_json::Value = list_resp.json().await.unwrap();
    let agents = list_body["agents"].as_array().unwrap();
    assert!(
        !agents.is_empty(),
        "Should have at least one agent after adding"
    );

    // 3. Remove agent
    let del_resp = client
        .delete(&format!("{}/swarm/agents/{}", BASE_URL, agent_id))
        .send()
        .await
        .unwrap();
    assert_eq!(del_resp.status(), 200);
    let del_body: serde_json::Value = del_resp.json().await.unwrap();
    assert_eq!(del_body["status"], "ok");
}

#[tokio::test]
async fn test_api_swarm_add_different_providers() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();

    // Test adding agents with different LLM providers
    let providers = vec![
        (
            "OllamaBot",
            "ollama",
            "llama3.2",
            Some("http://localhost:11434"),
            None,
        ),
        (
            "AnthropicBot",
            "anthropic",
            "claude-sonnet-4-20250514",
            None,
            Some("test-key"),
        ),
        ("OpenAIBot", "openai", "gpt-4o", None, Some("test-key")),
    ];

    let mut created_ids = vec![];

    for (name, provider, model, base_url, api_key) in &providers {
        let mut body = serde_json::json!({
            "name": name,
            "agent_type": "Researcher",
            "llm_provider": provider,
            "llm_model": model,
            "capabilities": [],
        });
        if let Some(url) = base_url {
            body["llm_base_url"] = serde_json::json!(url);
        }
        if let Some(key) = api_key {
            body["llm_api_key"] = serde_json::json!(key);
        }

        let resp = client
            .post(&format!("{}/swarm/agents", BASE_URL))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "Failed to add {} agent", provider);
        let resp_body: serde_json::Value = resp.json().await.unwrap();
        created_ids.push(resp_body["agent_id"].as_str().unwrap().to_string());
    }

    // Clean up
    for id in &created_ids {
        let _ = client
            .delete(&format!("{}/swarm/agents/{}", BASE_URL, id))
            .send()
            .await;
    }
}

#[tokio::test]
async fn test_api_swarm_add_agent_types() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();

    let agent_types = vec![
        "Researcher",
        "Coder",
        "Analyst",
        "Writer",
        "Validator",
        "Planner",
        "CustomType",
    ];
    let mut created_ids = vec![];

    for agent_type in &agent_types {
        let resp = client
            .post(&format!("{}/swarm/agents", BASE_URL))
            .json(&serde_json::json!({
                "name": format!("Test-{}", agent_type),
                "agent_type": agent_type,
                "llm_provider": "ollama",
                "llm_model": "llama3.2",
                "llm_base_url": "http://localhost:11434",
                "capabilities": [],
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "Failed for agent type: {}", agent_type);
        let body: serde_json::Value = resp.json().await.unwrap();
        created_ids.push(body["agent_id"].as_str().unwrap().to_string());
    }

    // Clean up
    for id in &created_ids {
        let _ = client
            .delete(&format!("{}/swarm/agents/{}", BASE_URL, id))
            .send()
            .await;
    }
}

#[tokio::test]
async fn test_api_hooks_crud_with_action_name() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let hook_name = format!("test-hook-{}", unique_suffix());
    let action_name = "demo-action";

    let create_resp = client
        .post(&format!("{}/hooks", BASE_URL))
        .json(&serde_json::json!({
            "name": hook_name,
            "trigger": "on_error",
            "hook_type": "webhook",
            "url": "https://example.com/hook",
            "action_name": action_name
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), 201);
    let create_body: serde_json::Value = create_resp.json().await.unwrap();
    let id = create_body["id"].as_str().unwrap().to_string();

    let list_resp = client
        .get(&format!("{}/hooks", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp.status(), 200);
    let hooks: serde_json::Value = list_resp.json().await.unwrap();
    let items = hooks.as_array().expect("hooks response should be an array");
    let found = items
        .iter()
        .find(|h| h["id"] == id)
        .expect("created hook not found");
    assert_eq!(found["action_name"].as_str().unwrap_or(""), action_name);

    let del_resp = client
        .delete(&format!("{}/hooks/{}", BASE_URL, id))
        .send()
        .await
        .unwrap();
    assert_eq!(del_resp.status(), 200);
}

#[tokio::test]
async fn test_api_browser_session_missing_returns_404() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let missing_id = format!("missing-session-{}", unique_suffix());

    let status_resp = client
        .get(&format!(
            "{}/browser/sessions/{}/status",
            BASE_URL, missing_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(status_resp.status(), 404);

    let respond_resp = client
        .post(&format!(
            "{}/browser/sessions/{}/respond",
            BASE_URL, missing_id
        ))
        .json(&serde_json::json!({ "response": "continue" }))
        .send()
        .await
        .unwrap();
    assert_eq!(respond_resp.status(), 404);

    let empty_response_resp = client
        .post(&format!(
            "{}/browser/sessions/{}/respond",
            BASE_URL, missing_id
        ))
        .json(&serde_json::json!({ "response": "" }))
        .send()
        .await
        .unwrap();
    assert_eq!(empty_response_resp.status(), 400);
}

#[tokio::test]
async fn test_api_mcp_ssh_and_autonomy_surface_smoke() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();

    let mcp_resp = client
        .get(&format!("{}/mcp/servers?include_details=true", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(mcp_resp.status(), 200);
    let mcp_body: serde_json::Value = mcp_resp.json().await.unwrap();
    assert!(
        mcp_body
            .get("servers")
            .map(|v| v.is_array())
            .unwrap_or(false),
        "mcp servers response should contain array field 'servers'"
    );

    let ssh_keys_resp = client
        .get(&format!("{}/ssh/keys", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(ssh_keys_resp.status(), 200);
    let ssh_keys_body: serde_json::Value = ssh_keys_resp.json().await.unwrap();
    assert!(
        ssh_keys_body
            .get("keys")
            .map(|v| v.is_array())
            .unwrap_or(false),
        "ssh keys response should contain array field 'keys'"
    );

    let ssh_conn_resp = client
        .get(&format!("{}/ssh/connections", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(ssh_conn_resp.status(), 200);
    let ssh_conn_body: serde_json::Value = ssh_conn_resp.json().await.unwrap();
    assert!(
        ssh_conn_body
            .get("connections")
            .map(|v| v.is_string())
            .unwrap_or(false),
        "ssh connections response should contain string field 'connections'"
    );

    let incidents_resp = client
        .get(&format!("{}/autonomy/incidents/live", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(incidents_resp.status(), 200);
    let incidents_body: serde_json::Value = incidents_resp.json().await.unwrap();
    assert!(
        incidents_body
            .get("incidents")
            .map(|v| v.is_array())
            .unwrap_or(false),
        "autonomy incidents response should contain array field 'incidents'"
    );

    let timeline_resp = client
        .get(&format!("{}/autonomy/timeline?limit=20", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(timeline_resp.status(), 200);
    let timeline_body: serde_json::Value = timeline_resp.json().await.unwrap();
    assert!(
        timeline_body
            .get("events")
            .map(|v| v.is_array())
            .unwrap_or(false),
        "autonomy timeline response should contain array field 'events'"
    );
}
