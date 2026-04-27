use super::*;

pub(super) async fn load_global_user_preference_value(
    storage: &crate::storage::Storage,
    key: &str,
) -> Option<String> {
    storage
        .get_user_preference(key, None)
        .await
        .ok()
        .flatten()
        .map(|item| item.value)
}

pub(super) async fn upsert_global_user_preference_with_memory_sync(
    storage: &crate::storage::Storage,
    key: &str,
    value: &str,
    source: &str,
    sensitivity: &str,
) -> Result<()> {
    let item = storage
        .upsert_user_preference(key, value, 0.96, Some(source), None, Some(sensitivity))
        .await?;
    crate::core::learning::sync_user_preference_to_experience_item(
        storage,
        &item.key,
        &item.value,
        item.confidence as f64,
        source,
        Some(&item.sensitivity),
    )
    .await?;
    Ok(())
}

/// Get user profile (for checking onboarding status)
pub(super) async fn get_profile(State(state): State<AppState>) -> Json<ProfileResponse> {
    let profile = state.user_profile.read().await.clone();
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    let preferred_name = load_global_user_preference_value(&storage, "user_name").await;
    let priority_focus =
        load_global_user_preference_value(&storage, "assistant_priority_focus").await;
    let onboarding_complete = profile.onboarding_complete
        || crate::core::Agent::onboarding_profile_ready(
            &profile,
            preferred_name.as_deref(),
            priority_focus.as_deref(),
        );
    let personalization_dismissed = profile.personalization_dismissed && !onboarding_complete;
    Json(ProfileResponse {
        name: preferred_name,
        location: profile.location.clone(),
        timezone: profile.timezone.clone(),
        language: profile.language.clone(),
        tone: profile.tone.clone(),
        email_format: profile.email_format.clone(),
        preferences: priority_focus.clone(),
        priority_focus,
        onboarding_complete,
        personalization_dismissed,
    })
}

pub(super) async fn update_profile_onboarding(
    State(state): State<AppState>,
    Json(payload): Json<ProfileOnboardingUpdate>,
) -> Response {
    let preferred_name = payload.preferred_name.trim();
    let timezone = payload.timezone.trim();
    let tone = payload.tone.trim();
    let priority_focus = payload.priority_focus.trim();
    if preferred_name.is_empty()
        || timezone.is_empty()
        || tone.is_empty()
        || priority_focus.is_empty()
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Preferred name, timezone, response style, and priority focus are required."
                    .to_string(),
            }),
        )
            .into_response();
    }
    if timezone.parse::<chrono_tz::Tz>().is_err() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid timezone. Use an IANA name like America/New_York".to_string(),
            }),
        )
            .into_response();
    }

    let (storage, encrypted_storage) = {
        let agent = state.agent.read().await;
        (agent.storage.clone(), agent.encrypted_storage.clone())
    };
    for (key, value, sensitivity) in [
        ("user_name", preferred_name, "personal_identifier"),
        ("user_timezone", timezone, "prompt_safe"),
        ("preferred_tone", tone, "prompt_safe"),
        ("assistant_priority_focus", priority_focus, "prompt_safe"),
    ] {
        if let Err(error) = upsert_global_user_preference_with_memory_sync(
            &storage,
            key,
            value,
            "web_onboarding",
            sensitivity,
        )
        .await
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to persist onboarding memory: {}", error),
                }),
            )
                .into_response();
        }
    }
    let profile_bytes = {
        let mut profile = state.user_profile.write().await;
        profile.timezone = Some(timezone.to_string());
        profile.tone = Some(tone.to_string());
        profile.onboarding_complete = true;
        profile.personalization_dismissed = false;
        match serde_json::to_vec(&*profile) {
            Ok(bytes) => bytes,
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to encode profile update: {}", error),
                    }),
                )
                    .into_response();
            }
        }
    };
    if let Err(error) = encrypted_storage
        .set_encrypted("user_profile", &profile_bytes)
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to persist profile update: {}", error),
            }),
        )
            .into_response();
    }

    get_profile(State(state)).await.into_response()
}

pub(super) async fn update_profile_onboarding_dismiss(State(state): State<AppState>) -> Response {
    let encrypted_storage = {
        let agent = state.agent.read().await;
        agent.encrypted_storage.clone()
    };
    let profile_bytes = {
        let mut profile = state.user_profile.write().await;
        profile.personalization_dismissed = true;
        match serde_json::to_vec(&*profile) {
            Ok(bytes) => bytes,
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to encode profile dismissal: {}", error),
                    }),
                )
                    .into_response();
            }
        }
    };
    if let Err(error) = encrypted_storage
        .set_encrypted("user_profile", &profile_bytes)
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to persist profile dismissal: {}", error),
            }),
        )
            .into_response();
    }

    get_profile(State(state)).await.into_response()
}
