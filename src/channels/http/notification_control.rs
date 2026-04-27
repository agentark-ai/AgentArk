use super::*;

// ==================== Notification Endpoints ====================
pub(super) async fn notification_stream_endpoint(State(state): State<AppState>) -> Response {
    let mut notification_events = {
        let agent = state.agent.read().await;
        agent.subscribe_notification_events()
    };

    let (tx, rx) =
        tokio::sync::mpsc::channel::<std::result::Result<Event, std::convert::Infallible>>(32);
    crate::spawn_logged!("src/channels/http.rs:33599", async move {
        let connected = serde_json::json!({
            "kind": "notifications.connected",
            "connected_at": chrono::Utc::now().to_rfc3339(),
        });
        if tx
            .send(Ok(Event::default()
                .event("connected")
                .data(connected.to_string())))
            .await
            .is_err()
        {
            return;
        }

        loop {
            match notification_events.recv().await {
                Ok(payload) => {
                    let message = match serde_json::to_string(&payload) {
                        Ok(message) => message,
                        Err(error) => {
                            tracing::warn!(
                                "Failed to serialize notification stream event: {}",
                                error
                            );
                            continue;
                        }
                    };
                    if tx
                        .send(Ok(Event::default().event("notification").data(message)))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    let resync = serde_json::json!({
                        "kind": "notifications.resync",
                        "reason": "lagged",
                        "skipped": skipped,
                    });
                    if tx
                        .send(Ok(Event::default()
                            .event("resync")
                            .data(resync.to_string())))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    let closed = serde_json::json!({
                        "kind": "notifications.closed",
                    });
                    let _ = tx
                        .send(Ok(Event::default()
                            .event("closed")
                            .data(closed.to_string())))
                        .await;
                    break;
                }
            }
        }
    });

    Sse::new(cap_sse_lifetime(
        tokio_stream::wrappers::ReceiverStream::new(rx),
    ))
    .keep_alive(KeepAlive::default())
    .into_response()
}

pub(super) async fn list_notifications_endpoint(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20u64);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let unread_only = params.get("unread").map(|v| v == "true").unwrap_or(false);
    let agent = state.agent.read().await;
    let total = agent
        .storage
        .count_notifications(unread_only)
        .await
        .unwrap_or(0);
    match agent
        .storage
        .list_notifications(limit, offset, unread_only)
        .await
    {
        Ok(notifs) => {
            let list: Vec<serde_json::Value> = notifs
                .iter()
                .map(|n| {
                    serde_json::json!({
                        "id": n.id, "title": n.title, "body": n.body,
                        "level": n.level, "source": n.source, "read": n.read,
                        "created_at": n.created_at,
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"notifications": list, "total": total, "limit": limit, "offset": offset}))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn mark_read_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.mark_notification_read(&id).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn mark_all_read_endpoint(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.mark_all_notifications_read().await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn notification_count_endpoint(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.count_unread_notifications().await {
        Ok(count) => (StatusCode::OK, Json(serde_json::json!({"unread": count}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

// ==================== Analytics Endpoints ====================
