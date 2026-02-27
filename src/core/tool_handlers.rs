use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc::Sender, RwLock};

use crate::core::{Agent, ExecutionTrace, StreamEvent, ToolCall};

pub struct ToolHandlerContext<'a> {
    pub trace_ref: &'a Arc<RwLock<ExecutionTrace>>,
    pub stream_tx: Option<&'a Sender<StreamEvent>>,
    pub request_channel: &'a str,
    pub public_base_url: Option<&'a str>,
    pub integration_aliases: &'a HashMap<String, String>,
}

#[async_trait]
pub trait ToolHandler: Send + Sync {
    fn id(&self) -> &'static str;
    fn can_handle(&self, agent: &Agent, call: &ToolCall, ctx: &ToolHandlerContext<'_>) -> bool;
    async fn handle(
        &self,
        agent: &Agent,
        call: &ToolCall,
        ctx: &ToolHandlerContext<'_>,
    ) -> Result<Option<String>>;
}

pub struct GenerateImageToolHandler;
pub struct GenerateVideoToolHandler;
pub struct BrowserAutoToolHandler;
pub struct ScreenshotToolHandler;
pub struct ComposeReportToolHandler;
pub struct IntegrationToolHandler;
pub struct SelfEvolveToolHandler;
pub struct AppDeployToolHandler;
pub struct RuntimeToolHandler;

#[async_trait]
impl ToolHandler for GenerateImageToolHandler {
    fn id(&self) -> &'static str {
        "generate_image"
    }

    fn can_handle(&self, _agent: &Agent, call: &ToolCall, _ctx: &ToolHandlerContext<'_>) -> bool {
        call.name == "generate_image"
    }

    async fn handle(
        &self,
        agent: &Agent,
        call: &ToolCall,
        ctx: &ToolHandlerContext<'_>,
    ) -> Result<Option<String>> {
        let out = agent
            .handle_generate_image_tool_call(call, ctx.stream_tx, ctx.request_channel)
            .await?;
        Ok(Some(out))
    }
}

#[async_trait]
impl ToolHandler for GenerateVideoToolHandler {
    fn id(&self) -> &'static str {
        "generate_video"
    }

    fn can_handle(&self, _agent: &Agent, call: &ToolCall, _ctx: &ToolHandlerContext<'_>) -> bool {
        call.name == "generate_video"
    }

    async fn handle(
        &self,
        agent: &Agent,
        call: &ToolCall,
        ctx: &ToolHandlerContext<'_>,
    ) -> Result<Option<String>> {
        let out = agent
            .handle_generate_video_tool_call(
                call,
                ctx.stream_tx,
                ctx.request_channel,
                ctx.public_base_url,
            )
            .await?;
        Ok(Some(out))
    }
}

#[async_trait]
impl ToolHandler for BrowserAutoToolHandler {
    fn id(&self) -> &'static str {
        "browser_auto"
    }

    fn can_handle(&self, _agent: &Agent, call: &ToolCall, _ctx: &ToolHandlerContext<'_>) -> bool {
        call.name == "browser_auto"
    }

    async fn handle(
        &self,
        agent: &Agent,
        call: &ToolCall,
        ctx: &ToolHandlerContext<'_>,
    ) -> Result<Option<String>> {
        let out = agent
            .handle_browser_auto_tool_call(call, ctx.stream_tx)
            .await?;
        Ok(Some(out))
    }
}

#[async_trait]
impl ToolHandler for ScreenshotToolHandler {
    fn id(&self) -> &'static str {
        "page_screenshot"
    }

    fn can_handle(&self, _agent: &Agent, call: &ToolCall, _ctx: &ToolHandlerContext<'_>) -> bool {
        call.name == "page_screenshot"
    }

    async fn handle(
        &self,
        agent: &Agent,
        call: &ToolCall,
        ctx: &ToolHandlerContext<'_>,
    ) -> Result<Option<String>> {
        let out = agent
            .handle_screenshot_tool_call(call, ctx.stream_tx, ctx.request_channel)
            .await?;
        Ok(Some(out))
    }
}

#[async_trait]
impl ToolHandler for ComposeReportToolHandler {
    fn id(&self) -> &'static str {
        "compose_report"
    }

    fn can_handle(&self, _agent: &Agent, call: &ToolCall, _ctx: &ToolHandlerContext<'_>) -> bool {
        call.name == "compose_report"
    }

    async fn handle(
        &self,
        agent: &Agent,
        call: &ToolCall,
        ctx: &ToolHandlerContext<'_>,
    ) -> Result<Option<String>> {
        let out = agent
            .handle_compose_report_tool_call(call, ctx.stream_tx)
            .await?;
        Ok(Some(out))
    }
}

#[async_trait]
impl ToolHandler for IntegrationToolHandler {
    fn id(&self) -> &'static str {
        "integration"
    }

    fn can_handle(&self, agent: &Agent, call: &ToolCall, ctx: &ToolHandlerContext<'_>) -> bool {
        if call.name == "browser_auto"
            || call.name == "generate_image"
            || call.name == "generate_video"
            || call.name == "page_screenshot"
            || call.name == "compose_report"
            || call.name == "self_evolve"
            || call.name == "app_deploy"
        {
            return false;
        }
        agent
            .resolve_tool_integration_id(&call.name, ctx.integration_aliases)
            .is_some()
    }

    async fn handle(
        &self,
        agent: &Agent,
        call: &ToolCall,
        ctx: &ToolHandlerContext<'_>,
    ) -> Result<Option<String>> {
        if let Some(tx) = ctx.stream_tx {
            let _ = tx.try_send(StreamEvent::ToolStart {
                name: call.name.clone(),
                payload: None,
            });
        }
        let allowed = if agent.should_auto_approve_action(&call.name) {
            tracing::info!(
                "Auto-approving command-like action '{}' for AgentArk",
                call.name
            );
            true
        } else {
            agent.safety.is_allowed(&call.name, &call.arguments).await?
        };
        if !allowed {
            let blocked = format!("Tool '{}' blocked by safety policy", call.name);
            if let Some(tx) = ctx.stream_tx {
                let _ = tx.try_send(StreamEvent::ToolResult {
                    name: call.name.clone(),
                    content: blocked.clone(),
                });
            }
            return Ok(Some(blocked));
        }

        let Some(integration_id) =
            agent.resolve_tool_integration_id(&call.name, ctx.integration_aliases)
        else {
            return Ok(None);
        };
        let out = agent
            .execute_integration_tool_call(
                call,
                ctx.stream_tx,
                ctx.request_channel,
                &integration_id,
            )
            .await;
        Ok(Some(out))
    }
}

#[async_trait]
impl ToolHandler for SelfEvolveToolHandler {
    fn id(&self) -> &'static str {
        "self_evolve"
    }

    fn can_handle(&self, _agent: &Agent, call: &ToolCall, _ctx: &ToolHandlerContext<'_>) -> bool {
        call.name == "self_evolve"
    }

    async fn handle(
        &self,
        agent: &Agent,
        call: &ToolCall,
        ctx: &ToolHandlerContext<'_>,
    ) -> Result<Option<String>> {
        let out = agent
            .handle_self_evolve_tool_call(call, ctx.stream_tx)
            .await?;
        Ok(Some(out))
    }
}

#[async_trait]
impl ToolHandler for AppDeployToolHandler {
    fn id(&self) -> &'static str {
        "app_deploy"
    }

    fn can_handle(&self, _agent: &Agent, call: &ToolCall, _ctx: &ToolHandlerContext<'_>) -> bool {
        call.name == "app_deploy"
    }

    async fn handle(
        &self,
        agent: &Agent,
        call: &ToolCall,
        ctx: &ToolHandlerContext<'_>,
    ) -> Result<Option<String>> {
        let out = agent
            .handle_app_deploy_tool_call(
                call,
                ctx.stream_tx,
                ctx.request_channel,
                ctx.public_base_url,
            )
            .await?;
        Ok(Some(out))
    }
}

#[async_trait]
impl ToolHandler for RuntimeToolHandler {
    fn id(&self) -> &'static str {
        "runtime"
    }

    fn can_handle(&self, _agent: &Agent, _call: &ToolCall, _ctx: &ToolHandlerContext<'_>) -> bool {
        true
    }

    async fn handle(
        &self,
        agent: &Agent,
        call: &ToolCall,
        ctx: &ToolHandlerContext<'_>,
    ) -> Result<Option<String>> {
        let out = agent
            .handle_runtime_tool_call(
                call,
                ctx.trace_ref,
                ctx.stream_tx,
                ctx.request_channel,
                ctx.public_base_url,
            )
            .await?;
        Ok(Some(out))
    }
}

pub fn default_tool_handlers() -> Vec<Box<dyn ToolHandler>> {
    vec![
        Box::new(GenerateImageToolHandler),
        Box::new(GenerateVideoToolHandler),
        Box::new(BrowserAutoToolHandler),
        Box::new(ScreenshotToolHandler),
        Box::new(ComposeReportToolHandler),
        Box::new(IntegrationToolHandler),
        Box::new(SelfEvolveToolHandler),
        Box::new(AppDeployToolHandler),
        Box::new(RuntimeToolHandler),
    ]
}
