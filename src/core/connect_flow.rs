//! Multi-turn chat flows for onboarding integrations (connect + secret setup).
//!
//! This is intentionally lightweight:
//! - Start when the user asks to connect a known integration.
//! - Ask the user to provide required secrets via chat-safe commands.
//! - When secrets are saved, run a connectivity check and enable on success.

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
        id: "home_assistant",
        name: "Home Assistant",
        triggers: &["home assistant", "homeassistant"],
        required: SecretRequirement {
            kind: SecretRequirementKind::All,
            keys: &["HOME_ASSISTANT_URL", "HOME_ASSISTANT_TOKEN"],
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
