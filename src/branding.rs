pub const LEGACY_PRODUCT_NAME: &str = "AgentArk";
pub const PRODUCT_NAME: &str = "AgentArk";
pub const PRODUCT_CATEGORY: &str = "Personal AI OS";
pub const PRODUCT_SLUG: &str = "agentark";
pub const PRODUCT_URI_SCHEME: &str = "agentark";
pub const PROJECT_DIRS_QUALIFIER: &str = "com";
pub const PROJECT_DIRS_ORGANIZATION: &str = PRODUCT_SLUG;
pub const REPOSITORY_URL: &str = "https://github.com/agentark-ai/AgentArk";
pub const DOCS_BASIC_AUTH_REALM: &str = "Basic realm=\"AgentArk Docs\"";
pub const SESSION_COOKIE_NAME: &str = "agentark_session";
pub const WEBHOOK_SIGNATURE_HEADER: &str = "X-AgentArk-Signature";
pub const WEBHOOK_SECRET_HEADER: &str = "X-AgentArk-Webhook-Secret";
pub const PRODUCT_NAME_TOKEN: &str = "__PRODUCT_NAME__";

pub fn default_agent_name() -> String {
    PRODUCT_NAME.to_string()
}

pub fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from(
        PROJECT_DIRS_QUALIFIER,
        PROJECT_DIRS_ORGANIZATION,
        PRODUCT_NAME,
    )
}

pub fn help_uri(path: &str) -> String {
    format!("{}://{}", PRODUCT_URI_SCHEME, path.trim_start_matches('/'))
}

pub fn brand_text(text: &str) -> String {
    render_template(&text.replace(LEGACY_PRODUCT_NAME, PRODUCT_NAME))
}

pub fn render_template(template: &str) -> String {
    template.replace(PRODUCT_NAME_TOKEN, PRODUCT_NAME)
}

pub fn versioned_user_agent() -> String {
    format!("{}/{}", PRODUCT_NAME, env!("CARGO_PKG_VERSION"))
}

pub fn user_agent_with_suffix(suffix: &str) -> String {
    format!("{} {}", versioned_user_agent(), suffix)
}
