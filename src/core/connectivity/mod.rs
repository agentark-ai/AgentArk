//! External connectivity, channels, integration auth, and browser session modules.

#[path = "auth/auth_profiles.rs"]
pub mod auth_profiles;
#[path = "browser/browser_profiles.rs"]
pub mod browser_profiles;
#[path = "browser/browser_session.rs"]
pub mod browser_session;
#[path = "channels/companion.rs"]
pub mod companion;
#[path = "integrations/connect_flow.rs"]
pub mod connect_flow;
#[path = "network/connector.rs"]
pub mod connector;
#[path = "channels/email_delivery.rs"]
pub mod email_delivery;
#[path = "gateway/gateway.rs"]
pub mod gateway;
#[path = "gateway/gateway_ops.rs"]
pub mod gateway_ops;
#[path = "auth/integration_auth.rs"]
pub mod integration_auth;
#[path = "integrations/integration_sync.rs"]
pub mod integration_sync;
#[path = "channels/sender_verification.rs"]
pub mod sender_verification;
