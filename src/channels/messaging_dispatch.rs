// The dispatch chain in this module is the "send a notification through a
// pack-declared channel" primitive. It is fully wired internally but has no
// call site yet inside `notify_user`/watcher routing. Removing the attribute
// and doing `cargo check` will enumerate every symbol that just went live —
// use that as a TODO list when plumbing dispatch into the main notify path.
//! HTTP template dispatcher for extension-pack-declared messaging channels.
//!
//! Bundled channels keep their existing Rust dispatch (nothing in this module
//! is used for them). For pack channels the flow is:
//!
//! 1. Registry hands us a [`crate::channels::messaging_registry::ChannelDescriptor`]
//!    whose `send_spec` carries url/body/headers/auth templates.
//! 2. We substitute placeholders (`{{text}}`, `{{to}}`, `{{conversation_id}}`,
//!    `{{secret:KEY}}`) against a [`DispatchInputs`] payload and the stored
//!    secret values. For JSON bodies the substitution is JSON-escaped so
//!    user-provided text can't break the JSON structure.
//! 3. Auth transport binding is applied to the `reqwest::RequestBuilder`.
//! 4. Request fires; a non-success status flips the channel to "failed" in
//!    the caller's error-handling path.
//!
//! This dispatcher is deliberately HTTP-only for v1. Non-HTTP transports
//! (SMTP, WebSocket, native plugins) are out of scope; the registry would
//! refuse to configure a channel that asked for them.

use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use crate::core::config::SecureConfigManager;
use crate::extension_packs::{AuthTransportBinding, MessagingHeaderSpec, MessagingSendSpec};

/// Minimal contract the dispatch path needs from any secret-backing store.
/// Keeps the module testable without a real [`SecureConfigManager`] and
/// leaves room for future stores (e.g. a remote secret broker) to plug in.
pub trait SecretReader: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
}

impl SecretReader for SecureConfigManager {
    fn get(&self, key: &str) -> Result<Option<String>> {
        self.get_custom_secret(key)
    }
}

/// What the agent / watcher gives the dispatcher to render into the template.
/// Keep this narrow so pack spec templates cannot reference anything outside
/// what we consciously expose.
#[derive(Debug, Clone, Default)]
pub struct DispatchInputs<'a> {
    pub text: &'a str,
    pub to: Option<&'a str>,
    pub conversation_id: Option<&'a str>,
    /// Optional subject line, useful for email-like channels. Falls back to
    /// the first line of `text` if omitted.
    pub subject: Option<&'a str>,
}

/// Result of a successful send. We don't return the provider's response body
/// to the caller by default (could contain provider-side tracking data we'd
/// rather not log). Status is preserved so the caller can branch on it.
#[derive(Debug, Clone)]
pub struct DispatchOutcome {
    pub http_status: u16,
}

/// Send one notification to a pack/custom declared HTTP channel, optionally
/// applying a resolved auth-profile overlay after template rendering.
pub async fn dispatch_pack_channel_with_overlay(
    _http_client: &reqwest::Client,
    secrets: &dyn SecretReader,
    send_spec: &MessagingSendSpec,
    inputs: &DispatchInputs<'_>,
    auth_overlay: Option<&crate::core::auth_profiles::HttpAuthOverlay>,
) -> Result<DispatchOutcome> {
    reject_literal_secrets_in_send_spec(send_spec)?;
    let content_type = send_spec
        .content_type
        .clone()
        .unwrap_or_else(|| "application/json".to_string());
    let json_body = content_type
        .trim()
        .to_ascii_lowercase()
        .starts_with("application/json");

    let rendered_url = render_template(
        &send_spec.url_template,
        inputs,
        secrets,
        // URL placeholders must be url-encoded after substitution; the
        // simple substitution here does NOT url-encode — pack authors must
        // pre-structure their urls so placeholders only appear where a
        // free-form string is valid (path segments, query values). JSON
        // escaping would mangle urls, so we switch it off on the url path.
        TemplateEscape::None,
    )?;
    let mut url = reqwest::Url::parse(&rendered_url).context("Rendered channel URL is invalid")?;
    if let Some(overlay) = auth_overlay {
        overlay.apply_to_url(&mut url);
    }
    let client = pinned_client_for_channel_url(&url).await?;

    let mut req = client
        .request(send_spec.method.as_reqwest(), url.clone())
        .timeout(Duration::from_secs(20));
    if let Some(overlay) = auth_overlay {
        req = overlay.apply_to_request_builder(req)?;
    }

    req = apply_content_type(req, &content_type);
    req = apply_static_headers(req, &send_spec.headers, inputs, secrets)?;
    req = apply_auth_binding(req, &send_spec.auth, inputs, secrets, json_body)?;

    if let Some(body_template) = &send_spec.body_template {
        let escape = if json_body {
            TemplateEscape::Json
        } else {
            TemplateEscape::None
        };
        let body = render_template(body_template, inputs, secrets, escape)?;
        req = req.body(body);
    }

    let response = req
        .send()
        .await
        .context("Failed to send HTTP request to channel endpoint")?;
    let status = response.status();
    let raw_status = status.as_u16();
    let ok = match &send_spec.expect_status {
        Some(codes) => codes.contains(&raw_status),
        None => status.is_success(),
    };
    if !ok {
        let body_preview = response
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(240)
            .collect::<String>();
        let safe_preview = crate::security::redact_secret_input(&body_preview).text;
        bail!(
            "Channel send failed: HTTP {}. Response preview: {}",
            raw_status,
            safe_preview
        );
    }
    Ok(DispatchOutcome {
        http_status: raw_status,
    })
}

fn reject_literal_secrets_in_send_spec(send_spec: &MessagingSendSpec) -> Result<()> {
    reject_literal_secret_template("URL template", &send_spec.url_template)?;
    if let Some(body) = send_spec.body_template.as_deref() {
        reject_literal_secret_template("body template", body)?;
    }
    for header in &send_spec.headers {
        reject_literal_secret_template("header value template", &header.value_template)?;
    }
    match &send_spec.auth {
        AuthTransportBinding::CustomHeader { value_template, .. }
        | AuthTransportBinding::QueryParam { value_template, .. } => {
            reject_literal_secret_template("auth value template", value_template)?;
        }
        AuthTransportBinding::None
        | AuthTransportBinding::Bearer { .. }
        | AuthTransportBinding::Basic { .. } => {}
    }
    Ok(())
}

fn reject_literal_secret_template(label: &str, raw: &str) -> Result<()> {
    let masked = mask_template_placeholders(raw);
    if crate::security::redact_secret_input(&masked).had_secret()
        || contains_opaque_literal_secret(&masked)
    {
        bail!(
            "{} contains secret-like literal material. Use {{{{secret:KEY}}}} placeholders instead.",
            label
        );
    }
    Ok(())
}

fn mask_template_placeholders(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '{' && chars.peek() == Some(&'{') {
            out.push(' ');
            out.push(' ');
            chars.next();
            let mut previous = '\0';
            while let Some(inner) = chars.next() {
                out.push(' ');
                if previous == '}' && inner == '}' {
                    break;
                }
                previous = inner;
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn contains_opaque_literal_secret(raw: &str) -> bool {
    for token in raw.split(|ch: char| {
        !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' || ch == '+')
    }) {
        let token = token.trim_matches('.');
        if token.chars().count() >= 20
            && token
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .count()
                >= 16
            && shannon_entropy_bits_per_char(token) >= 3.5
        {
            return true;
        }
    }
    false
}

fn shannon_entropy_bits_per_char(value: &str) -> f64 {
    let mut counts = BTreeMap::<char, usize>::new();
    let mut total = 0usize;
    for ch in value.chars() {
        total += 1;
        *counts.entry(ch).or_insert(0) += 1;
    }
    if total == 0 {
        return 0.0;
    }
    counts.values().fold(0.0, |entropy, count| {
        let p = *count as f64 / total as f64;
        entropy - p * p.log2()
    })
}

/// Validate a user-configured messaging endpoint without touching the network.
/// Dispatch performs the stronger DNS check immediately before sending.
pub fn validate_channel_url_static(url: &reqwest::Url) -> Result<()> {
    match url.scheme() {
        "http" | "https" => {}
        other => bail!(
            "Unsupported channel URL scheme '{}'. Use http or https.",
            other
        ),
    }
    let host = url
        .host_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Channel endpoint must include a host."))?;
    if !url.username().is_empty() || url.password().is_some() {
        bail!("Channel endpoint URL must not embed credentials.");
    }
    let normalized_host = host.trim_end_matches('.').to_ascii_lowercase();
    if normalized_host == "localhost" || normalized_host.ends_with(".localhost") {
        bail!("Channel endpoint cannot target localhost.");
    }
    if let Ok(ip) = normalized_host.parse::<IpAddr>() {
        ensure_public_channel_ip(ip)?;
    }
    Ok(())
}

async fn pinned_client_for_channel_url(url: &reqwest::Url) -> Result<reqwest::Client> {
    validate_channel_url_static(url)?;
    let host = url
        .host_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Channel endpoint must include a host."))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow::anyhow!("Channel endpoint must include a valid port."))?;
    let resolved = tokio::net::lookup_host((host, port))
        .await
        .context("Failed to resolve channel endpoint host")?
        .collect::<Vec<SocketAddr>>();
    if resolved.is_empty() {
        bail!("Channel endpoint host did not resolve to an address.");
    }
    for addr in &resolved {
        ensure_public_channel_ip(addr.ip())?;
    }
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &resolved)
        .build()
        .context("Failed to build pinned channel HTTP client")
}

fn ensure_public_channel_ip(ip: IpAddr) -> Result<()> {
    if channel_ip_is_public(ip) {
        Ok(())
    } else {
        bail!("Channel endpoint resolves to a private or local network address.")
    }
}

fn channel_ip_is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(addr) => {
            let [a, b, c, d] = addr.octets();
            if a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 255 && b == 255 && c == 255 && d == 255)
            {
                return false;
            }
            if a == 100 && (64..=127).contains(&b) {
                return false;
            }
            if a == 169 && b == 254 {
                return false;
            }
            if a == 172 && (16..=31).contains(&b) {
                return false;
            }
            if a == 192 && (b == 0 || b == 168) {
                return false;
            }
            if a == 198 && (b == 18 || b == 19) {
                return false;
            }
            if (a == 192 && b == 0 && c == 2)
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113)
            {
                return false;
            }
            true
        }
        IpAddr::V6(addr) => {
            let segments = addr.segments();
            let first = segments[0];
            if addr.is_unspecified() || addr.is_loopback() || addr.is_multicast() {
                return false;
            }
            if (first & 0xfe00) == 0xfc00 {
                return false;
            }
            if (first & 0xffc0) == 0xfe80 {
                return false;
            }
            if first == 0x2001 && segments[1] == 0x0db8 {
                return false;
            }
            true
        }
    }
}

/// Return every `{{secret:KEY}}` reference used by URL, body, headers, and auth
/// transport templates. This reads the declared shape only; it never resolves
/// secret values.
pub fn extract_secret_references(send_spec: &MessagingSendSpec) -> Vec<String> {
    let mut out = Vec::new();
    collect_secret_refs_from_template(&send_spec.url_template, &mut out);
    if let Some(body) = send_spec.body_template.as_ref() {
        collect_secret_refs_from_template(body, &mut out);
    }
    for header in &send_spec.headers {
        collect_secret_refs_from_template(&header.value_template, &mut out);
    }
    match &send_spec.auth {
        AuthTransportBinding::None => {}
        AuthTransportBinding::Bearer { secret_key } => push_unique(&mut out, secret_key),
        AuthTransportBinding::CustomHeader { value_template, .. }
        | AuthTransportBinding::QueryParam { value_template, .. } => {
            collect_secret_refs_from_template(value_template, &mut out);
        }
        AuthTransportBinding::Basic {
            username_key,
            password_key,
        } => {
            push_unique(&mut out, username_key);
            push_unique(&mut out, password_key);
        }
    }
    out
}

/// Rewrite logical secret references in a send spec to concrete storage slots.
/// Callers pass a mapping such as `api_key -> custom_messaging_channel:x:api_key`.
/// Existing fully-qualified keys may map to themselves.
pub fn rewrite_send_spec_secret_refs(
    send_spec: &MessagingSendSpec,
    mapping: &std::collections::BTreeMap<String, String>,
) -> MessagingSendSpec {
    let mut next = send_spec.clone();
    next.url_template = rewrite_template_secret_refs(&next.url_template, mapping);
    if let Some(body) = next.body_template.as_mut() {
        *body = rewrite_template_secret_refs(body, mapping);
    }
    for header in &mut next.headers {
        header.value_template = rewrite_template_secret_refs(&header.value_template, mapping);
    }
    next.auth = rewrite_auth_binding_secret_refs(&next.auth, mapping);
    next
}

fn rewrite_auth_binding_secret_refs(
    binding: &AuthTransportBinding,
    mapping: &std::collections::BTreeMap<String, String>,
) -> AuthTransportBinding {
    match binding {
        AuthTransportBinding::None => AuthTransportBinding::None,
        AuthTransportBinding::Bearer { secret_key } => AuthTransportBinding::Bearer {
            secret_key: map_secret_key(secret_key, mapping),
        },
        AuthTransportBinding::CustomHeader {
            name,
            value_template,
        } => AuthTransportBinding::CustomHeader {
            name: name.clone(),
            value_template: rewrite_template_secret_refs(value_template, mapping),
        },
        AuthTransportBinding::Basic {
            username_key,
            password_key,
        } => AuthTransportBinding::Basic {
            username_key: map_secret_key(username_key, mapping),
            password_key: map_secret_key(password_key, mapping),
        },
        AuthTransportBinding::QueryParam {
            name,
            value_template,
        } => AuthTransportBinding::QueryParam {
            name: name.clone(),
            value_template: rewrite_template_secret_refs(value_template, mapping),
        },
    }
}

fn map_secret_key(key: &str, mapping: &std::collections::BTreeMap<String, String>) -> String {
    let trimmed = key.trim();
    mapping
        .get(trimmed)
        .cloned()
        .unwrap_or_else(|| trimmed.to_string())
}

fn collect_secret_refs_from_template(template: &str, out: &mut Vec<String>) {
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(end) = find_close(template, i + 2) {
                let placeholder = &template[i + 2..end];
                let (_, key) = split_escape_prefix(placeholder);
                if let Some(secret_key) = key.strip_prefix("secret:") {
                    push_unique(out, secret_key.trim());
                }
                i = end + 2;
                continue;
            }
        }
        i += 1;
    }
}

fn rewrite_template_secret_refs(
    template: &str,
    mapping: &std::collections::BTreeMap<String, String>,
) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(end) = find_close(template, i + 2) {
                let placeholder = &template[i + 2..end];
                let (escape_prefix, key) = split_escape_prefix(placeholder);
                if let Some(secret_key) = key.strip_prefix("secret:") {
                    let mapped = map_secret_key(secret_key, mapping);
                    out.push_str("{{");
                    match escape_prefix {
                        Some(TemplateEscape::Json) => out.push_str("safe:"),
                        Some(TemplateEscape::None) => out.push_str("raw:"),
                        None => {}
                    }
                    out.push_str("secret:");
                    out.push_str(&mapped);
                    out.push_str("}}");
                    i = end + 2;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn push_unique(out: &mut Vec<String>, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    if !out.iter().any(|existing| existing == trimmed) {
        out.push(trimmed.to_string());
    }
}

fn apply_content_type(
    builder: reqwest::RequestBuilder,
    content_type: &str,
) -> reqwest::RequestBuilder {
    builder.header(reqwest::header::CONTENT_TYPE, content_type)
}

fn apply_static_headers(
    mut builder: reqwest::RequestBuilder,
    headers: &[MessagingHeaderSpec],
    inputs: &DispatchInputs<'_>,
    secrets: &dyn SecretReader,
) -> Result<reqwest::RequestBuilder> {
    for header in headers {
        let name = header.name.trim();
        if name.is_empty() {
            continue;
        }
        let value = render_template(
            &header.value_template,
            inputs,
            secrets,
            TemplateEscape::None,
        )?;
        builder = builder.header(name, value);
    }
    Ok(builder)
}

fn apply_auth_binding(
    builder: reqwest::RequestBuilder,
    binding: &AuthTransportBinding,
    inputs: &DispatchInputs<'_>,
    secrets: &dyn SecretReader,
    _json_body: bool,
) -> Result<reqwest::RequestBuilder> {
    match binding {
        AuthTransportBinding::None => Ok(builder),
        AuthTransportBinding::Bearer { secret_key } => {
            let token = read_secret(secrets, secret_key)?;
            Ok(builder.bearer_auth(token))
        }
        AuthTransportBinding::CustomHeader {
            name,
            value_template,
        } => {
            let value = render_template(value_template, inputs, secrets, TemplateEscape::None)?;
            Ok(builder.header(name.trim(), value))
        }
        AuthTransportBinding::Basic {
            username_key,
            password_key,
        } => {
            let user = read_secret(secrets, username_key)?;
            let pass = read_secret(secrets, password_key)?;
            Ok(builder.basic_auth(user, Some(pass)))
        }
        AuthTransportBinding::QueryParam {
            name,
            value_template,
        } => {
            let value = render_template(value_template, inputs, secrets, TemplateEscape::None)?;
            Ok(builder.query(&[(name.trim(), value.as_str())]))
        }
    }
}

fn read_secret(secrets: &dyn SecretReader, key: &str) -> Result<String> {
    let key = key.trim();
    if key.is_empty() {
        bail!("Auth transport references an empty secret key");
    }
    let value = secrets
        .get(key)
        .with_context(|| format!("Failed to load secret `{}`", key))?
        .unwrap_or_default();
    if value.trim().is_empty() {
        bail!(
            "Required secret `{}` is not set; channel is not yet configured.",
            key
        );
    }
    Ok(value)
}

/// How placeholder values should be escaped before being inserted into the
/// template. JSON-escape is only used for bodies with `application/json`
/// content-type so user-provided text cannot break the body structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateEscape {
    None,
    Json,
}

/// Render a template by substituting `{{…}}` placeholders.
///
/// Supported placeholders:
/// - `{{text}}` — the notification text.
/// - `{{to}}` — the optional recipient string.
/// - `{{conversation_id}}` — the originating conversation id.
/// - `{{subject}}` — the optional subject (falls back to first line of text).
/// - `{{secret:KEY}}` — current value of the named secret slot.
/// - `{{safe:text}}` / `{{safe:to}}` / etc — force JSON escape even when
///   the enclosing escape is `None`; useful for pack authors who need a
///   JSON-safe fragment inside a larger url-encoded body.
///
/// An unknown placeholder is treated as an empty string (after a
/// `tracing::warn!`) so a dispatch never panics on a typo in a pack spec.
pub fn render_template(
    template: &str,
    inputs: &DispatchInputs<'_>,
    secrets: &dyn SecretReader,
    default_escape: TemplateEscape,
) -> Result<String> {
    let mut out = String::with_capacity(template.len() + 32);
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(end) = find_close(template, i + 2) {
                let placeholder = &template[i + 2..end];
                let (force_escape, key) = split_escape_prefix(placeholder);
                let effective_escape = force_escape.unwrap_or(default_escape);
                let value = resolve_placeholder(key, inputs, secrets)?;
                let rendered = match effective_escape {
                    TemplateEscape::Json => json_escape(&value),
                    TemplateEscape::None => value,
                };
                out.push_str(&rendered);
                i = end + 2;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    Ok(out)
}

fn find_close(template: &str, from: usize) -> Option<usize> {
    let bytes = template.as_bytes();
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'}' && bytes[i + 1] == b'}' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn split_escape_prefix(raw: &str) -> (Option<TemplateEscape>, &str) {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("safe:") {
        (Some(TemplateEscape::Json), rest.trim())
    } else if let Some(rest) = trimmed.strip_prefix("raw:") {
        (Some(TemplateEscape::None), rest.trim())
    } else {
        (None, trimmed)
    }
}

fn resolve_placeholder(
    key: &str,
    inputs: &DispatchInputs<'_>,
    secrets: &dyn SecretReader,
) -> Result<String> {
    if let Some(secret_key) = key.strip_prefix("secret:") {
        return read_secret(secrets, secret_key.trim());
    }
    Ok(match key {
        "text" => inputs.text.to_string(),
        "to" => inputs.to.unwrap_or("").to_string(),
        "conversation_id" => inputs.conversation_id.unwrap_or("").to_string(),
        "subject" => match inputs.subject {
            Some(subject) => subject.to_string(),
            None => inputs
                .text
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(120)
                .collect(),
        },
        other => {
            tracing::warn!(
                "messaging_dispatch: unknown placeholder `{{{{{}}}}}`; substituting empty string",
                other
            );
            String::new()
        }
    })
}

fn json_escape(value: &str) -> String {
    let json = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string());
    // `serde_json::to_string` wraps the value in surrounding quotes. Strip
    // them — callers place the placeholder inside their own quotes in the
    // template, same convention as Slack/Discord examples.
    if json.starts_with('"') && json.ends_with('"') && json.len() >= 2 {
        json[1..json.len() - 1].to_string()
    } else {
        json
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension_packs::MessagingHeaderSpec;
    use std::collections::HashMap;

    /// Minimal in-memory [`SecretReader`] used only by unit tests. Avoids a
    /// filesystem dependency on [`SecureConfigManager`].
    struct InMemorySecrets(HashMap<String, String>);

    impl SecretReader for InMemorySecrets {
        fn get(&self, key: &str) -> Result<Option<String>> {
            Ok(self.0.get(key).cloned())
        }
    }

    fn empty_cfg() -> InMemorySecrets {
        InMemorySecrets(HashMap::new())
    }

    fn cfg_with(pairs: &[(&str, &str)]) -> InMemorySecrets {
        let mut map = HashMap::new();
        for (k, v) in pairs {
            map.insert((*k).to_string(), (*v).to_string());
        }
        InMemorySecrets(map)
    }

    #[test]
    fn render_substitutes_plain_placeholders() {
        let cfg = empty_cfg();
        let inputs = DispatchInputs {
            text: "hello",
            to: Some("room-1"),
            conversation_id: Some("conv-42"),
            subject: None,
        };
        let out = render_template(
            "u={{to}} c={{conversation_id}} m={{text}}",
            &inputs,
            &cfg,
            TemplateEscape::None,
        )
        .expect("render");
        assert_eq!(out, "u=room-1 c=conv-42 m=hello");
    }

    #[test]
    fn render_json_escapes_for_json_body() {
        let cfg = empty_cfg();
        let inputs = DispatchInputs {
            text: "hi \"world\"\nnew",
            to: None,
            conversation_id: None,
            subject: None,
        };
        let out = render_template(
            "{\"msg\":\"{{text}}\"}",
            &inputs,
            &cfg,
            TemplateEscape::Json,
        )
        .expect("render");
        assert!(out.contains("\\\""));
        assert!(out.contains("\\n"));
        assert!(serde_json::from_str::<serde_json::Value>(&out).is_ok());
    }

    #[test]
    fn render_unknown_placeholder_is_empty_and_logged() {
        let cfg = empty_cfg();
        let inputs = DispatchInputs::default();
        let out = render_template(
            "before-{{bogus}}-after",
            &inputs,
            &cfg,
            TemplateEscape::None,
        )
        .expect("render");
        assert_eq!(out, "before--after");
    }

    #[test]
    fn render_secret_placeholder_reads_from_secrets() {
        let cfg = cfg_with(&[("webhook_url", "https://example/x")]);
        let inputs = DispatchInputs::default();
        let out = render_template(
            "target={{secret:webhook_url}}",
            &inputs,
            &cfg,
            TemplateEscape::None,
        )
        .expect("render");
        assert_eq!(out, "target=https://example/x");
    }

    #[test]
    fn render_secret_missing_returns_error() {
        let cfg = empty_cfg();
        let inputs = DispatchInputs::default();
        let err = render_template(
            "{{secret:missing_key}}",
            &inputs,
            &cfg,
            TemplateEscape::None,
        )
        .expect_err("should fail on missing secret");
        assert!(err.to_string().contains("missing_key"));
    }

    #[test]
    fn render_safe_prefix_forces_json_escape_even_in_plain_context() {
        let cfg = empty_cfg();
        let inputs = DispatchInputs {
            text: "a \"quoted\" thing",
            ..DispatchInputs::default()
        };
        let plain =
            render_template("{{text}}", &inputs, &cfg, TemplateEscape::None).expect("render");
        assert!(plain.contains('"'));

        let escaped =
            render_template("{{safe:text}}", &inputs, &cfg, TemplateEscape::None).expect("render");
        assert!(!escaped.contains("\"quoted\""));
        assert!(escaped.contains("\\\"quoted\\\""));
    }

    #[test]
    fn header_spec_substitution_uses_templates() {
        // Smoke-test apply_static_headers via the template path it delegates
        // to. A real HTTP call is covered by integration tests.
        let headers = vec![MessagingHeaderSpec {
            name: "X-Trace".to_string(),
            value_template: "conv={{conversation_id}}".to_string(),
        }];
        assert_eq!(headers[0].name, "X-Trace");
        let cfg = empty_cfg();
        let inputs = DispatchInputs {
            conversation_id: Some("abc"),
            ..DispatchInputs::default()
        };
        let rendered = render_template(
            &headers[0].value_template,
            &inputs,
            &cfg,
            TemplateEscape::None,
        )
        .expect("render");
        assert_eq!(rendered, "conv=abc");
    }

    #[test]
    fn static_url_validation_rejects_local_and_private_targets() {
        for raw in [
            "http://localhost/admin",
            "http://127.0.0.1/admin",
            "http://169.254.169.254/latest/meta-data",
            "http://10.0.0.5/hook",
        ] {
            let url = reqwest::Url::parse(raw).expect("url");
            assert!(
                validate_channel_url_static(&url).is_err(),
                "{raw} should be rejected"
            );
        }
    }

    #[test]
    fn static_url_validation_rejects_embedded_credentials() {
        let url = reqwest::Url::parse("https://user:pass@example.com/hook").expect("url");
        let error = validate_channel_url_static(&url).expect_err("embedded credentials");
        assert!(error.to_string().contains("must not embed credentials"));
    }

    #[test]
    fn static_url_validation_allows_public_https_target() {
        let url = reqwest::Url::parse("https://example.com/hook").expect("url");
        validate_channel_url_static(&url).expect("public https should pass static validation");
    }

    #[test]
    fn dispatch_template_guard_rejects_literal_secret_material() {
        let send = MessagingSendSpec {
            body_template: Some("token=2skdjfkj2wlfrj23kr2rlm&text={{text}}".to_string()),
            ..MessagingSendSpec::default()
        };
        let error =
            reject_literal_secrets_in_send_spec(&send).expect_err("literal token should fail");
        assert!(error.to_string().contains("secret-like literal material"));
    }

    #[test]
    fn dispatch_template_guard_allows_secret_placeholders() {
        let send = MessagingSendSpec {
            url_template: "{{secret:webhook_url}}".to_string(),
            body_template: Some("text={{text}}".to_string()),
            ..MessagingSendSpec::default()
        };
        reject_literal_secrets_in_send_spec(&send).expect("placeholder use should pass");
    }
}
