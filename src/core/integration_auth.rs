//! Generic integration authentication manifest.
//!
//! Models how any integration — bundled, extension-pack, or custom — advertises
//! what credentials it needs, how to collect them, where they land in the flat
//! secret store, and what happens when the user submits. The manifest is the
//! single source of truth consumed by:
//!
//! - the chat-inline credential prompt (rendered at
//!   `frontend/src/components/NativeWorkspace.tsx:18341`),
//! - the settings-panel form,
//! - the runtime missing-credential interception path in the tool executor,
//! - and the OAuth callback handler.
//!
//! Bundled integrations keep their legacy flat storage keys
//! (e.g. `github_token`, `twilio_account_sid`). Extension-pack and custom
//! integrations default to namespaced keys (`ext.<pack_id>.<field_key>`) so
//! two custom integrations cannot silently collide on generic slot names like
//! `client_id` or `access_token`.
//!
//! This module is intent-based: it never infers the auth shape from user
//! phrasing. Every manifest is either explicitly declared or synthesised by a
//! deterministic adapter from existing registry data.

use serde::{Deserialize, Serialize};

/// Default namespace prefix for custom/extension-pack manifests. Bundled
/// adapters pass `None` for the namespace and keep their legacy flat keys.
pub const EXTENSION_PACK_NAMESPACE_PREFIX: &str = "ext";

/// Integration id used for generic raw-key prompts that are synthesised when
/// the runtime sees an unmapped `{{secret:KEY}}` reference or an ambiguous
/// manifest match. Kept separate so callers can tell the UX apart.
pub const RAW_KEY_MANIFEST_ID: &str = "__raw_secret__";

/// Top-level manifest describing an integration's auth contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IntegrationAuthManifest {
    /// Stable slug, e.g. "github". For generic raw-key prompts use
    /// [`RAW_KEY_MANIFEST_ID`].
    pub integration_id: String,
    /// Human-readable name shown in the prompt header.
    pub display_name: String,
    /// One-line help text shown under the header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional link to the integration's credential-provisioning docs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
    /// Short warning rendered prominently (e.g. "The LLM never sees your
    /// key — this form posts directly to AgentArk.").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    pub mode: AuthMode,
    /// Submit-button label and post-submit behaviour.
    #[serde(default)]
    pub post_submit: PostSubmitAction,
}

/// Auth shape. Everything the inline prompt needs to render + everything
/// the backend needs to honour the handoff.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthMode {
    /// One or more secret fields. Covers plain API keys, multi-field tokens
    /// (Twilio SID+token), basic auth (username+password), bearer tokens, and
    /// generic raw-key prompts.
    Secrets { fields: Vec<AuthField> },

    /// OAuth 2.0 authorization-code flow. The inline prompt renders a
    /// "Connect with <name>" button that opens the authorize URL; the
    /// callback handler exchanges the code for tokens and writes them into
    /// `token_storage`.
    OAuth2AuthorizationCode(OAuth2CodeFlow),

    /// OAuth 2.0 device-code flow for limited-input devices.
    OAuth2DeviceCode(OAuth2DeviceFlow),

    /// Hybrid: form fields (typically client_id + client_secret) submitted
    /// first, then OAuth2 authorization-code step uses those values.
    Hybrid {
        fields: Vec<AuthField>,
        oauth: OAuth2CodeFlow,
    },
}

/// One field shown to the user inside the inline prompt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthField {
    /// Stable form key used by the frontend. Not the storage slot.
    pub key: String,
    /// Display label. E.g. "Personal API Key".
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Per-field help text rendered below the input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    pub input_type: FieldInputType,
    pub required: bool,
    /// Concrete slot(s) in the flat secret store that receive this field's
    /// value on submit. Multi-target means the same value is written to
    /// multiple aliases (e.g. `GITHUB_TOKEN` + legacy `github_token`).
    /// For extension-pack manifests these should be namespaced via
    /// [`namespaced_storage_target`] so generic slot names like `client_id`
    /// cannot silently collide across packs.
    pub storage_targets: Vec<String>,
    /// Optional value-shape validation. `None` means accept any non-empty
    /// string. Validation enforced on the backend before any storage write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<FieldValidation>,
}

/// Shape of the input control rendered for an [`AuthField`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FieldInputType {
    /// Plain text (e.g. a client_id or account SID).
    Text,
    /// Password-masked input. Default for anything secret.
    Password,
    /// Multi-line free text (rare — used for things like PEM private keys).
    Textarea,
    /// Dropdown with a fixed option list.
    Select { options: Vec<String> },
}

/// Optional validator applied before a secret is persisted. Kept intentionally
/// narrow: we never derive the validator from user phrasing; it's declared by
/// the manifest author.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FieldValidation {
    /// Minimum character length, inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_len: Option<usize>,
    /// Maximum character length, inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_len: Option<usize>,
    /// Required prefix (e.g. "sk-" for OpenAI keys). Matched byte-for-byte.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub must_start_with: Option<String>,
}

/// OAuth 2.0 authorization-code flow parameters.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OAuth2CodeFlow {
    /// Authorization endpoint URL. Placeholders `{client_id}`, `{redirect_uri}`,
    /// `{scopes}`, `{state}`, and `{code_challenge}` are substituted by the
    /// backend when generating the authorize URL; no regex or free-form
    /// interpolation is performed.
    pub authorize_url: String,
    /// Token-exchange endpoint URL.
    pub token_url: String,
    /// Requested scopes joined by the scope delimiter declared by the
    /// provider (space by default).
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Whether PKCE should be used. Defaults to `true` for public clients.
    #[serde(default = "default_true")]
    pub pkce: bool,
    /// Secret-store slot to read the OAuth client_id from. For Hybrid mode
    /// this points at one of the fields the user just filled in; for pure
    /// OAuth it points at an app-owned slot the operator provisioned.
    pub client_id_source: SecretSlot,
    /// Optional client_secret source. `None` when the provider supports
    /// public clients (no client_secret).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret_source: Option<SecretSlot>,
    /// Where access, refresh, and expiry should be persisted.
    pub token_storage: OAuthTokenStorage,
    /// Optional extra query params on the authorize URL (e.g. `prompt=consent`).
    #[serde(default)]
    pub extra_authorize_params: Vec<(String, String)>,
}

/// OAuth 2.0 device-code flow parameters (RFC 8628).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OAuth2DeviceFlow {
    pub device_authorization_url: String,
    pub token_url: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    pub client_id_source: SecretSlot,
    pub token_storage: OAuthTokenStorage,
}

/// A pointer to one slot in the flat secret store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct SecretSlot(pub String);

/// Where OAuth tokens land after a successful exchange. The inline prompt
/// never sees raw token values; they're written here by the callback
/// handler.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OAuthTokenStorage {
    pub access_token_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token_key: Option<String>,
    /// RFC 3339 expiry timestamp key. If present, tokens stored here get
    /// timestamp-checked before use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_key: Option<String>,
}

/// What the frontend does after the user clicks the submit button.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PostSubmitAction {
    /// Button text. Defaults to "Save" for Secrets, "Connect" for OAuth/Hybrid.
    pub label: String,
    pub after: PostSubmitAfter,
}

impl Default for PostSubmitAction {
    fn default() -> Self {
        Self {
            label: "Save".to_string(),
            after: PostSubmitAfter::CloseAndResume,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PostSubmitAfter {
    /// Store each field value, call the resume hook, then dismiss the prompt.
    CloseAndResume,
    /// Store each field value, then open the OAuth authorize URL the backend
    /// returns. The callback handler dismisses the prompt after token exchange.
    LaunchOAuth,
    /// Run a connectivity check before clearing the prompt. If the check
    /// fails the prompt stays open with an error message.
    TestConnection,
}

fn default_true() -> bool {
    true
}

/// Build a namespaced storage slot for an extension-pack manifest. Returns a
/// key like `ext.<pack_id>.<field_key>`. Bundled adapters MUST NOT use this;
/// they pass the raw legacy key directly so the existing secrets survive
/// unchanged.
pub fn namespaced_storage_target(pack_id: &str, field_key: &str) -> String {
    let pack_id = pack_id.trim();
    let field_key = field_key.trim();
    format!(
        "{}.{}.{}",
        EXTENSION_PACK_NAMESPACE_PREFIX, pack_id, field_key
    )
}

/// Synthesize a generic "enter value for <KEY>" manifest used when the
/// runtime sees an unmapped `{{secret:KEY}}` reference or an ambiguous
/// manifest match. Single password field; the literal key becomes its
/// storage target so the next tool call succeeds immediately.
pub fn raw_key_manifest(missing_key: &str) -> IntegrationAuthManifest {
    let key = missing_key.trim();
    IntegrationAuthManifest {
        integration_id: RAW_KEY_MANIFEST_ID.to_string(),
        display_name: "Secure credential".to_string(),
        description: Some(format!(
            "A tool run needs a value for `{key}`. Enter it securely — it is stored encrypted and never shown to the assistant."
        )),
        docs_url: None,
        warning: Some(
            "The assistant does not see this value. It is sent directly to AgentArk over the inline prompt and stored encrypted."
                .to_string(),
        ),
        mode: AuthMode::Secrets {
            fields: vec![AuthField {
                key: "value".to_string(),
                label: format!("Value for {key}"),
                placeholder: None,
                help: None,
                input_type: FieldInputType::Password,
                required: true,
                storage_targets: vec![key.to_string()],
                validation: None,
            }],
        },
        post_submit: PostSubmitAction {
            label: "Save".to_string(),
            after: PostSubmitAfter::CloseAndResume,
        },
    }
}

/// All storage slots this manifest writes to across Secrets/Hybrid fields.
/// Does not include OAuth token storage (which is written by the callback,
/// not by the form submit).
pub fn manifest_form_storage_targets(manifest: &IntegrationAuthManifest) -> Vec<String> {
    match &manifest.mode {
        AuthMode::Secrets { fields } | AuthMode::Hybrid { fields, .. } => fields
            .iter()
            .flat_map(|field| field.storage_targets.iter().cloned())
            .collect(),
        AuthMode::OAuth2AuthorizationCode(_) | AuthMode::OAuth2DeviceCode(_) => Vec::new(),
    }
}

/// All storage slots the manifest eventually owns, including OAuth token
/// slots. Used by the reverse-lookup (runtime missing-credential path) to
/// decide whether a missing key belongs to a known manifest.
pub fn manifest_all_storage_targets(manifest: &IntegrationAuthManifest) -> Vec<String> {
    let mut out = manifest_form_storage_targets(manifest);
    let oauth = match &manifest.mode {
        AuthMode::OAuth2AuthorizationCode(flow) => Some(flow),
        AuthMode::Hybrid { oauth, .. } => Some(oauth),
        AuthMode::OAuth2DeviceCode(_) => None,
        AuthMode::Secrets { .. } => None,
    };
    if let Some(flow) = oauth {
        out.push(flow.token_storage.access_token_key.clone());
        if let Some(refresh) = flow.token_storage.refresh_token_key.clone() {
            out.push(refresh);
        }
        if let Some(expires) = flow.token_storage.expires_at_key.clone() {
            out.push(expires);
        }
        out.push(flow.client_id_source.0.clone());
        if let Some(secret) = flow.client_secret_source.as_ref() {
            out.push(secret.0.clone());
        }
    }
    if let AuthMode::OAuth2DeviceCode(flow) = &manifest.mode {
        out.push(flow.token_storage.access_token_key.clone());
        if let Some(refresh) = flow.token_storage.refresh_token_key.clone() {
            out.push(refresh);
        }
        if let Some(expires) = flow.token_storage.expires_at_key.clone() {
            out.push(expires);
        }
        out.push(flow.client_id_source.0.clone());
    }
    out
}

/// Outcome of reverse-looking-up a missing `{{secret:KEY}}` against a set of
/// manifests. Drives the runtime-missing-credential UX decision ladder.
#[derive(Debug, Clone, PartialEq)]
pub enum ReverseLookupOutcome {
    /// Exactly one manifest claims the key — raise the integration-specific
    /// inline prompt from that manifest.
    Unique(IntegrationAuthManifest),
    /// Multiple manifests claim the same key — ambiguity. The caller raises
    /// a generic raw-key inline prompt and logs a warning.
    Ambiguous {
        key: String,
        candidates: Vec<String>,
    },
    /// No manifest claims the key. Caller raises a generic raw-key inline
    /// prompt.
    None,
}

/// Classify a `{{secret:KEY}}` key against a pool of known manifests. The
/// caller supplies the pool (bundled + extension-pack) so this function stays
/// pure and testable without touching storage.
pub fn resolve_secret_key_among_manifests(
    key: &str,
    manifests: &[IntegrationAuthManifest],
) -> ReverseLookupOutcome {
    let needle = key.trim();
    if needle.is_empty() {
        return ReverseLookupOutcome::None;
    }
    let mut matches: Vec<&IntegrationAuthManifest> = Vec::new();
    for manifest in manifests {
        if manifest_all_storage_targets(manifest)
            .iter()
            .any(|slot| slot == needle)
        {
            matches.push(manifest);
        }
    }
    match matches.len() {
        0 => ReverseLookupOutcome::None,
        1 => ReverseLookupOutcome::Unique(matches[0].clone()),
        _ => ReverseLookupOutcome::Ambiguous {
            key: needle.to_string(),
            candidates: matches
                .iter()
                .map(|manifest| manifest.integration_id.clone())
                .collect(),
        },
    }
}

/// Synthesize an [`IntegrationAuthManifest`] from an extension pack's
/// declared auth spec. Returns `None` for packs that declare no auth
/// (`ExtensionPackAuthMode::None`). All storage slots are namespaced via
/// [`namespaced_storage_target`] so two custom packs cannot silently collide
/// on generic slot names like `client_id`.
pub fn manifest_from_extension_pack(
    pack: &crate::extension_packs::ExtensionPackManifest,
) -> Option<IntegrationAuthManifest> {
    use crate::extension_packs::ExtensionPackAuthMode;

    let pack_id = pack.id.trim().to_string();
    if pack_id.is_empty() {
        return None;
    }
    let display_name = if pack.name.trim().is_empty() {
        pack_id.clone()
    } else {
        pack.name.trim().to_string()
    };
    let description = if pack.description.trim().is_empty() {
        None
    } else {
        Some(pack.description.trim().to_string())
    };
    let warning = Some(
        "The assistant does not see these values. They are sent directly to AgentArk and stored encrypted."
            .to_string(),
    );

    let required = &pack.auth.required_secrets;
    let mode = match pack.auth.mode {
        ExtensionPackAuthMode::None => return None,
        ExtensionPackAuthMode::ApiKey => AuthMode::Secrets {
            fields: pack_secret_fields(&pack_id, required),
        },
        ExtensionPackAuthMode::Basic => AuthMode::Secrets {
            fields: vec![
                AuthField {
                    key: "username".to_string(),
                    label: "Username".to_string(),
                    placeholder: None,
                    help: None,
                    input_type: FieldInputType::Text,
                    required: true,
                    storage_targets: vec![namespaced_storage_target(&pack_id, "username")],
                    validation: None,
                },
                AuthField {
                    key: "password".to_string(),
                    label: "Password".to_string(),
                    placeholder: None,
                    help: None,
                    input_type: FieldInputType::Password,
                    required: true,
                    storage_targets: vec![namespaced_storage_target(&pack_id, "password")],
                    validation: None,
                },
            ],
        },
        ExtensionPackAuthMode::OAuth2External => match pack.auth.oauth2.as_ref() {
            Some(oauth) => {
                AuthMode::OAuth2AuthorizationCode(oauth_code_flow_from_pack(&pack_id, oauth))
            }
            None => AuthMode::Secrets {
                fields: pack_secret_fields(&pack_id, required),
            },
        },
    };

    let submit_label = match &mode {
        AuthMode::Secrets { .. } => "Save".to_string(),
        AuthMode::OAuth2AuthorizationCode(_) | AuthMode::Hybrid { .. } => {
            format!("Connect with {}", display_name)
        }
        AuthMode::OAuth2DeviceCode(_) => "Start device sign-in".to_string(),
    };
    let after = match &mode {
        AuthMode::Secrets { .. } => PostSubmitAfter::CloseAndResume,
        AuthMode::OAuth2AuthorizationCode(_) | AuthMode::Hybrid { .. } => {
            PostSubmitAfter::LaunchOAuth
        }
        AuthMode::OAuth2DeviceCode(_) => PostSubmitAfter::LaunchOAuth,
    };

    Some(IntegrationAuthManifest {
        integration_id: pack_id,
        display_name,
        description,
        docs_url: pack.docs_url.clone(),
        warning,
        mode,
        post_submit: PostSubmitAction {
            label: submit_label,
            after,
        },
    })
}

fn pack_secret_fields(pack_id: &str, required: &[String]) -> Vec<AuthField> {
    required
        .iter()
        .map(|key| {
            let key_trimmed = key.trim();
            let label = humanise_secret_key(key_trimmed);
            AuthField {
                key: key_trimmed.to_string(),
                label,
                placeholder: None,
                help: None,
                input_type: FieldInputType::Password,
                required: true,
                storage_targets: vec![namespaced_storage_target(pack_id, key_trimmed)],
                validation: None,
            }
        })
        .collect()
}

fn oauth_code_flow_from_pack(
    pack_id: &str,
    oauth: &crate::extension_packs::ExtensionPackOAuth2Spec,
) -> OAuth2CodeFlow {
    OAuth2CodeFlow {
        authorize_url: oauth.auth_url.clone(),
        token_url: oauth.token_url.clone(),
        scopes: oauth.scopes.clone(),
        pkce: oauth.use_pkce,
        client_id_source: SecretSlot(namespaced_storage_target(pack_id, "client_id")),
        client_secret_source: if oauth.client_secret.trim().is_empty() {
            None
        } else {
            Some(SecretSlot(namespaced_storage_target(
                pack_id,
                "client_secret",
            )))
        },
        token_storage: OAuthTokenStorage {
            access_token_key: namespaced_storage_target(pack_id, "access_token"),
            refresh_token_key: Some(namespaced_storage_target(pack_id, "refresh_token")),
            expires_at_key: Some(namespaced_storage_target(pack_id, "expires_at")),
        },
        extra_authorize_params: oauth
            .extra_auth_params
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    }
}

fn humanise_secret_key(key: &str) -> String {
    if key.is_empty() {
        return "Secret".to_string();
    }
    let mut out = String::with_capacity(key.len());
    for part in key.split(['_', '-', '.']) {
        if part.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
        }
        out.push_str(&chars.as_str().to_ascii_lowercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secrets_manifest(
        id: &str,
        field_key: &str,
        storage_targets: Vec<&str>,
    ) -> IntegrationAuthManifest {
        IntegrationAuthManifest {
            integration_id: id.to_string(),
            display_name: id.to_string(),
            description: None,
            docs_url: None,
            warning: None,
            mode: AuthMode::Secrets {
                fields: vec![AuthField {
                    key: field_key.to_string(),
                    label: field_key.to_string(),
                    placeholder: None,
                    help: None,
                    input_type: FieldInputType::Password,
                    required: true,
                    storage_targets: storage_targets
                        .into_iter()
                        .map(|value| value.to_string())
                        .collect(),
                    validation: None,
                }],
            },
            post_submit: PostSubmitAction::default(),
        }
    }

    #[test]
    fn namespaced_storage_target_applies_ext_prefix() {
        assert_eq!(
            namespaced_storage_target("jira_cloud", "api_token"),
            "ext.jira_cloud.api_token"
        );
    }

    #[test]
    fn raw_key_manifest_uses_literal_key_as_storage_target() {
        let manifest = raw_key_manifest("MY_SERVICE_TOKEN");
        assert_eq!(manifest.integration_id, RAW_KEY_MANIFEST_ID);
        let slots = manifest_form_storage_targets(&manifest);
        assert_eq!(slots, vec!["MY_SERVICE_TOKEN".to_string()]);
    }

    #[test]
    fn reverse_lookup_returns_unique_for_single_claim() {
        let pool = vec![
            secrets_manifest("github", "token", vec!["github_token"]),
            secrets_manifest("slack", "bot", vec!["slack_bot_token"]),
        ];
        let outcome = resolve_secret_key_among_manifests("github_token", &pool);
        match outcome {
            ReverseLookupOutcome::Unique(manifest) => {
                assert_eq!(manifest.integration_id, "github");
            }
            other => panic!("expected Unique, got {:?}", other),
        }
    }

    #[test]
    fn reverse_lookup_returns_ambiguous_when_multiple_manifests_claim_same_slot() {
        let pool = vec![
            secrets_manifest("pack_a", "client_id", vec!["client_id"]),
            secrets_manifest("pack_b", "client_id", vec!["client_id"]),
        ];
        let outcome = resolve_secret_key_among_manifests("client_id", &pool);
        match outcome {
            ReverseLookupOutcome::Ambiguous { key, candidates } => {
                assert_eq!(key, "client_id");
                assert_eq!(candidates.len(), 2);
            }
            other => panic!("expected Ambiguous, got {:?}", other),
        }
    }

    #[test]
    fn reverse_lookup_returns_none_for_unmapped_key() {
        let pool = vec![secrets_manifest("github", "token", vec!["github_token"])];
        let outcome = resolve_secret_key_among_manifests("RANDOM_KEY", &pool);
        assert_eq!(outcome, ReverseLookupOutcome::None);
    }

    #[test]
    fn manifest_all_storage_targets_includes_oauth_token_slots() {
        let manifest = IntegrationAuthManifest {
            integration_id: "demo".to_string(),
            display_name: "Demo".to_string(),
            description: None,
            docs_url: None,
            warning: None,
            mode: AuthMode::OAuth2AuthorizationCode(OAuth2CodeFlow {
                authorize_url: "https://example/authorize".to_string(),
                token_url: "https://example/token".to_string(),
                scopes: vec!["read".to_string()],
                pkce: true,
                client_id_source: SecretSlot("demo.client_id".to_string()),
                client_secret_source: Some(SecretSlot("demo.client_secret".to_string())),
                token_storage: OAuthTokenStorage {
                    access_token_key: "demo.access_token".to_string(),
                    refresh_token_key: Some("demo.refresh_token".to_string()),
                    expires_at_key: Some("demo.expires_at".to_string()),
                },
                extra_authorize_params: Vec::new(),
            }),
            post_submit: PostSubmitAction::default(),
        };
        let targets = manifest_all_storage_targets(&manifest);
        assert!(targets.contains(&"demo.access_token".to_string()));
        assert!(targets.contains(&"demo.refresh_token".to_string()));
        assert!(targets.contains(&"demo.expires_at".to_string()));
        assert!(targets.contains(&"demo.client_id".to_string()));
        assert!(targets.contains(&"demo.client_secret".to_string()));
    }

    #[test]
    fn manifest_round_trips_through_json() {
        let manifest = IntegrationAuthManifest {
            integration_id: "demo".to_string(),
            display_name: "Demo".to_string(),
            description: Some("desc".to_string()),
            docs_url: Some("https://example".to_string()),
            warning: None,
            mode: AuthMode::Secrets {
                fields: vec![AuthField {
                    key: "api_key".to_string(),
                    label: "API Key".to_string(),
                    placeholder: Some("sk-…".to_string()),
                    help: None,
                    input_type: FieldInputType::Password,
                    required: true,
                    storage_targets: vec!["demo_api_key".to_string()],
                    validation: Some(FieldValidation {
                        min_len: Some(16),
                        max_len: None,
                        must_start_with: Some("sk-".to_string()),
                    }),
                }],
            },
            post_submit: PostSubmitAction::default(),
        };
        let json = serde_json::to_string(&manifest).expect("serialize");
        let back: IntegrationAuthManifest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, manifest);
    }
}
