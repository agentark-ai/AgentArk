//! Core API integration tests for AgentArk
//!
//! Tests run against a live server at localhost:8990.
//! Start the server before running: docker compose up -d --build
//!
//! Auth: Set AGENTARK_TEST_API_KEY to a valid API key, or start the server
//! with AGENTARK_INSECURE_NO_AUTH=true to bypass authentication.
//!
//! Run with: cargo test --test api_core_test

const BASE_URL: &str = "http://localhost:8990";

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

// ==================== Notification Tests ====================

#[tokio::test]
async fn test_notifications_list() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/notifications", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["notifications"].is_array());
    assert!(body["total"].is_number());
}

#[tokio::test]
async fn test_notifications_count() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/notifications/count", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["total"].is_number());
}

#[tokio::test]
async fn test_notifications_mark_all_read() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();

    // Mark all read
    let resp = client
        .post(&format!("{}/notifications/read-all", BASE_URL))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Verify unread count is 0
    let count_resp = client
        .get(&format!("{}/notifications/count?unread=true", BASE_URL))
        .send()
        .await
        .unwrap();
    let count_body: serde_json::Value = count_resp.json().await.unwrap();
    assert_eq!(count_body["total"].as_u64().unwrap(), 0);
}

#[tokio::test]
async fn test_notifications_unread_filter() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();

    // First mark all read
    let _ = client
        .post(&format!("{}/notifications/read-all", BASE_URL))
        .json(&serde_json::json!({}))
        .send()
        .await;

    // Fetch with unread filter — should be empty
    let resp = client
        .get(&format!("{}/notifications?unread=true", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["notifications"].as_array().unwrap().len(), 0);
}

// ==================== Task Tests ====================

#[tokio::test]
async fn test_tasks_list() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/tasks", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    // tasks can be an array or { tasks: [] }
    let _tasks = body
        .as_array()
        .or_else(|| body["tasks"].as_array())
        .expect("Should return tasks");
}

#[tokio::test]
async fn test_task_create() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .post(&format!("{}/tasks", BASE_URL))
        .json(&serde_json::json!({
            "description": "Integration test task — safe to delete",
            "schedule": "manual"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"].as_str().unwrap(), "ok");
    assert!(body["task_id"].is_string());
}

// ==================== Chat Tests ====================

#[tokio::test]
async fn test_chat_endpoint_exists() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .post(&format!("{}/chat", BASE_URL))
        .json(&serde_json::json!({
            "message": "ping",
            "channel": "web"
        }))
        .send()
        .await
        .unwrap();
    // 200 if LLM configured, 500 if not — both are valid
    assert!(
        resp.status() == 200 || resp.status() == 500,
        "Expected 200 or 500, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn test_conversations_list() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/conversations", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

// ==================== Integration Tests ====================

#[tokio::test]
async fn test_integrations_list() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/integrations", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["integrations"].is_array());
}

// ==================== Skills Tests ====================

#[tokio::test]
async fn test_skills_list() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/skills", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

// ==================== Security Tests ====================

#[tokio::test]
async fn test_security_logs() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/security/logs?limit=5", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["logs"].is_array());
}

#[tokio::test]
async fn test_master_password_status() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/security/master-password/status", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["is_set"].is_boolean());
}

// ==================== Autonomy Tests ====================

#[tokio::test]
async fn test_autonomy_briefing() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/autonomy/briefing", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn test_autonomy_nudges() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/autonomy/nudges", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["nudges"].is_array());
}

#[tokio::test]
async fn test_autonomy_settings() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/autonomy/settings", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

// ==================== Memory Tests ====================

#[tokio::test]
async fn test_memory_stats() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/memory/stats", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn test_memory_episodes() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/memory/episodes", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn test_memory_facts() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/memory/facts", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

// ==================== LLM Analytics Tests ====================

#[tokio::test]
async fn test_llm_analytics() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/analytics/llm?range=24h&bucket=hour", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

// ==================== Documents Tests ====================

#[tokio::test]
async fn test_documents_list() {
    skip_if_no_server!();
    skip_if_no_auth!();
    let client = authed_client();
    let resp = client
        .get(&format!("{}/documents", BASE_URL))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}
