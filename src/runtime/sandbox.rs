//! Sandbox implementations for action execution

use serde::{Deserialize, Serialize};

/// Sandbox execution mode
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum SandboxMode {
    /// No sandbox - run directly on host
    Native,
    /// WASM sandbox - lightweight, fast
    #[default]
    Wasm,
    /// Docker sandbox - full isolation
    Docker,
}
