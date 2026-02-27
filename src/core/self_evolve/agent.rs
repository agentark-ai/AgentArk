//! Self-Evolve Agent — the inner LLM loop that modifies AgentArk's own code.
//!
//! Orchestrates: research → plan → implement → build → test → fix → report.

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::actions::ActionDef;
use crate::core::agent::ConversationMessage;
use crate::core::llm::{LlmClient, ToolCall};

use super::coding_guidelines;
use super::security_review;
use super::tools;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for a self-evolve session.
#[derive(Debug, Clone)]
pub struct SelfEvolveConfig {
    /// Maximum inner-loop iterations (LLM turns with tool calls).
    pub max_iterations: usize,
    /// Maximum build-test-fix cycles before aborting.
    pub max_build_fix_cycles: usize,
    /// Project root directory (where Cargo.toml lives).
    pub project_root: PathBuf,
}

impl Default for SelfEvolveConfig {
    fn default() -> Self {
        Self {
            max_iterations: 25,
            max_build_fix_cycles: 5,
            project_root: PathBuf::from("."),
        }
    }
}

// ---------------------------------------------------------------------------
// Result
// ---------------------------------------------------------------------------

/// Result of a self-evolve session.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SelfEvolveResult {
    pub success: bool,
    pub diff_summary: String,
    pub files_changed: Vec<String>,
    pub iterations_used: usize,
    pub error: Option<String>,
    pub security_warnings: Vec<String>,
    pub push_recommended: bool,
    pub push_suggestion: Option<String>,
}

// ---------------------------------------------------------------------------
// Agent
// ---------------------------------------------------------------------------

pub struct SelfEvolveAgent {
    config: SelfEvolveConfig,
    llm: LlmClient,
}

impl SelfEvolveAgent {
    pub fn new(config: SelfEvolveConfig, llm: LlmClient) -> Self {
        Self { config, llm }
    }

    /// Main entry point. Runs the full self-evolve loop.
    pub async fn execute(&self, user_request: &str) -> Result<SelfEvolveResult> {
        tracing::info!(
            "Self-evolve starting: '{}' (max {} iterations)",
            &user_request[..user_request.len().min(80)],
            self.config.max_iterations
        );

        let system_prompt = self.build_system_prompt(user_request);
        let tool_defs = self.build_tool_definitions();
        let mut history: Vec<ConversationMessage> = Vec::new();
        let mut build_fix_cycles = 0_usize;
        let mut iterations_used = 0_usize;
        let mut file_backups: HashMap<String, Option<String>> = HashMap::new();

        // Initial user message
        let initial_msg = format!(
            "Implement the following change to AgentArk:\n\n{}\n\n\
             Start by reading the relevant existing source files to understand patterns, \
             then implement the change. After writing code, run build_check to verify.",
            user_request
        );

        for iteration in 0..self.config.max_iterations {
            iterations_used = iteration + 1;

            // Determine the user message for this turn
            let user_msg = if iteration == 0 {
                initial_msg.clone()
            } else {
                // After the first turn, the "user" messages are tool results
                // which are already appended to history. Use a continuation prompt.
                "Continue with the next step.".to_string()
            };

            // Call LLM
            let response = match self
                .llm
                .chat_with_history(&system_prompt, &user_msg, &history, &[], &tool_defs)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("Self-evolve LLM error on iteration {}: {}", iteration, e);
                    // Retry once
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    match self
                        .llm
                        .chat_with_history(&system_prompt, &user_msg, &history, &[], &tool_defs)
                        .await
                    {
                        Ok(r) => r,
                        Err(e2) => {
                            return Ok(SelfEvolveResult {
                                success: false,
                                diff_summary: String::new(),
                                files_changed: Vec::new(),
                                iterations_used,
                                error: Some(format!("LLM error after retry: {}", e2)),
                                security_warnings: Vec::new(),
                                push_recommended: false,
                                push_suggestion: None,
                            });
                        }
                    }
                }
            };

            // Record assistant message in history
            history.push(ConversationMessage {
                role: "assistant".to_string(),
                content: response.content.clone(),
                _timestamp: chrono::Utc::now(),
            });

            // If no tool calls, the agent is done (or stuck)
            if response.tool_calls.is_empty() {
                // Check if it called task_complete in the content
                if response.content.contains("TASK_COMPLETE")
                    || response
                        .content
                        .to_lowercase()
                        .contains("implementation complete")
                {
                    break;
                }
                // If there's meaningful content, it might be a final summary
                if iteration > 3 && !response.content.is_empty() {
                    break;
                }
                // Otherwise nudge it to continue
                history.push(ConversationMessage {
                    role: "user".to_string(),
                    content: "You didn't call any tools. Please continue implementing \
                              by calling the appropriate source or build tools."
                        .to_string(),
                    _timestamp: chrono::Utc::now(),
                });
                continue;
            }

            // Execute tool calls
            let mut tool_results = Vec::new();
            for call in &response.tool_calls {
                tracing::debug!("Self-evolve tool call: {}({})", call.name, call.id);
                let result = self.dispatch_tool_call(call, &mut file_backups).await;

                let result_text = match result {
                    Ok(text) => {
                        // Track build failures
                        if (call.name == "build_check"
                            || call.name == "run_tests"
                            || call.name == "lint_check")
                            && text.contains("FAILED")
                        {
                            build_fix_cycles += 1;
                            if build_fix_cycles > self.config.max_build_fix_cycles {
                                return Ok(SelfEvolveResult {
                                    success: false,
                                    diff_summary: String::new(),
                                    files_changed: Vec::new(),
                                    iterations_used,
                                    error: Some(format!(
                                        "Exceeded {} build-fix cycles. Last failure:\n{}",
                                        self.config.max_build_fix_cycles, text
                                    )),
                                    security_warnings: Vec::new(),
                                    push_recommended: false,
                                    push_suggestion: None,
                                });
                            }
                        }
                        text
                    }
                    Err(e) => format!("Tool error: {}", e),
                };

                tool_results.push(format!("[{}] {}", call.name, result_text));

                // Check for task_complete signal
                if call.name == "task_complete" {
                    // Break out of tool loop
                    break;
                }
            }

            // Append tool results as a "user" message (simulating tool response)
            let combined_results = tool_results.join("\n\n");
            history.push(ConversationMessage {
                role: "user".to_string(),
                content: format!("Tool results:\n{}", combined_results),
                _timestamp: chrono::Utc::now(),
            });

            // Check if task_complete was called
            if response
                .tool_calls
                .iter()
                .any(|c| c.name == "task_complete")
            {
                break;
            }
        }

        // --- Post-loop: security review and result ---

        let changed_files = tools::git_changed_files(&self.config.project_root)
            .await
            .unwrap_or_default();

        if changed_files.is_empty() {
            return Ok(SelfEvolveResult {
                success: false,
                diff_summary: String::new(),
                files_changed: Vec::new(),
                iterations_used,
                error: Some("No files were changed".to_string()),
                security_warnings: Vec::new(),
                push_recommended: false,
                push_suggestion: None,
            });
        }

        // Security review
        let findings = security_review::review(&changed_files, &self.config.project_root).await;
        let mut security_warnings: Vec<String> = findings.iter().map(|f| f.to_string()).collect();

        if security_review::has_blocking(&findings) {
            tracing::warn!(
                "Self-evolve blocked by security review: {:?}",
                security_warnings
            );

            let rollback_note = match self.rollback_file_changes(&file_backups).await {
                Ok(restored_count) => format!("Rollback applied to {} file(s).", restored_count),
                Err(e) => format!("Rollback failed: {}", e),
            };

            return Ok(SelfEvolveResult {
                success: false,
                diff_summary: String::new(),
                files_changed: changed_files
                    .iter()
                    .filter_map(|f| {
                        f.strip_prefix(&self.config.project_root)
                            .ok()
                            .map(|p| p.to_string_lossy().to_string())
                    })
                    .collect(),
                iterations_used,
                error: Some(format!(
                    "Security review found blocking issues. {}",
                    rollback_note
                )),
                security_warnings,
                push_recommended: false,
                push_suggestion: None,
            });
        }

        // Final verification gate: require build + tests to pass.
        let final_build = tools::build_check(&self.config.project_root)
            .await
            .unwrap_or_else(|e| format!("cargo check: FAILED\n{}", e));
        let final_tests = tools::run_tests(&self.config.project_root)
            .await
            .unwrap_or_else(|e| format!("cargo test: FAILED\n{}", e));

        if final_build.contains("FAILED") || final_tests.contains("FAILED") {
            return Ok(SelfEvolveResult {
                success: false,
                diff_summary: String::new(),
                files_changed: changed_files
                    .iter()
                    .filter_map(|f| {
                        f.strip_prefix(&self.config.project_root)
                            .ok()
                            .map(|p| p.to_string_lossy().to_string())
                    })
                    .collect(),
                iterations_used,
                error: Some(format!(
                    "Final verification failed.\n{}\n\n{}",
                    final_build, final_tests
                )),
                security_warnings,
                push_recommended: false,
                push_suggestion: None,
            });
        }

        let final_lint = tools::lint_check(&self.config.project_root)
            .await
            .unwrap_or_else(|e| format!("cargo clippy: FAILED\n{}", e));
        if final_lint.contains("FAILED") {
            security_warnings.push(
                "Final lint check failed; review clippy output before publishing.".to_string(),
            );
        }

        // Generate diff summary
        let diff_summary = tools::git_diff_summary(&self.config.project_root)
            .await
            .unwrap_or_else(|_| "Unable to generate diff".to_string());

        let files_list: Vec<String> = changed_files
            .iter()
            .filter_map(|f| {
                f.strip_prefix(&self.config.project_root)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            })
            .collect();

        let push_suggestion = match tools::git_current_branch(&self.config.project_root).await {
            Ok(branch) => Some(format!(
                "Local changes are ready on branch '{}'. Ask the user for approval before pushing (e.g. `git push origin {}`).",
                branch, branch
            )),
            Err(_) => Some(
                "Local changes are ready. Ask the user whether these commits should be pushed to the remote repository."
                    .to_string(),
            ),
        };

        tracing::info!(
            "Self-evolve complete: {} files changed, {} iterations",
            files_list.len(),
            iterations_used
        );

        Ok(SelfEvolveResult {
            success: true,
            diff_summary,
            files_changed: files_list,
            iterations_used,
            error: None,
            security_warnings,
            push_recommended: true,
            push_suggestion,
        })
    }

    // -----------------------------------------------------------------------
    // System prompt
    // -----------------------------------------------------------------------

    fn build_system_prompt(&self, user_request: &str) -> String {
        format!(
            r#"You are AgentArk's Self-Evolve Agent — a coding specialist that modifies AgentArk's own source code.

## Your Mission
Implement the following change request:
{request}

## Available Tools
- source_read(path) — Read a source file
- source_write(path, content) — Write/create a file
- source_edit(path, search, replace) — Search-and-replace edit in a file
- source_list(path, pattern?) — List directory contents
- source_search(pattern, glob?) — Search across the codebase
- build_check() — Run `cargo check` for fast syntax validation
- run_tests() — Run `cargo test`
- lint_check() — Run `cargo clippy -- -D warnings`
- frontend_build() — Run `npm run build` in frontend/
- web_search(query) — Search the web for API docs, examples
- task_complete(summary) — Signal that you are done

## Workflow
1. RESEARCH: Read relevant existing files to understand patterns. Use source_search and source_list.
2. PLAN: Think about what files need to change and in what order.
3. IMPLEMENT: Write code using source_write (new files) or source_edit (modify existing).
4. VERIFY: Run build_check after Rust changes. Run frontend_build after TypeScript changes.
5. TEST: Run run_tests to verify nothing is broken.
6. FIX: If build/test fails, read the error carefully, fix the code, re-verify.
7. COMPLETE: Call task_complete with a summary of what you changed.

## Rules
- Always read existing files BEFORE modifying them — understand the patterns.
- Use source_edit for small targeted changes to existing files.
- Use source_write for new files or complete rewrites.
- Run build_check after EVERY batch of Rust file changes.
- Follow the coding guidelines below EXACTLY.
- Never modify .env, secrets.enc, or credential files.
- Never introduce unsafe{{}} blocks.
- Always handle errors with ? — never unwrap() in non-test code.
- Add tracing::info!() for important new code paths.

{guidelines}"#,
            request = user_request,
            guidelines = coding_guidelines::coding_guidelines(),
        )
    }

    // -----------------------------------------------------------------------
    // Tool definitions (for the LLM)
    // -----------------------------------------------------------------------

    fn build_tool_definitions(&self) -> Vec<ActionDef> {
        vec![
            ActionDef {
                name: "source_read".to_string(),
                description: "Read a source file and return its contents".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Relative path from project root (e.g. src/integrations/mod.rs)"
                        }
                    },
                    "required": ["path"]
                }),
                capabilities: vec![],
                sandbox_mode: None,
                source: crate::actions::ActionSource::System,
                file_path: None,
            },
            ActionDef {
                name: "source_write".to_string(),
                description: "Write content to a file (creates parent dirs if needed)".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Relative path from project root"
                        },
                        "content": {
                            "type": "string",
                            "description": "Full file content to write"
                        }
                    },
                    "required": ["path", "content"]
                }),
                capabilities: vec![],
                sandbox_mode: None,
                source: crate::actions::ActionSource::System,
                file_path: None,
            },
            ActionDef {
                name: "source_edit".to_string(),
                description: "Apply a search-and-replace edit to a file. The search string must be unique in the file.".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Relative path from project root"
                        },
                        "search": {
                            "type": "string",
                            "description": "Exact text to find in the file"
                        },
                        "replace": {
                            "type": "string",
                            "description": "Text to replace the search string with"
                        }
                    },
                    "required": ["path", "search", "replace"]
                }),
                capabilities: vec![],
                sandbox_mode: None,
                source: crate::actions::ActionSource::System,
                file_path: None,
            },
            ActionDef {
                name: "source_list".to_string(),
                description: "List files in a directory, with optional name filter".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Directory path relative to project root"
                        },
                        "pattern": {
                            "type": "string",
                            "description": "Optional substring filter for file names"
                        }
                    },
                    "required": ["path"]
                }),
                capabilities: vec![],
                sandbox_mode: None,
                source: crate::actions::ActionSource::System,
                file_path: None,
            },
            ActionDef {
                name: "source_search".to_string(),
                description: "Search for a text pattern across source files (grep)".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Text or regex pattern to search for"
                        },
                        "glob": {
                            "type": "string",
                            "description": "File glob filter (e.g. *.rs, *.ts). Default: *.rs"
                        }
                    },
                    "required": ["pattern"]
                }),
                capabilities: vec![],
                sandbox_mode: None,
                source: crate::actions::ActionSource::System,
                file_path: None,
            },
            ActionDef {
                name: "build_check".to_string(),
                description: "Run `cargo check` for fast Rust syntax validation".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({"type": "object", "properties": {}}),
                capabilities: vec![],
                sandbox_mode: None,
                source: crate::actions::ActionSource::System,
                file_path: None,
            },
            ActionDef {
                name: "run_tests".to_string(),
                description: "Run `cargo test` to execute unit tests".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({"type": "object", "properties": {}}),
                capabilities: vec![],
                sandbox_mode: None,
                source: crate::actions::ActionSource::System,
                file_path: None,
            },
            ActionDef {
                name: "lint_check".to_string(),
                description: "Run `cargo clippy -- -D warnings` for linting".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({"type": "object", "properties": {}}),
                capabilities: vec![],
                sandbox_mode: None,
                source: crate::actions::ActionSource::System,
                file_path: None,
            },
            ActionDef {
                name: "frontend_build".to_string(),
                description: "Run `npm run build` in the frontend directory".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({"type": "object", "properties": {}}),
                capabilities: vec![],
                sandbox_mode: None,
                source: crate::actions::ActionSource::System,
                file_path: None,
            },
            ActionDef {
                name: "web_search".to_string(),
                description: "Search the web for API documentation, examples, or information".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query"
                        }
                    },
                    "required": ["query"]
                }),
                capabilities: vec![],
                sandbox_mode: None,
                source: crate::actions::ActionSource::System,
                file_path: None,
            },
            ActionDef {
                name: "task_complete".to_string(),
                description: "Signal that implementation is complete. Call this when done.".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "summary": {
                            "type": "string",
                            "description": "Brief summary of what was implemented"
                        }
                    },
                    "required": ["summary"]
                }),
                capabilities: vec![],
                sandbox_mode: None,
                source: crate::actions::ActionSource::System,
                file_path: None,
            },
        ]
    }

    // -----------------------------------------------------------------------
    // Tool dispatch
    // -----------------------------------------------------------------------

    async fn rollback_file_changes(
        &self,
        file_backups: &HashMap<String, Option<String>>,
    ) -> Result<usize> {
        let mut restored_count = 0_usize;
        let mut paths: Vec<&String> = file_backups.keys().collect();
        paths.sort();
        for path in paths {
            if let Some(original) = file_backups.get(path) {
                tools::source_restore(&self.config.project_root, path, original).await?;
                restored_count += 1;
            }
        }
        Ok(restored_count)
    }

    async fn dispatch_tool_call(
        &self,
        call: &ToolCall,
        file_backups: &mut HashMap<String, Option<String>>,
    ) -> Result<String> {
        let root = &self.config.project_root;
        let args = &call.arguments;

        match call.name.as_str() {
            "source_read" => {
                let path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("Missing 'path' parameter"))?;
                tools::source_read(root, path).await
            }
            "source_write" => {
                let path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("Missing 'path' parameter"))?;
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("Missing 'content' parameter"))?;
                if !file_backups.contains_key(path) {
                    let original = tools::source_capture(root, path).await?;
                    file_backups.insert(path.to_string(), original);
                }
                tools::source_write(root, path, content).await
            }
            "source_edit" => {
                let path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("Missing 'path' parameter"))?;
                let search = args
                    .get("search")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("Missing 'search' parameter"))?;
                let replace = args
                    .get("replace")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("Missing 'replace' parameter"))?;
                if !file_backups.contains_key(path) {
                    let original = tools::source_capture(root, path).await?;
                    file_backups.insert(path.to_string(), original);
                }
                tools::source_edit(root, path, search, replace).await
            }
            "source_list" => {
                let path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("Missing 'path' parameter"))?;
                let pattern = args.get("pattern").and_then(|v| v.as_str());
                tools::source_list(root, path, pattern).await
            }
            "source_search" => {
                let pattern = args
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("Missing 'pattern' parameter"))?;
                let glob = args.get("glob").and_then(|v| v.as_str());
                tools::source_search(root, pattern, glob).await
            }
            "build_check" => tools::build_check(root).await,
            "run_tests" => tools::run_tests(root).await,
            "lint_check" => tools::lint_check(root).await,
            "frontend_build" => tools::frontend_build(root).await,
            "web_search" => {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("Missing 'query' parameter"))?;
                tools::web_search(query).await
            }
            "task_complete" => {
                let summary = args
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Task completed");
                Ok(format!("TASK_COMPLETE: {}", summary))
            }
            other => Err(anyhow!("Unknown tool: {}", other)),
        }
    }
}
