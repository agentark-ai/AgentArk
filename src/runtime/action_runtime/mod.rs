#![allow(clippy::too_many_lines)]

#[path = "management/action_management.rs"]
mod action_management;
#[path = "governance/action_scope.rs"]
mod action_scope;
#[path = "governance/authorization.rs"]
mod authorization;
#[path = "web/browser.rs"]
mod browser;
#[path = "startup/builtin_actions.rs"]
mod builtin_actions;
#[path = "integrations/capabilities.rs"]
mod capabilities;
#[path = "execution/cli_execution.rs"]
mod cli_execution;
#[path = "execution/code_execute.rs"]
mod code_execute;
#[path = "management/control_actions.rs"]
mod control_actions;
#[path = "execution/execution.rs"]
mod execution;
#[path = "integrations/extension_packs.rs"]
mod extension_packs;
#[path = "execution/external_actions.rs"]
mod external_actions;
#[path = "web/http_access.rs"]
mod http_access;
#[path = "runtime/inspection_lookup.rs"]
mod inspection_lookup;
#[path = "integrations/integrations.rs"]
mod integrations;
#[path = "startup/markdown_actions.rs"]
mod markdown_actions;
#[path = "execution/native_execution.rs"]
mod native_execution;
#[path = "integrations/pipeline_connectors.rs"]
mod pipeline_connectors;
#[path = "runtime/pipeline_runtime.rs"]
mod pipeline_runtime;
#[path = "startup/registration.rs"]
mod registration;
#[path = "governance/reviews.rs"]
mod reviews;
#[path = "startup/startup.rs"]
mod startup;
#[path = "runtime/tunnel_artifacts.rs"]
mod tunnel_artifacts;
#[path = "runtime/vision.rs"]
mod vision;
#[path = "execution/wasm_docker.rs"]
mod wasm_docker;
#[path = "web/web_requests.rs"]
mod web_requests;
#[path = "runtime/workflows.rs"]
mod workflows;
