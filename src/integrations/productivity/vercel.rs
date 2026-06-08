use super::{Capability, Integration, IntegrationStatus};
use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;

pub struct VercelConnector {
    config_dir: PathBuf,
    data_dir: Option<PathBuf>,
}

impl VercelConnector {
    pub fn new_with_paths(config_dir: PathBuf, data_dir: PathBuf) -> Self {
        Self {
            config_dir,
            data_dir: Some(data_dir),
        }
    }

    fn data_dir(&self) -> PathBuf {
        self.data_dir.clone().unwrap_or_else(|| {
            self.config_dir
                .parent()
                .map(|path| path.join("data"))
                .unwrap_or_else(|| self.config_dir.clone())
        })
    }
}

#[async_trait]
impl Integration for VercelConnector {
    fn id(&self) -> &str {
        "vercel"
    }

    fn name(&self) -> &str {
        "Vercel"
    }

    fn description(&self) -> &str {
        "Vercel deployment provider for publishing AgentArk apps"
    }

    fn icon(&self) -> &str {
        ""
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Write]
    }

    async fn status(&self) -> IntegrationStatus {
        let data_dir = self.data_dir();
        if crate::actions::vercel::vercel_token_is_configured(&self.config_dir, &data_dir) {
            IntegrationStatus::Connected
        } else {
            IntegrationStatus::NotConfigured
        }
    }

    async fn execute(
        &self,
        _action: &str,
        _params: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        Err(anyhow::anyhow!(
            "Use app_deploy or the Apps publish endpoint for Vercel deployments"
        ))
    }
}
