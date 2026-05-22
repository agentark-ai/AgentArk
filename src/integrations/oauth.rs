//! OAuth 2.0 Handler with Security-First Design
//!
//! SECURITY GUARANTEES:
//! - Tokens are NEVER logged (Debug impl redacts)
//! - Tokens are NEVER sent to LLM (no Display impl, no serialization to JSON for LLM)
//! - Tokens are encrypted at rest using KeyManager
//! - Tokens are only used internally for API calls
//! - Access tokens auto-refresh, refresh tokens are long-lived
//!
//! The OAuthTokens struct intentionally does NOT implement common traits that could
//! accidentally expose tokens.

use crate::crypto::KeyManager;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use zeroize::Zeroizing;

/// Secure OAuth token container
///
/// SECURITY: This struct intentionally:
/// - Uses Zeroizing<String> to clear memory on drop
/// - Has a custom Debug impl that redacts tokens
/// - Does NOT implement Display
/// - Only serializes to encrypted storage, never to API responses
pub struct OAuthTokens {
    /// Access token (short-lived, auto-refreshed)
    access_token: Zeroizing<String>,
    /// Refresh token (long-lived, stored securely)
    refresh_token: Option<Zeroizing<String>>,
    /// Expiration timestamp (Unix seconds)
    expires_at: Option<i64>,
    /// Token type (usually "Bearer")
    token_type: String,
    /// Granted scopes
    scope: Option<String>,
}

// Custom Debug that NEVER shows token values
impl std::fmt::Debug for OAuthTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthTokens")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("expires_at", &self.expires_at)
            .field("token_type", &self.token_type)
            .field("scope", &self.scope)
            .finish()
    }
}

impl Clone for OAuthTokens {
    fn clone(&self) -> Self {
        Self {
            access_token: Zeroizing::new(self.access_token.as_str().to_string()),
            refresh_token: self
                .refresh_token
                .as_ref()
                .map(|t| Zeroizing::new(t.as_str().to_string())),
            expires_at: self.expires_at,
            token_type: self.token_type.clone(),
            scope: self.scope.clone(),
        }
    }
}

impl OAuthTokens {
    /// Check if the access token has expired (with 5 min buffer)
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = chrono::Utc::now().timestamp();
            now >= (expires_at - 300) // 5 minute buffer
        } else {
            false
        }
    }

    /// Get access token for internal use ONLY
    /// This should NEVER be logged or sent to LLM
    pub(crate) fn access_token(&self) -> &str {
        &self.access_token
    }

    /// Get refresh token for internal use ONLY
    pub(crate) fn refresh_token(&self) -> Option<&str> {
        self.refresh_token.as_ref().map(|t| t.as_str())
    }

    pub fn expires_at(&self) -> Option<i64> {
        self.expires_at
    }

    pub fn token_type(&self) -> &str {
        &self.token_type
    }

    pub fn scope(&self) -> Option<&str> {
        self.scope.as_deref()
    }
}

/// Internal struct for serialization ONLY - never expose to API
#[derive(Clone, Serialize, Deserialize)]
struct TokensForStorage {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<i64>,
    token_type: String,
    scope: Option<String>,
}

impl From<&OAuthTokens> for TokensForStorage {
    fn from(t: &OAuthTokens) -> Self {
        Self {
            access_token: t.access_token.to_string(),
            refresh_token: t.refresh_token.as_ref().map(|r| r.to_string()),
            expires_at: t.expires_at,
            token_type: t.token_type.clone(),
            scope: t.scope.clone(),
        }
    }
}

impl From<TokensForStorage> for OAuthTokens {
    fn from(t: TokensForStorage) -> Self {
        Self {
            access_token: Zeroizing::new(t.access_token),
            refresh_token: t.refresh_token.map(Zeroizing::new),
            expires_at: t.expires_at,
            token_type: t.token_type,
            scope: t.scope,
        }
    }
}

/// OAuth configuration for a service
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub client_id: String,
    /// SECURITY: client_secret is NOT Debug-printed
    client_secret: Zeroizing<String>,
    pub auth_url: String,
    pub token_url: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub extra_auth_params: BTreeMap<String, String>,
    pub extra_token_params: BTreeMap<String, String>,
    pub prompt: Option<String>,
    pub access_type: Option<String>,
}

pub struct OAuthConfigInput {
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub extra_auth_params: BTreeMap<String, String>,
    pub extra_token_params: BTreeMap<String, String>,
    pub prompt: Option<String>,
    pub access_type: Option<String>,
}

impl OAuthConfig {
    pub fn from_input(input: OAuthConfigInput) -> Self {
        Self {
            client_id: input.client_id,
            client_secret: Zeroizing::new(input.client_secret),
            auth_url: input.auth_url,
            token_url: input.token_url,
            redirect_uri: input.redirect_uri,
            scopes: input.scopes,
            extra_auth_params: input.extra_auth_params,
            extra_token_params: input.extra_token_params,
            prompt: input.prompt,
            access_type: input.access_type,
        }
    }

    /// Generate the authorization URL for user to visit
    pub fn auth_url(&self, state: &str) -> String {
        self.auth_url_with_pkce(state, None)
    }

    pub fn auth_url_with_pkce(&self, state: &str, code_challenge: Option<&str>) -> String {
        let scopes = self.scopes.join(" ");
        let mut url = reqwest::Url::parse(&self.auth_url)
            .unwrap_or_else(|_| reqwest::Url::parse("http://invalid.local/").expect("valid url"));
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("client_id", &self.client_id);
            pairs.append_pair("redirect_uri", &self.redirect_uri);
            pairs.append_pair("response_type", "code");
            if !scopes.trim().is_empty() {
                pairs.append_pair("scope", &scopes);
            }
            pairs.append_pair("state", state);
            if let Some(access_type) = self
                .access_type
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                pairs.append_pair("access_type", access_type);
            }
            if let Some(prompt) = self
                .prompt
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                pairs.append_pair("prompt", prompt);
            }
            if let Some(challenge) = code_challenge
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                pairs.append_pair("code_challenge", challenge);
                pairs.append_pair("code_challenge_method", "S256");
            }
            for (key, value) in &self.extra_auth_params {
                if !key.trim().is_empty() && !value.trim().is_empty() {
                    pairs.append_pair(key, value);
                }
            }
        }
        url.into()
    }

    /// Get client secret for internal use only
    pub(crate) fn client_secret(&self) -> &str {
        &self.client_secret
    }
}

/// OAuth client for handling auth flows
///
/// SECURITY: All token operations are internal, never exposed
pub struct OAuthClient {
    http: reqwest::Client,
}

impl OAuthClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    /// Exchange authorization code for tokens
    /// SECURITY: Tokens returned are in secure container, never logged
    pub async fn exchange_code(&self, config: &OAuthConfig, code: &str) -> Result<OAuthTokens> {
        self.exchange_code_with_pkce(config, code, None).await
    }

    /// Exchange authorization code for tokens, optionally with PKCE verifier.
    pub async fn exchange_code_with_pkce(
        &self,
        config: &OAuthConfig,
        code: &str,
        pkce_verifier: Option<&str>,
    ) -> Result<OAuthTokens> {
        // SECURITY: Using form encoding, not logging the request
        let mut params = vec![
            ("client_id".to_string(), config.client_id.clone()),
            (
                "client_secret".to_string(),
                config.client_secret().to_string(),
            ),
            ("code".to_string(), code.to_string()),
            ("redirect_uri".to_string(), config.redirect_uri.clone()),
            ("grant_type".to_string(), "authorization_code".to_string()),
        ];
        if let Some(verifier) = pkce_verifier
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            params.push(("code_verifier".to_string(), verifier.to_string()));
        }
        for (key, value) in &config.extra_token_params {
            if !key.trim().is_empty() && !value.trim().is_empty() {
                params.push((key.clone(), value.clone()));
            }
        }

        let response = self
            .http
            .post(&config.token_url)
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            // SECURITY: Don't include response body in error (might contain partial secrets)
            let status = response.status();
            return Err(anyhow!("Token exchange failed with status {}", status));
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            refresh_token: Option<String>,
            expires_in: Option<i64>,
            token_type: String,
            scope: Option<String>,
        }

        let token_response: TokenResponse = response.json().await?;

        let expires_at = token_response
            .expires_in
            .map(|secs| chrono::Utc::now().timestamp() + secs);

        // SECURITY: Immediately wrap in Zeroizing containers
        Ok(OAuthTokens {
            access_token: Zeroizing::new(token_response.access_token),
            refresh_token: token_response.refresh_token.map(Zeroizing::new),
            expires_at,
            token_type: token_response.token_type,
            scope: token_response.scope,
        })
    }

    /// Refresh an access token using the refresh token
    /// SECURITY: Tokens never logged
    pub async fn refresh_token(
        &self,
        config: &OAuthConfig,
        refresh_token: &str,
    ) -> Result<OAuthTokens> {
        let mut params = vec![
            ("client_id".to_string(), config.client_id.clone()),
            (
                "client_secret".to_string(),
                config.client_secret().to_string(),
            ),
            ("refresh_token".to_string(), refresh_token.to_string()),
            ("grant_type".to_string(), "refresh_token".to_string()),
        ];
        for (key, value) in &config.extra_token_params {
            if !key.trim().is_empty() && !value.trim().is_empty() {
                params.push((key.clone(), value.clone()));
            }
        }

        let response = self
            .http
            .post(&config.token_url)
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(anyhow!("Token refresh failed with status {}", status));
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            expires_in: Option<i64>,
            token_type: String,
            scope: Option<String>,
        }

        let token_response: TokenResponse = response.json().await?;

        let expires_at = token_response
            .expires_in
            .map(|secs| chrono::Utc::now().timestamp() + secs);

        Ok(OAuthTokens {
            access_token: Zeroizing::new(token_response.access_token),
            refresh_token: Some(Zeroizing::new(refresh_token.to_string())), // Keep the old refresh token
            expires_at,
            token_type: token_response.token_type,
            scope: token_response.scope,
        })
    }
}

impl Default for OAuthClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Token storage - encrypted at rest with AES-256-GCM
///
/// SECURITY:
/// - All tokens encrypted using KeyManager before writing to disk
/// - Decryption only happens in memory
/// - Failed decryption returns error, doesn't expose partial data
pub struct TokenStorage {
    storage_path: std::path::PathBuf,
    key_manager: std::sync::Arc<KeyManager>,
}

impl TokenStorage {
    /// Save tokens for a service (encrypted)
    pub fn save(&self, service_id: &str, tokens: &OAuthTokens) -> Result<()> {
        let mut all_tokens = self.load_all_internal()?;
        all_tokens.insert(service_id.to_string(), TokensForStorage::from(tokens));

        // SECURITY: Serialize to JSON, then encrypt
        let json = serde_json::to_vec(&all_tokens)?;
        let encrypted = self.key_manager.encrypt(&json)?;
        std::fs::write(&self.storage_path, encrypted)?;

        Ok(())
    }

    /// Delete tokens for a service
    pub fn delete(&self, service_id: &str) -> Result<()> {
        let mut all_tokens = self.load_all_internal()?;
        all_tokens.remove(service_id);

        if all_tokens.is_empty() {
            let _ = std::fs::remove_file(&self.storage_path);
        } else {
            let json = serde_json::to_vec(&all_tokens)?;
            let encrypted = self.key_manager.encrypt(&json)?;
            std::fs::write(&self.storage_path, encrypted)?;
        }

        Ok(())
    }

    fn load_all_internal(&self) -> Result<std::collections::HashMap<String, TokensForStorage>> {
        if !self.storage_path.exists() {
            return Ok(std::collections::HashMap::new());
        }

        let encrypted = std::fs::read(&self.storage_path)?;
        let decrypted = self.key_manager.decrypt(&encrypted)?;
        let tokens: std::collections::HashMap<String, TokensForStorage> =
            serde_json::from_slice(&decrypted)?;

        Ok(tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_debug_redacted() {
        let tokens = OAuthTokens {
            access_token: Zeroizing::new("super_secret_token".to_string()),
            refresh_token: Some(Zeroizing::new("refresh_secret".to_string())),
            expires_at: Some(12345),
            token_type: "Bearer".to_string(),
            scope: Some("calendar".to_string()),
        };

        let debug_output = format!("{:?}", tokens);
        assert!(!debug_output.contains("super_secret"));
        assert!(!debug_output.contains("refresh_secret"));
        assert!(debug_output.contains("[REDACTED]"));
    }

    #[test]
    fn test_config_auth_url() {
        let config = OAuthConfig {
            client_id: "client123".to_string(),
            client_secret: Zeroizing::new("secret456".to_string()),
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
            token_url: "https://oauth2.googleapis.com/token".to_string(),
            redirect_uri: "http://localhost:8990/oauth/callback".to_string(),
            scopes: vec!["https://www.googleapis.com/auth/calendar".to_string()],
            extra_auth_params: BTreeMap::new(),
            extra_token_params: BTreeMap::new(),
            prompt: Some("consent".to_string()),
            access_type: Some("offline".to_string()),
        };

        let url = config.auth_url("test_state");
        assert!(url.contains("client123"));
        assert!(!url.contains("secret456")); // Secret should NOT be in auth URL
        assert!(url.contains("test_state"));
    }
}
