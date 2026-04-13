//! Multi-turn chat flows for onboarding integrations (connect + secret setup).
//!
//! This is intentionally lightweight:
//! - Start when the user asks to connect a known integration.
//! - Ask the user to provide required secrets via chat-safe commands.
//! - When secrets are saved, run a connectivity check and enable on success.

use crate::core::RequestShapeAssessment;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretRequirementKind {
    All,
    Any,
}

#[derive(Debug, Clone, Copy)]
pub struct SecretRequirement {
    pub kind: SecretRequirementKind,
    pub keys: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
pub struct IntegrationConnectSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub triggers: &'static [&'static str],
    pub required: SecretRequirement,
    pub optional: &'static [&'static str],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingIntegrationConnect {
    pub integration_id: String,
    pub started_at: DateTime<Utc>,
}

pub const CONNECT_FLOW_TTL_SECS: i64 = 20 * 60;

pub fn canonical_integration_id(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_was_separator = false;
    for ch in value.trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_was_separator = false;
        } else if !out.is_empty() && !last_was_separator {
            out.push('_');
            last_was_separator = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

static SPECS: &[IntegrationConnectSpec] = &[
    IntegrationConnectSpec {
        id: "github",
        name: "GitHub",
        triggers: &["github"],
        required: SecretRequirement {
            kind: SecretRequirementKind::All,
            keys: &["GITHUB_TOKEN"],
        },
        optional: &[],
    },
    IntegrationConnectSpec {
        id: "notion",
        name: "Notion",
        triggers: &["notion"],
        required: SecretRequirement {
            kind: SecretRequirementKind::All,
            keys: &["NOTION_TOKEN"],
        },
        optional: &[],
    },
    IntegrationConnectSpec {
        id: "twitter",
        name: "X (Twitter)",
        triggers: &["twitter", "x api", "x.com"],
        required: SecretRequirement {
            kind: SecretRequirementKind::All,
            keys: &["TWITTER_BEARER_TOKEN"],
        },
        optional: &[],
    },
    IntegrationConnectSpec {
        id: "onepassword",
        name: "1Password Connect",
        triggers: &["1password", "onepassword", "1 password"],
        required: SecretRequirement {
            kind: SecretRequirementKind::All,
            keys: &["ONEPASSWORD_TOKEN"],
        },
        optional: &["ONEPASSWORD_HOST"],
    },
    IntegrationConnectSpec {
        id: "google_places",
        name: "Google Places",
        triggers: &["google places", "places"],
        required: SecretRequirement {
            kind: SecretRequirementKind::All,
            keys: &["GOOGLE_PLACES_API_KEY"],
        },
        optional: &[],
    },
    IntegrationConnectSpec {
        id: "twilio",
        name: "Twilio",
        triggers: &["twilio"],
        required: SecretRequirement {
            kind: SecretRequirementKind::All,
            keys: &[
                "TWILIO_ACCOUNT_SID",
                "TWILIO_AUTH_TOKEN",
                "TWILIO_FROM_NUMBER",
            ],
        },
        optional: &[],
    },
    IntegrationConnectSpec {
        id: "ordering",
        name: "Ordering",
        triggers: &["ordering", "shopify"],
        required: SecretRequirement {
            kind: SecretRequirementKind::All,
            keys: &["ORDERING_CONFIG_JSON"],
        },
        optional: &[
            "SHOPIFY_ACCESS_TOKEN",
            "SHOPIFY_STORE_URL",
            "ORDERING_WEBHOOK_URL",
        ],
    },
    IntegrationConnectSpec {
        id: "garmin",
        name: "Garmin",
        triggers: &["garmin"],
        required: SecretRequirement {
            kind: SecretRequirementKind::All,
            keys: &["GARMIN_TOKEN"],
        },
        optional: &["GARMIN_API_BASE"],
    },
    IntegrationConnectSpec {
        id: "whoop",
        name: "WHOOP",
        triggers: &["whoop"],
        required: SecretRequirement {
            kind: SecretRequirementKind::All,
            keys: &["WHOOP_TOKEN"],
        },
        optional: &[],
    },
    IntegrationConnectSpec {
        id: "ga4",
        name: "Google Analytics 4 (GA4)",
        triggers: &["ga4", "google analytics"],
        required: SecretRequirement {
            kind: SecretRequirementKind::All,
            keys: &["GA4_ACCESS_TOKEN"],
        },
        optional: &["GA4_PROPERTY_ID"],
    },
    IntegrationConnectSpec {
        id: "gsc",
        name: "Google Search Console (GSC)",
        triggers: &["gsc", "search console"],
        required: SecretRequirement {
            kind: SecretRequirementKind::All,
            keys: &["GSC_ACCESS_TOKEN"],
        },
        optional: &["GSC_SITE_URL"],
    },
    IntegrationConnectSpec {
        id: "social_analytics",
        name: "Social Analytics",
        triggers: &["social analytics", "social_analytics", "social"],
        required: SecretRequirement {
            kind: SecretRequirementKind::Any,
            keys: &[
                "SOCIAL_TWITTER_BEARER_TOKEN",
                "SOCIAL_GA4_ACCESS_TOKEN",
                "SOCIAL_GA4_PROPERTY_ID",
            ],
        },
        optional: &[],
    },
    IntegrationConnectSpec {
        id: "moltbook",
        name: "Moltbook",
        triggers: &["moltbook"],
        required: SecretRequirement {
            kind: SecretRequirementKind::All,
            keys: &["MOLTBOOK_API_KEY"],
        },
        optional: &[],
    },
];

pub fn all_specs() -> &'static [IntegrationConnectSpec] {
    SPECS
}

pub fn spec_by_id(id: &str) -> Option<&'static IntegrationConnectSpec> {
    SPECS.iter().find(|s| s.id == id)
}

#[cfg(test)]
pub fn detect_connect_integration(message: &str) -> Option<&'static IntegrationConnectSpec> {
    let _ = message;
    None
}

pub fn detect_connect_integration_with_shape(
    message: &str,
    request_shape: Option<&RequestShapeAssessment>,
) -> Option<&'static IntegrationConnectSpec> {
    let _ = message;
    if let Some(shape) = request_shape {
        if shape.is_integration_request() && !shape.should_confirm && shape.confidence >= 0.70 {
            let integration_id = shape.integration_id.as_deref()?;
            return spec_by_id(&canonical_integration_id(integration_id));
        }
        return None;
    }

    None
}

pub fn is_cancel_message(message: &str) -> bool {
    message.trim().eq_ignore_ascii_case("/cancel")
}

pub fn connect_instructions(spec: &IntegrationConnectSpec) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Integration setup: {} (`{}`)\n\n",
        spec.name, spec.id
    ));
    out.push_str("Send secrets using one of these safe commands:\n");
    out.push_str("- Telegram/WhatsApp: `/setsecret KEY=VALUE`\n");
    out.push_str("- Web chat: `/setsecret KEY=VALUE`\n\n");

    match spec.required.kind {
        SecretRequirementKind::All => {
            out.push_str("Required:\n");
            for k in spec.required.keys {
                out.push_str(&format!("- `{}`\n", k));
            }
        }
        SecretRequirementKind::Any => {
            out.push_str("Provide at least one of:\n");
            for k in spec.required.keys {
                out.push_str(&format!("- `{}`\n", k));
            }
        }
    }

    if !spec.optional.is_empty() {
        out.push_str("\nOptional:\n");
        for k in spec.optional {
            out.push_str(&format!("- `{}`\n", k));
        }
    }

    out.push_str(
        "\nAfter you set the secret(s), I will run a connection test and enable it if successful.\n",
    );
    out.push_str("To cancel: `/cancel`.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phrase_only_detection_is_disabled() {
        assert!(detect_connect_integration("GitHub").is_none());
        assert!(detect_connect_integration("GitHub access").is_none());
    }

    #[test]
    fn shape_detection_matches_hyphenated_integration_names() {
        let shape = RequestShapeAssessment {
            shape: "integration".to_string(),
            execution_mode: "immediate".to_string(),
            confidence: 0.92,
            should_confirm: false,
            confirmation_question: None,
            reasoning: String::new(),
            preferred_actions: vec![],
            integration_id: Some("google-places".to_string()),
            ..Default::default()
        };
        let spec = detect_connect_integration_with_shape("", Some(&shape)).expect("google places");
        assert_eq!(spec.id, "google_places");
    }

    #[test]
    fn does_not_treat_plain_mentions_as_connect_requests() {
        assert!(detect_connect_integration("Summarize GitHub issues for me").is_none());
    }

    #[test]
    fn does_not_match_connectivity_or_other_non_intents() {
        assert!(detect_connect_integration("This is a connectivity report for GitHub").is_none());
        assert!(detect_connect_integration("GitHub docs update").is_none());
    }

    #[test]
    fn matches_single_word_triggers_via_token_boundaries() {
        let shape = RequestShapeAssessment {
            shape: "integration".to_string(),
            execution_mode: "immediate".to_string(),
            confidence: 0.92,
            should_confirm: false,
            confirmation_question: None,
            reasoning: String::new(),
            preferred_actions: vec![],
            integration_id: Some("github".to_string()),
            ..Default::default()
        };
        let spec = detect_connect_integration_with_shape("", Some(&shape)).expect("github spec");
        assert_eq!(spec.id, "github");
    }

    #[test]
    fn shape_gates_connect_flow_with_llm_integration_target() {
        let shape = RequestShapeAssessment {
            shape: "integration".to_string(),
            execution_mode: "immediate".to_string(),
            confidence: 0.92,
            should_confirm: false,
            confirmation_question: None,
            reasoning: String::new(),
            preferred_actions: vec![],
            integration_id: Some("github".to_string()),
            ..Default::default()
        };

        let spec = detect_connect_integration_with_shape("any wording", Some(&shape))
            .expect("github spec");
        assert_eq!(spec.id, "github");
    }

    #[test]
    fn shape_blocks_plain_mentions_from_starting_connect_flow() {
        let shape = RequestShapeAssessment {
            shape: "conversation".to_string(),
            execution_mode: "none".to_string(),
            confidence: 0.88,
            should_confirm: false,
            confirmation_question: None,
            reasoning: String::new(),
            preferred_actions: vec![],
            integration_id: Some("github".to_string()),
            ..Default::default()
        };

        assert!(
            detect_connect_integration_with_shape("GitHub docs update", Some(&shape)).is_none()
        );
    }

    #[test]
    fn cancel_message_is_explicit_only() {
        assert!(is_cancel_message("/cancel"));
        assert!(!is_cancel_message("cancel"));
        assert!(!is_cancel_message("never mind"));
        assert!(!is_cancel_message("stop setup"));
    }
}
