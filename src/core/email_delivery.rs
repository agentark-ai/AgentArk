use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use lettre::message::{header, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use reqwest::Url;
use ring::hmac;
use sha2::{Digest, Sha256};
use std::str::FromStr;

use crate::core::config::EmailConfig;

pub const EMAIL_PROVIDER_AUTO: &str = "auto";
pub const EMAIL_PROVIDER_GMAIL: &str = "gmail";
pub const EMAIL_PROVIDER_GOOGLE_WORKSPACE: &str = "google_workspace";
pub const EMAIL_PROVIDER_RESEND: &str = "resend";
pub const EMAIL_PROVIDER_POSTMARK: &str = "postmark";
pub const EMAIL_PROVIDER_SES: &str = "ses";
pub const EMAIL_PROVIDER_SMTP: &str = "smtp";

pub const EMAIL_TRANSPORT_HTTP: &str = "http";
pub const EMAIL_TRANSPORT_SMTP: &str = "smtp";

pub const EMAIL_AUTH_NONE: &str = "none";
pub const EMAIL_AUTH_BEARER: &str = "bearer";
pub const EMAIL_AUTH_HEADER: &str = "header";
pub const EMAIL_AUTH_BASIC: &str = "basic";
pub const EMAIL_AUTH_AWS_SIGV4: &str = "aws_sigv4";

#[derive(Debug, Clone)]
pub struct RenderedNotificationEmail {
    pub subject: String,
    pub text_body: String,
    pub html_body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExternalEmailProviderKind {
    Resend,
    Postmark,
    Ses,
    Smtp,
}

#[derive(Debug, Clone)]
struct ResolvedExternalEmailProvider {
    provider_id: String,
    kind: ExternalEmailProviderKind,
    from_address: String,
    base_url: Option<String>,
    send_path: Option<String>,
    auth_kind: String,
    auth_header_name: Option<String>,
    auth_scheme: Option<String>,
    api_key: String,
    basic_username: String,
    basic_password: String,
    aws_access_key_id: String,
    aws_secret_access_key: String,
    aws_session_token: Option<String>,
    aws_region: Option<String>,
    aws_service: String,
    smtp_host: String,
    smtp_port: u16,
    smtp_security: String,
}

fn normalize_identifier(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

fn configured_secret(value: &str) -> bool {
    !value.trim().is_empty() && value != "[ENCRYPTED]"
}

fn trimmed_or_none(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn normalize_email_format(value: Option<&str>) -> String {
    normalize_identifier(value.unwrap_or_default())
}

fn normalize_smtp_security(value: &str) -> String {
    match normalize_identifier(value).as_str() {
        "tls" | "ssl" | "smtps" => "tls".to_string(),
        "none" | "plain" | "insecure" => "none".to_string(),
        _ => "starttls".to_string(),
    }
}

fn message_content_for_format(message: &str, email_format: Option<&str>) -> String {
    match normalize_email_format(email_format).as_str() {
        "narrative" => message
            .lines()
            .map(|line| line.trim().trim_start_matches("- ").trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        "sections" => format!("Summary\n\n{}", message.trim()),
        _ => message.trim().to_string(),
    }
}

fn render_html_message_body(message: &str) -> String {
    let paragraphs = message
        .split("\n\n")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            let with_breaks = html_escape(part).replace('\n', "<br/>");
            format!(
                "<p style=\"margin:0 0 16px;color:#1f2937;font-size:15px;line-height:1.7;\">{}</p>",
                with_breaks
            )
        })
        .collect::<Vec<_>>();
    if paragraphs.is_empty() {
        "<p style=\"margin:0;color:#1f2937;font-size:15px;line-height:1.7;\">No content.</p>"
            .to_string()
    } else {
        paragraphs.join("")
    }
}

pub fn normalize_email_provider(value: &str) -> String {
    match normalize_identifier(value).as_str() {
        "" | "auto" | "default" => EMAIL_PROVIDER_AUTO.to_string(),
        "gmail" | "google_mail" => EMAIL_PROVIDER_GMAIL.to_string(),
        "workspace" | "gws" | "googleworkspace" | "google_workspace" => {
            EMAIL_PROVIDER_GOOGLE_WORKSPACE.to_string()
        }
        "resend" => EMAIL_PROVIDER_RESEND.to_string(),
        "postmark" => EMAIL_PROVIDER_POSTMARK.to_string(),
        "ses" | "aws_ses" | "amazon_ses" => EMAIL_PROVIDER_SES.to_string(),
        "smtp" => EMAIL_PROVIDER_SMTP.to_string(),
        other => other.to_string(),
    }
}

pub fn normalize_transport_kind(value: &str) -> String {
    match normalize_identifier(value).as_str() {
        EMAIL_TRANSPORT_SMTP => EMAIL_TRANSPORT_SMTP.to_string(),
        _ => EMAIL_TRANSPORT_HTTP.to_string(),
    }
}

pub fn normalize_auth_kind(value: &str) -> String {
    match normalize_identifier(value).as_str() {
        "token" | "bearer" => EMAIL_AUTH_BEARER.to_string(),
        "header" | "api_key" | "apikey" => EMAIL_AUTH_HEADER.to_string(),
        "basic" | "password" => EMAIL_AUTH_BASIC.to_string(),
        "aws" | "sigv4" | "aws_sigv4" => EMAIL_AUTH_AWS_SIGV4.to_string(),
        _ => EMAIL_AUTH_NONE.to_string(),
    }
}

pub fn normalize_email_backend_selection(
    provider_value: &str,
    available_backends: &[String],
) -> Result<String> {
    let provider = normalize_email_provider(provider_value);
    if provider == EMAIL_PROVIDER_AUTO {
        return match available_backends {
            [only] => Ok(only.clone()),
            [] => Err(anyhow!(
                "Email delivery is not ready. Connect Gmail/Google Workspace or configure an external email provider."
            )),
            _ => Err(anyhow!(
                "Email provider is set to auto, but multiple backends are ready. Set email.provider explicitly."
            )),
        };
    }
    if available_backends
        .iter()
        .any(|backend| backend == &provider)
    {
        Ok(provider)
    } else {
        Err(anyhow!("Email provider '{}' is not ready yet.", provider))
    }
}

pub fn email_channel_is_ready(provider_value: &str, available_backends: &[String]) -> bool {
    normalize_email_backend_selection(provider_value, available_backends).is_ok()
}

pub fn validate_email_address(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("Email address cannot be empty");
    }
    Address::from_str(trimmed)
        .map(|_| trimmed.to_string())
        .map_err(|error| anyhow!("Invalid email address '{}': {}", trimmed, error))
}

pub fn validate_optional_email_address(value: Option<&str>) -> Result<Option<String>> {
    match trimmed_or_none(value) {
        Some(address) => Ok(Some(validate_email_address(&address)?)),
        None => Ok(None),
    }
}

pub fn render_notification_email(
    agent_name: &str,
    subject: &str,
    message: &str,
    generated_at: Option<&str>,
    email_format: Option<&str>,
) -> RenderedNotificationEmail {
    let generated_at = generated_at
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    let content = message_content_for_format(message, email_format);
    let text_body =
        format!("{content}\n\nGenerated at: {generated_at}\nSent by AgentArk for {agent_name}");
    let html_body = format!(
        concat!(
            "<!doctype html><html><head><meta charset=\"utf-8\"/>",
            "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"/>",
            "<title>{title}</title></head>",
            "<body style=\"margin:0;padding:0;background:#eef2f7;font-family:Arial,Helvetica,sans-serif;\">",
            "<div style=\"display:none;max-height:0;overflow:hidden;opacity:0;\">{preview}</div>",
            "<table role=\"presentation\" width=\"100%\" cellspacing=\"0\" cellpadding=\"0\" style=\"background:#eef2f7;padding:24px 12px;\">",
            "<tr><td align=\"center\">",
            "<table role=\"presentation\" width=\"100%\" cellspacing=\"0\" cellpadding=\"0\" style=\"max-width:680px;background:#ffffff;border:1px solid #d1d5db;border-top:6px solid #f59e0b;\">",
            "<tr><td style=\"padding:28px 32px 12px;background:#0f766e;\">",
            "<div style=\"font-size:12px;letter-spacing:1px;text-transform:uppercase;color:#ccfbf1;\">AgentArk</div>",
            "<h1 style=\"margin:10px 0 0;font-size:28px;line-height:1.2;color:#ffffff;\">{title}</h1>",
            "</td></tr>",
            "<tr><td style=\"padding:28px 32px 8px;\">",
            "<div style=\"margin:0 0 18px;color:#4b5563;font-size:13px;line-height:1.5;\">Sent by <strong style=\"color:#111827;\">{agent_name}</strong> on {generated_at}</div>",
            "{content_html}",
            "</td></tr>",
            "<tr><td style=\"padding:0 32px 28px;\">",
            "<div style=\"padding-top:18px;border-top:1px solid #e5e7eb;color:#6b7280;font-size:12px;line-height:1.6;\">",
            "This email was generated by AgentArk and delivered through the configured email provider.",
            "</div></td></tr></table></td></tr></table></body></html>"
        ),
        title = html_escape(subject),
        preview = html_escape(&content),
        agent_name = html_escape(agent_name),
        generated_at = html_escape(&generated_at),
        content_html = render_html_message_body(&content),
    );
    RenderedNotificationEmail {
        subject: subject.to_string(),
        text_body,
        html_body,
    }
}

pub fn build_email_message(
    from_address: &str,
    to_address: &str,
    subject: &str,
    text_body: &str,
    html_body: Option<&str>,
    from_name: Option<&str>,
) -> Result<Message> {
    let from_address = validate_email_address(from_address)?;
    let to_address = validate_email_address(to_address)?;
    let from = Mailbox::new(
        from_name
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        from_address.parse()?,
    );
    let to = Mailbox::new(None, to_address.parse()?);
    let builder = Message::builder().from(from).to(to).subject(subject);
    let plain = SinglePart::builder()
        .header(header::ContentType::TEXT_PLAIN)
        .body(text_body.to_string());
    if let Some(html_body) = html_body.filter(|value| !value.trim().is_empty()) {
        let html = SinglePart::builder()
            .header(header::ContentType::TEXT_HTML)
            .body(html_body.to_string());
        Ok(builder.multipart(MultiPart::alternative().singlepart(plain).singlepart(html))?)
    } else {
        Ok(builder.singlepart(plain)?)
    }
}

pub fn external_email_provider_id(config: &EmailConfig) -> Option<String> {
    let provider = normalize_email_provider(&config.provider);
    if provider == EMAIL_PROVIDER_GMAIL || provider == EMAIL_PROVIDER_GOOGLE_WORKSPACE {
        return None;
    }
    let transport_kind = normalize_transport_kind(&config.transport.kind);
    if provider == EMAIL_PROVIDER_AUTO {
        if transport_kind == EMAIL_TRANSPORT_SMTP {
            return Some(EMAIL_PROVIDER_SMTP.to_string());
        }
        return None;
    }
    Some(provider)
}

fn resolve_external_email_provider(config: &EmailConfig) -> Result<ResolvedExternalEmailProvider> {
    let provider_id = external_email_provider_id(config)
        .ok_or_else(|| anyhow!("No external email provider is configured"))?;
    let auth = &config.auth;
    let transport = &config.transport;
    let transport_kind = if provider_id == EMAIL_PROVIDER_SMTP {
        EMAIL_TRANSPORT_SMTP.to_string()
    } else {
        normalize_transport_kind(&transport.kind)
    };
    let kind = match provider_id.as_str() {
        EMAIL_PROVIDER_RESEND => ExternalEmailProviderKind::Resend,
        EMAIL_PROVIDER_POSTMARK => ExternalEmailProviderKind::Postmark,
        EMAIL_PROVIDER_SES => ExternalEmailProviderKind::Ses,
        EMAIL_PROVIDER_SMTP => ExternalEmailProviderKind::Smtp,
        other if transport_kind == EMAIL_TRANSPORT_SMTP => {
            let _ = other;
            ExternalEmailProviderKind::Smtp
        }
        other => bail!("Unsupported external email provider '{}'", other),
    };
    let from_address = validate_optional_email_address(config.from_address.as_deref())?
        .ok_or_else(|| anyhow!("email.from_address is required for external email delivery"))?;
    let auth_kind = match kind {
        ExternalEmailProviderKind::Resend => {
            let normalized = normalize_auth_kind(&auth.kind);
            if normalized == EMAIL_AUTH_NONE {
                EMAIL_AUTH_BEARER.to_string()
            } else {
                normalized
            }
        }
        ExternalEmailProviderKind::Postmark => {
            let normalized = normalize_auth_kind(&auth.kind);
            if normalized == EMAIL_AUTH_NONE {
                EMAIL_AUTH_HEADER.to_string()
            } else {
                normalized
            }
        }
        ExternalEmailProviderKind::Ses => {
            let normalized = normalize_auth_kind(&auth.kind);
            if normalized == EMAIL_AUTH_NONE {
                EMAIL_AUTH_AWS_SIGV4.to_string()
            } else {
                normalized
            }
        }
        ExternalEmailProviderKind::Smtp => {
            let normalized = normalize_auth_kind(&auth.kind);
            if normalized == EMAIL_AUTH_NONE {
                EMAIL_AUTH_BASIC.to_string()
            } else {
                normalized
            }
        }
    };
    let provider = ResolvedExternalEmailProvider {
        provider_id,
        kind,
        from_address,
        base_url: trimmed_or_none(transport.http.base_url.as_deref()),
        send_path: trimmed_or_none(transport.http.send_path.as_deref()),
        auth_kind,
        auth_header_name: trimmed_or_none(auth.header_name.as_deref()),
        auth_scheme: trimmed_or_none(auth.scheme.as_deref()),
        api_key: auth.api_key.clone(),
        basic_username: auth.basic_username.clone(),
        basic_password: auth.basic_password.clone(),
        aws_access_key_id: auth.aws_access_key_id.clone(),
        aws_secret_access_key: auth.aws_secret_access_key.clone(),
        aws_session_token: trimmed_or_none(auth.aws_session_token.as_deref()),
        aws_region: trimmed_or_none(auth.aws_region.as_deref()),
        aws_service: trimmed_or_none(auth.aws_service.as_deref())
            .unwrap_or_else(|| "ses".to_string()),
        smtp_host: transport.smtp.host.clone(),
        smtp_port: transport.smtp.port,
        smtp_security: normalize_smtp_security(&transport.smtp.security),
    };

    match provider.kind {
        ExternalEmailProviderKind::Resend | ExternalEmailProviderKind::Postmark => {
            if !configured_secret(&provider.api_key) {
                bail!(
                    "email.auth.api_key is required for {}",
                    provider.provider_id
                );
            }
        }
        ExternalEmailProviderKind::Ses => {
            if provider.auth_kind == EMAIL_AUTH_AWS_SIGV4 {
                if !configured_secret(&provider.aws_access_key_id)
                    || !configured_secret(&provider.aws_secret_access_key)
                {
                    bail!(
                        "SES HTTP delivery requires email.auth.aws_access_key_id and email.auth.aws_secret_access_key"
                    );
                }
                if provider.aws_region.is_none() {
                    bail!("SES delivery requires email.auth.aws_region");
                }
            } else {
                if provider.smtp_host.trim().is_empty() {
                    bail!("SES SMTP delivery requires email.transport.smtp.host");
                }
                if provider.auth_kind == EMAIL_AUTH_BASIC
                    && (!configured_secret(&provider.basic_username)
                        || !configured_secret(&provider.basic_password))
                {
                    bail!(
                        "SES SMTP delivery requires email.auth.basic_username and email.auth.basic_password"
                    );
                }
            }
        }
        ExternalEmailProviderKind::Smtp => {
            if provider.smtp_host.trim().is_empty() {
                bail!("SMTP delivery requires email.transport.smtp.host");
            }
            if provider.auth_kind == EMAIL_AUTH_BASIC
                && (!configured_secret(&provider.basic_username)
                    || !configured_secret(&provider.basic_password))
            {
                bail!(
                    "SMTP delivery requires email.auth.basic_username and email.auth.basic_password"
                );
            }
        }
    }
    Ok(provider)
}

pub fn external_email_delivery_is_ready(config: &EmailConfig) -> bool {
    resolve_external_email_provider(config).is_ok()
}

fn join_url(base_url: &str, send_path: &str) -> Result<Url> {
    let base = base_url.trim().trim_end_matches('/');
    let path = if send_path.trim().starts_with('/') {
        send_path.trim().to_string()
    } else {
        format!("/{}", send_path.trim())
    };
    Url::parse(&format!("{}{}", base, path))
        .with_context(|| format!("Invalid email transport URL '{}{}'", base, path))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let key = hmac::Key::new(hmac::HMAC_SHA256, key);
    hmac::sign(&key, data).as_ref().to_vec()
}

async fn send_resend_email(
    provider: &ResolvedExternalEmailProvider,
    recipient: &str,
    email: &RenderedNotificationEmail,
) -> Result<()> {
    let request_url = join_url(
        provider
            .base_url
            .as_deref()
            .unwrap_or("https://api.resend.com"),
        provider.send_path.as_deref().unwrap_or("/emails"),
    )?;
    let auth_header = match provider.auth_kind.as_str() {
        EMAIL_AUTH_BEARER => format!(
            "{} {}",
            provider.auth_scheme.as_deref().unwrap_or("Bearer"),
            provider.api_key
        ),
        EMAIL_AUTH_HEADER | EMAIL_AUTH_NONE => provider.api_key.clone(),
        other => bail!("Unsupported Resend auth kind '{}'", other),
    };
    let request = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?
        .post(request_url)
        .header(
            provider
                .auth_header_name
                .as_deref()
                .unwrap_or("Authorization"),
            auth_header,
        )
        .json(&serde_json::json!({
            "from": provider.from_address.as_str(),
            "to": [recipient],
            "subject": email.subject.as_str(),
            "html": email.html_body.as_str(),
            "text": email.text_body.as_str(),
        }))
        .send()
        .await?;
    if request.status().is_success() {
        Ok(())
    } else {
        let status = request.status();
        let body = request.text().await.unwrap_or_default();
        Err(anyhow!("Resend send failed ({}): {}", status, body.trim()))
    }
}

async fn send_postmark_email(
    provider: &ResolvedExternalEmailProvider,
    recipient: &str,
    email: &RenderedNotificationEmail,
) -> Result<()> {
    let request_url = join_url(
        provider
            .base_url
            .as_deref()
            .unwrap_or("https://api.postmarkapp.com"),
        provider.send_path.as_deref().unwrap_or("/email"),
    )?;
    let header_name = provider.auth_header_name.as_deref().unwrap_or(
        if provider.auth_kind == EMAIL_AUTH_BEARER {
            "Authorization"
        } else {
            "X-Postmark-Server-Token"
        },
    );
    let header_value = if provider.auth_kind == EMAIL_AUTH_BEARER {
        format!(
            "{} {}",
            provider.auth_scheme.as_deref().unwrap_or("Bearer"),
            provider.api_key
        )
    } else {
        provider.api_key.clone()
    };
    let request = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?
        .post(request_url)
        .header(header_name, header_value)
        .json(&serde_json::json!({
            "From": provider.from_address.as_str(),
            "To": recipient,
            "Subject": email.subject.as_str(),
            "HtmlBody": email.html_body.as_str(),
            "TextBody": email.text_body.as_str(),
        }))
        .send()
        .await?;
    if request.status().is_success() {
        Ok(())
    } else {
        let status = request.status();
        let body = request.text().await.unwrap_or_default();
        Err(anyhow!(
            "Postmark send failed ({}): {}",
            status,
            body.trim()
        ))
    }
}

async fn send_ses_http_email(
    provider: &ResolvedExternalEmailProvider,
    recipient: &str,
    email: &RenderedNotificationEmail,
) -> Result<()> {
    let region = provider
        .aws_region
        .as_deref()
        .ok_or_else(|| anyhow!("SES delivery requires email.auth.aws_region"))?;
    let request_url = join_url(
        provider
            .base_url
            .as_deref()
            .unwrap_or(&format!("https://email.{}.amazonaws.com", region)),
        provider
            .send_path
            .as_deref()
            .unwrap_or("/v2/email/outbound-emails"),
    )?;
    let host = request_url
        .host_str()
        .ok_or_else(|| anyhow!("SES request URL must include a host"))?;
    let canonical_uri = if request_url.path().is_empty() {
        "/"
    } else {
        request_url.path()
    };
    let canonical_query = request_url.query().unwrap_or_default();
    let payload = serde_json::json!({
        "FromEmailAddress": provider.from_address.as_str(),
        "Destination": { "ToAddresses": [recipient] },
        "Content": {
            "Simple": {
                "Subject": { "Data": email.subject.as_str(), "Charset": "UTF-8" },
                "Body": {
                    "Text": { "Data": email.text_body.as_str(), "Charset": "UTF-8" },
                    "Html": { "Data": email.html_body.as_str(), "Charset": "UTF-8" }
                }
            }
        }
    });
    let payload_text = serde_json::to_string(&payload)?;
    let payload_hash = sha256_hex(payload_text.as_bytes());
    let now = Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();
    let mut canonical_headers = vec![
        ("content-type", "application/json".to_string()),
        ("host", host.to_string()),
        ("x-amz-content-sha256", payload_hash.clone()),
        ("x-amz-date", amz_date.clone()),
    ];
    if let Some(session_token) = provider.aws_session_token.as_ref() {
        canonical_headers.push(("x-amz-security-token", session_token.clone()));
    }
    canonical_headers.sort_by(|left, right| left.0.cmp(right.0));
    let canonical_headers_text = canonical_headers
        .iter()
        .map(|(name, value)| format!("{}:{}\n", name, value.trim()))
        .collect::<String>();
    let signed_headers = canonical_headers
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(";");
    let canonical_request = format!(
        "POST\n{}\n{}\n{}\n{}\n{}",
        canonical_uri, canonical_query, canonical_headers_text, signed_headers, payload_hash
    );
    let credential_scope = format!(
        "{}/{}/{}/aws4_request",
        date_stamp, region, provider.aws_service
    );
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        amz_date,
        credential_scope,
        sha256_hex(canonical_request.as_bytes())
    );
    let k_date = hmac_sha256(
        format!("AWS4{}", provider.aws_secret_access_key).as_bytes(),
        date_stamp.as_bytes(),
    );
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, provider.aws_service.as_bytes());
    let signing_key = hmac_sha256(&k_service, b"aws4_request");
    let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
        provider.aws_access_key_id, credential_scope, signed_headers, signature
    );
    let mut request = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?
        .post(request_url)
        .header("content-type", "application/json")
        .header("x-amz-content-sha256", payload_hash)
        .header("x-amz-date", amz_date)
        .header("authorization", authorization)
        .body(payload_text);
    if let Some(session_token) = provider.aws_session_token.as_ref() {
        request = request.header("x-amz-security-token", session_token);
    }
    let response = request.send().await?;
    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        Err(anyhow!("SES send failed ({}): {}", status, body.trim()))
    }
}

async fn send_smtp_email(
    provider: &ResolvedExternalEmailProvider,
    recipient: &str,
    email: &RenderedNotificationEmail,
) -> Result<()> {
    let message = build_email_message(
        &provider.from_address,
        recipient,
        &email.subject,
        &email.text_body,
        Some(&email.html_body),
        Some("AgentArk"),
    )?;
    let transport_builder = match provider.smtp_security.as_str() {
        "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(provider.smtp_host.trim())?,
        "none" => {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(provider.smtp_host.trim())
        }
        _ => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(provider.smtp_host.trim())?,
    };
    let transport_builder = transport_builder.port(provider.smtp_port);
    let transport = if provider.auth_kind == EMAIL_AUTH_BASIC {
        transport_builder
            .credentials(Credentials::new(
                provider.basic_username.clone(),
                provider.basic_password.clone(),
            ))
            .build()
    } else {
        transport_builder.build()
    };
    transport.send(message).await.map(|_| ()).map_err(|error| {
        anyhow!(
            "SMTP send failed via {}:{}: {}",
            provider.smtp_host,
            provider.smtp_port,
            error
        )
    })
}

pub async fn send_external_email(
    config: &EmailConfig,
    email: &RenderedNotificationEmail,
    recipient: &str,
) -> Result<()> {
    let recipient = validate_email_address(recipient)?;
    let provider = resolve_external_email_provider(config)?;
    match provider.kind {
        ExternalEmailProviderKind::Resend => send_resend_email(&provider, &recipient, email).await,
        ExternalEmailProviderKind::Postmark => {
            send_postmark_email(&provider, &recipient, email).await
        }
        ExternalEmailProviderKind::Ses => {
            if provider.auth_kind == EMAIL_AUTH_BASIC
                || normalize_transport_kind(&config.transport.kind) == EMAIL_TRANSPORT_SMTP
            {
                send_smtp_email(&provider, &recipient, email).await
            } else {
                send_ses_http_email(&provider, &recipient, email).await
            }
        }
        ExternalEmailProviderKind::Smtp => send_smtp_email(&provider, &recipient, email).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_template_renders_html_and_text() {
        let rendered = render_notification_email(
            "AgentArk",
            "AgentArk - 2026-04-18",
            "Line one\n\n- Line two",
            Some("2026-04-18 09:30 IST"),
            Some("sections"),
        );
        assert!(rendered
            .text_body
            .contains("Generated at: 2026-04-18 09:30 IST"));
        assert!(rendered.html_body.contains("AgentArk"));
        assert!(rendered.html_body.contains("Line one"));
    }

    #[test]
    fn email_backend_auto_requires_exactly_one_ready_backend() {
        assert_eq!(
            normalize_email_backend_selection("auto", &[EMAIL_PROVIDER_GMAIL.to_string()]).unwrap(),
            EMAIL_PROVIDER_GMAIL
        );
        assert!(normalize_email_backend_selection(
            "auto",
            &[
                EMAIL_PROVIDER_GMAIL.to_string(),
                EMAIL_PROVIDER_GOOGLE_WORKSPACE.to_string(),
            ],
        )
        .is_err());
    }

    #[test]
    fn external_provider_id_uses_smtp_transport_when_provider_is_auto() {
        let mut config = EmailConfig::default();
        config.transport.kind = "smtp".to_string();
        assert_eq!(
            external_email_provider_id(&config).as_deref(),
            Some(EMAIL_PROVIDER_SMTP)
        );
    }
}
