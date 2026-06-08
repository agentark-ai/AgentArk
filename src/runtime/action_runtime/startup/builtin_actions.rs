use super::super::*;

impl ActionRuntime {
    /// Load built-in actions
    pub(in crate::runtime) async fn load_builtin_actions(&self) -> Result<()> {
        // File operations
        self.register_builtin_action(ActionDef {
            name: "file_read".to_string(),
            description: "Read a single file from the data-owned workspace, data directory, or other explicitly allowed runtime roots. Text files return UTF-8 content. Binary or non-UTF-8 files return a ResourceRef so the framework can save, pass to code_execute, or refer to the bytes later without converting them to text. Credential-looking files such as runtime env files, .env files, private keys, and credential JSON files are refused; use the secure credential store instead.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to read" }
                },
                "required": ["path"]
            }),
            capabilities: vec!["capability_inventory".to_string(), "file_read".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "file_write".to_string(),
            description: "Author or overwrite a single file in the data-owned workspace or data directory. Relative paths and /workspace-style paths are isolated from the product source checkout. Provide text with content, raw bytes with content_base64, copy an existing allowed file with source_path, or copy any ResourceRef returned by a prior tool with source_resource. Use source_resource when a fetch/browser/http tool produced payload.kind=resource so binary artifacts are saved exactly instead of being converted to text. Parent directories are created if they do not already exist. For generated multi-file services or browser apps, write each file under one data-owned workspace subdirectory, then register the staged directory with service_manage or app_deploy using source_dir so the runnable service is assembled from those files.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to write" },
                    "content": { "type": "string", "description": "UTF-8 text content to write" },
                    "content_base64": { "type": "string", "description": "Base64-encoded bytes to write exactly" },
                    "source_resource": {
                        "description": "ResourceRef object, structured tool payload, resource id, or resource path returned by a prior tool. Copies the resource bytes exactly.",
                        "oneOf": [
                            { "type": "object" },
                            { "type": "string" }
                        ]
                    },
                    "source_path": { "type": "string", "description": "Existing allowed workspace/data path to copy bytes from" },
                    "content_type": { "type": "string", "description": "Optional MIME type for the written file metadata" },
                    "duplicate_policy": {
                        "type": "string",
                        "enum": ["reuse_existing", "create_new"],
                        "default": "reuse_existing",
                        "description": "For document-visible writes, skip Documents ingestion when identical content is already indexed. Use create_new only when the user explicitly wants another duplicate document entry."
                    },
                    "allow_duplicate": {
                        "type": "boolean",
                        "description": "Compatibility boolean for duplicate_policy=create_new. Default false."
                    }
                },
                "required": ["path"]
            }),
            capabilities: vec!["file_write".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "file_delete".to_string(),
            description: "Delete a single managed workspace/data file. Writable paths must resolve inside data-owned or config/action roots; directories and credential-looking files are refused. A missing file returns a terminal not_found result instead of failing so callers can answer without retry loops.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Workspace/data-relative file path or allowed absolute path to delete" }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
            capabilities: vec!["file_write".to_string(), "file_delete".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "file_search".to_string(),
            description: "Search workspace and data-root files without shell access. Use to inspect generated app sources, local documents, logs, artifacts, and explicitly allowed repositories before deciding what to read or modify. Supports filename search, content search, include/exclude globs, context lines, limits, and a root restricted to allowed workspace/data paths. Credential-looking files are skipped. For app follow-up work, use this with ark_inspect when needed to locate the deployed or staged source, then use file_patch for small verified edits.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "x-agentark-semantic-query": true, "description": "Text to search for. In auto mode this is matched against both filenames and file contents." },
                    "filename_query": { "type": "string", "description": "Optional filename/path substring to search for." },
                    "content_query": { "type": "string", "description": "Optional file-content substring to search for." },
                    "mode": { "type": "string", "enum": ["auto", "filename", "content", "both"], "default": "auto", "description": "Search mode. auto searches filenames and contents when query is provided." },
                    "root": { "type": "string", "description": "Optional root directory. Must resolve inside allowed workspace/data roots." },
                    "globs": { "type": "array", "items": { "type": "string" }, "description": "Optional include globs such as [\"src/**/*.rs\", \"*.md\"]." },
                    "exclude_globs": { "type": "array", "items": { "type": "string" }, "description": "Optional exclude globs." },
                    "context_lines": { "type": "integer", "minimum": 0, "maximum": 8, "default": 2, "description": "Number of surrounding lines to return for content matches." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 50, "description": "Maximum number of matches to return." },
                    "case_sensitive": { "type": "boolean", "default": false },
                    "max_file_bytes": { "type": "integer", "minimum": 4096, "maximum": 2000000, "default": 1000000, "description": "Skip content search for files larger than this size." }
                },
                "additionalProperties": false
            }),
            capabilities: vec![
                "capability_inventory".to_string(),
                "file_read".to_string(),
                "file_search".to_string(),
                "search_files".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: read_only_authorization(Default::default()),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "file_patch".to_string(),
            description: "Apply targeted unified diffs to existing workspace/data files. Use after inspecting the current file with file_read, file_search, ark_inspect, or browser evidence, especially for small source edits to generated or deployed app files. Patches must target existing files inside allowed roots, credential-looking paths are refused, unified diff hunks are verified against the current file before writing, and the result returns a structured changed-files summary. Use dry_run to validate a patch without writing.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Single file path to patch. Must resolve inside allowed workspace/data roots." },
                    "patch": { "type": "string", "description": "Unified diff hunks for path. Include context lines so the patch can be verified against the current file." },
                    "patches": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": { "type": "string" },
                                "patch": { "type": "string" }
                            },
                            "required": ["path", "patch"],
                            "additionalProperties": false
                        },
                        "description": "Batch of file patches. Use instead of top-level path/patch when editing multiple files."
                    },
                    "dry_run": { "type": "boolean", "default": false, "description": "Validate all patches and return the changed-files summary without writing." }
                },
                "additionalProperties": false
            }),
            capabilities: vec![
                "file_write".to_string(),
                "file_patch".to_string(),
                "patch".to_string(),
                "apply_patch".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "memory_lookup".to_string(),
            description: "Look up relevant user memory on demand. Use when the answer may depend on prior user facts, preferences, saved links/data, or knowledge base context that is not already in the recent conversation. Memory is not authoritative for mutable runtime state such as currently installed integrations, extension packs, auth status, files, tasks, runs, or queues; verify those through live state-inspection actions. For source-scoped external learnings such as Moltbook, set `external_sources` only when that source is directly relevant.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "x-agentark-semantic-query": true, "description": "What memory or prior context to look up" },
                    "limit": { "type": "integer", "description": "Maximum number of memory hits to return (default: 5)" },
                    "include_semantic": { "type": "boolean", "description": "Include learned semantic facts and constraints from durable memory (default: true)" },
                    "include_structured": { "type": "boolean", "description": "Include structured preferences, user data, and knowledge base context (default: true)" },
                    "include_procedures": { "type": "boolean", "description": "Include learned procedural patterns and workflow guidance (default: true)" },
                    "include_lessons": { "type": "boolean", "description": "Include learned lessons and operating constraints (default: true)" },
                    "external_sources": {
                        "type": "array",
                        "description": "Optional source-scoped external memory surfaces to include only when directly relevant, for example [\"moltbook\"]",
                        "items": { "type": "string" }
                    }
                },
                "required": ["query"]
            }),
            capabilities: vec!["memory".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "document_lookup".to_string(),
            description: "Search indexed documents and uploaded attachments on demand. Use when a question depends on document contents beyond the small excerpts already visible in the prompt, or when the user references uploaded files, attachments, or explicit doc ids like `doc:<id>`.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "x-agentark-semantic-query": true, "description": "The question or search query to run against indexed documents" },
                    "limit": { "type": "integer", "description": "Maximum number of excerpts to return (default: 6)" },
                    "doc_ids": {
                        "type": "array",
                        "description": "Optional document ids to prioritize, for example [\"abcd1234\", \"efgh5678\"]",
                        "items": { "type": "string" }
                    }
                },
                "required": ["query"]
            }),
            capabilities: vec!["documents".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "agentark_capability_lookup".to_string(),
            description: "Search the live AgentArk capability registry with curated AgentArk manual context. Use when the user asks what AgentArk can do, how a feature works, where it is configured, or whether a built-in/plugin/MCP capability exists. The live registry is authoritative; manual text is supplemental explanation. This is read-only; current run logs and object state still require state-inspection actions.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "x-agentark-semantic-query": true, "description": "Question or topic to search in the AgentArk capability registry and manual" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 8, "description": "Maximum registry entries and supplemental manual entries to return per source (default: 4)" },
                    "doc_ids": {
                        "type": "array",
                        "description": "Optional AgentArk knowledge document IDs that scope supplemental manual retrieval.",
                        "items": { "type": "string" }
                    }
                },
                "required": ["query"]
            }),
            capabilities: vec![
                "agentark_capabilities".to_string(),
                "agentark_manual".to_string(),
                "capability_inventory".to_string(),
                "documentation".to_string(),
                "database_readonly".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "session_search".to_string(),
            description: "Search prior conversations, persisted messages, and execution traces in AgentArk's existing history.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query or topic. Leave empty to return recent sessions." },
                    "scope": {
                        "type": "string",
                        "enum": ["all", "conversations", "messages", "traces"],
                        "description": "History area to search"
                    },
                    "conversation_id": { "type": "string", "description": "Optional conversation id to inspect directly" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 25, "description": "Maximum results to return" }
                }
            }),
            capabilities: vec!["session_history".to_string(), "database_readonly".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "vision_ocr".to_string(),
            description: "Analyze an uploaded image/PDF or image/PDF URL. Use for OCR, screenshot understanding, visual document extraction, and image questions in chat or tool flows.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "upload_id": { "type": "string", "description": "Optional uploaded image or PDF id from AgentArk uploads" },
                    "image_url": { "type": "string", "description": "Optional public image or PDF URL" },
                    "file_url": { "type": "string", "description": "Optional public image or PDF URL alias" },
                    "task": {
                        "type": "string",
                        "enum": ["extract_text", "describe", "answer_question", "analyze_document"],
                        "description": "Vision task"
                    },
                    "question": { "type": "string", "description": "Question for answer_question or extra analysis instructions" },
                    "provider": {
                        "type": "string",
                        "enum": ["openai", "google_gemini"],
                        "description": "Optional provider override"
                    },
                    "model": { "type": "string", "description": "Optional provider model override" },
                    "detail": {
                        "type": "string",
                        "enum": ["auto", "low", "high"],
                        "description": "OpenAI image detail level"
                    }
                }
            }),
            capabilities: vec!["vision_ocr".to_string(), "network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        // HTTP requests
        self.register_builtin_action(ActionDef {
            name: "http_get".to_string(),
            description: "Perform an HTTP GET request against a publicly reachable URL and return a typed payload. Text/JSON/HTML are returned inline for inspection and reasoning; binary or non-text resources are persisted as ResourceRefs so they can be saved, read, or passed to later tools without byte corruption. Prefer a dedicated integration action when the user has connected the relevant service rather than treating an authenticated endpoint as plain HTTP.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to fetch" },
                    "headers": { "type": "object", "description": "Optional headers" }
                },
                "required": ["url"]
            }),
            capabilities: vec![
                "network".to_string(),
                "search".to_string(),
                "web_fetch".to_string(),
                "url_fetch".to_string(),
                "external_read".to_string(),
                "public_web".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Wasm),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "page_fetch".to_string(),
            description: "Fetch a single user-provided public URL and return a typed payload for the current workflow. Readable pages and documents return text by default; set as_resource=true when later file, document, skill, app, ingestion, or follow-up steps need the exact fetched bytes as a ResourceRef. Binary or non-text resources return ResourceRefs automatically. Uses a built-in fetch ladder and reports degenerate/empty content as a recoverable failure instead of pretending the page was read.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Public HTTP or HTTPS URL to fetch." },
                    "max_chars": { "type": "integer", "minimum": 1000, "maximum": 50000, "description": "Maximum readable characters to return. Defaults to 12000." },
                    "as_resource": { "type": "boolean", "description": "Return exact response bytes as a managed ResourceRef instead of clipped readable text." },
                    "suggested_name": { "type": "string", "description": "Optional safe filename hint for the managed ResourceRef when as_resource=true." }
                },
                "required": ["url"],
                "additionalProperties": false
            }),
            capabilities: vec![
                "network".to_string(),
                "search".to_string(),
                "page_fetch".to_string(),
                "web_fetch".to_string(),
                "url_fetch".to_string(),
                "external_read".to_string(),
                "public_web".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "http_request".to_string(),
            description: "Execute a direct public HTTP request and return a structured, redacted response summary. Use when the requested outcome is the actual one-off API operation described by the user or by instructions already read in the workflow. Use as_resource=true to return raw response bytes as a ResourceRef without choosing a save_to path; use save_to only when a concrete workspace/data path is needed. Binary or non-text bodies are auto-persisted as ResourceRefs. When a JSON response contains values that must persist for later steps, use persist_response to write selected response fields to durable runtime files without exposing sensitive values in chat or model-visible tool output.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Public HTTP or HTTPS URL to call." },
                    "method": { "type": "string", "enum": ["get", "post", "put", "patch", "delete"], "description": "HTTP method. Defaults to get." },
                    "headers": { "type": "object", "description": "Optional HTTP headers. Values may use {{secret:KEY}} placeholders." },
                    "query": { "type": "object", "description": "Optional query parameters." },
                    "body": { "description": "Optional JSON request body for methods that accept a body." },
                    "timeout_secs": { "type": "integer", "minimum": 1, "maximum": 300, "description": "Request timeout in seconds. Defaults to 30." },
                    "save_to": { "type": "string", "description": "Optional workspace/data file path where the raw response body bytes should be written exactly as returned." },
                    "as_resource": { "type": "boolean", "description": "Return exact response bytes as a managed ResourceRef without requiring a caller-chosen path." },
                    "suggested_name": { "type": "string", "description": "Optional safe filename hint for the managed ResourceRef when as_resource=true or auto-persisting a non-text response." },
                    "persist_response": {
                        "type": "array",
                        "description": "Optional response fields to persist durably after a successful response. Each item extracts a JSON path from the response body and writes it to target_path or encrypted secret_key before response redaction. Use this instead of code_exec when returned credentials or config must survive later turns.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "response_path": { "type": "string", "description": "Dot path into the JSON response body, such as agentId or data.token." },
                                "target_path": { "type": "string", "description": "Durable runtime file path. Use ~/relative paths for runtime-home files." },
                                "secret_key": { "type": "string", "description": "Encrypted custom-secret key for the extracted response value. Use for returned credentials/tokens that must persist without appearing in model-visible output." },
                                "format": { "type": "string", "enum": ["text", "json"], "description": "How to serialize the extracted value. Defaults to text." },
                                "sensitive": { "type": "boolean", "description": "Whether the value is sensitive. Sensitive values are written but never returned in tool output." }
                            },
                            "required": ["response_path"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["url"],
                "additionalProperties": false
            }),
            capabilities: vec![
                "network".to_string(),
                "raw_http".to_string(),
                "external_write".to_string(),
                "durable_file_write".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "lan_discover".to_string(),
            description: "Discover devices and host-local apps on the user's own LAN through a dedicated local-network discovery path. Use for requests like finding Sonos, lights, local devices, or localhost apps. In Docker installs this prefers the authenticated host LAN helper and falls back to degraded container-visible discovery. Discovery is read-only inventory; ask before any device-control action.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "Optional discovery target such as sonos, lights, localhost_apps, apps, devices, or all."
                    },
                    "cidr": {
                        "type": "string",
                        "description": "Optional private IPv4 CIDR scope such as 192.168.1.0/24. Public and broad ranges are rejected."
                    },
                    "max_hosts": {
                        "type": "integer",
                        "description": "Maximum bounded host scope for any CIDR hint. Default 64, hard cap 512."
                    },
                    "include_host_local": {
                        "type": "boolean",
                        "description": "Whether to include host-local app probes. Default true."
                    },
                    "include_http_metadata": {
                        "type": "boolean",
                        "description": "Whether to run light HTTP metadata probes for discovered candidates. Default true."
                    }
                }
            }),
            capabilities: vec![
                "local_network_discovery".to_string(),
                "network".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: ActionAuthorization {
                risk_level: ActionRiskLevel::High,
                requires_auth: true,
                human_approval: crate::actions::ActionHumanApproval { required: true },
                ..Default::default()
            },
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "app_restart".to_string(),
            description: "Restart an existing deployed app from its saved metadata. Use after file_write edits to /app/data/apps/<id>/..., when a deployed app needs reload, or when the user asks to restart or re-run an existing app. Prefer app_id from ark_inspect app-registry results when available; otherwise use query to match an app.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "app_id": {
                        "type": "string",
                        "description": "Exact deployed app ID to restart. Preferred when already known."
                    },
                    "title": {
                        "type": "string",
                        "description": "Optional new app title to persist before restarting. Use when a repurposed app should show a new name in the Apps list."
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional app title or app ID to match when app_id is not known."
                    }
                }
            }),
            capabilities: vec!["app_hosting".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["app_hosting".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "app_stop".to_string(),
            description: "Stop the runtime for an existing deployed app without deleting its files. Use when the user asks to stop, pause, or shut down a deployed app. For repo-based multi-service deployments, `bundle_id` stops all dynamic services in that bundle and skips static ones.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "app_id": {
                        "type": "string",
                        "description": "Exact deployed app ID to stop."
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional app title or app ID to match when app_id is not known."
                    },
                    "bundle_id": {
                        "type": "string",
                        "description": "Optional repo deployment bundle ID to stop all matching dynamic services together."
                    }
                }
            }),
            capabilities: vec!["app_hosting".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["app_hosting".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "app_delete".to_string(),
            description: "Stop and delete an existing deployed app, including its stored files. Use when the user asks to remove, delete, or tear down a deployed app. For repo-based multi-service deployments, `bundle_id` deletes every app in that repo bundle and cleans up the bundle metadata once the last service is gone.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "app_id": {
                        "type": "string",
                        "description": "Exact deployed app ID to delete."
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional app title or app ID to match when app_id is not known."
                    },
                    "bundle_id": {
                        "type": "string",
                        "description": "Optional repo deployment bundle ID to delete all matching services together."
                    }
                }
            }),
            capabilities: vec!["app_hosting".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["app_hosting".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        })
        .await;

        // Shell commands (requires approval by default)
        self.register_builtin_action(ActionDef {
            name: "shell".to_string(),
            description: "Run a single shell command in AgentArk's configured command executor and return combined stdout/stderr plus exit status. Use it as a general terminal primitive for diagnostics, repo setup, build commands, small scripted transformations, or process-oriented work. For ordered diagnostics, run a compact non-interactive script or call shell repeatedly as evidence is discovered; checking versions, installed tools, logs, build output, and test output is expected repair work, not a reason to regenerate an app. The executor may be isolated, so durable artifacts should be written through file tools or registered as managed services when they must remain available after the turn. Use background=true for long-running command work that should outlive the current turn.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Command to execute" },
                    "cwd": { "type": "string", "description": "Working directory" },
                    "background": { "type": "boolean", "description": "Queue this command as durable background work instead of blocking the current turn. The command runs through the same action path later with background=false." },
                    "notify_on_complete": { "type": "boolean", "description": "When background=true, record that the user should be notified when the queued task completes." }
                },
                "required": ["command"]
            }),
            capabilities: vec!["shell".to_string()],
            sandbox_mode: Some(SandboxMode::Docker),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        // Clipboard
        self.register_builtin_action(ActionDef {
            name: "clipboard_read".to_string(),
            description: "Read the current text on the host system clipboard and return it as a UTF-8 string. Useful when the user has just copied something such as a snippet, a URL, or a structured payload, and wants the assistant to operate on that exact content without retyping it. Returns the clipboard's text contents, or an empty string when the clipboard does not currently hold text.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            capabilities: vec!["clipboard_read".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "clipboard_write".to_string(),
            description: "Replace the host system clipboard's text contents with the provided string so the user can paste it elsewhere immediately. Useful when the assistant has produced a snippet, command, address, or structured value the user wants to use outside the conversation. The full text body is required; existing clipboard content is overwritten.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string", "description": "Content to copy" }
                },
                "required": ["content"]
            }),
            capabilities: vec!["clipboard_write".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "current_time".to_string(),
            description: "Return the current date and time without using any external integration. Use for date-based reminders, time checks, and internal automation scheduling logic.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "timezone": {
                        "type": "string",
                        "description": "Optional IANA timezone such as 'Asia/Kolkata' or 'America/New_York'. Defaults to UTC."
                    }
                }
            }),
            capabilities: vec!["time".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "notify_user".to_string(),
            description: format!(
                "Return a notification message for internal reminder/scheduler delivery. Use for reminders and nudges that should be delivered through {}'s delivery routing instead of an external data source.",
                crate::branding::PRODUCT_NAME
            ),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "Notification body to deliver"
                    },
                    "title": {
                        "type": "string",
                        "description": "Optional title for the reminder"
                    },
                    "in_app_title": {
                        "type": "string",
                        "description": "Optional title for the local notification record. Does not change the external message body."
                    },
                    "source": {
                        "type": "string",
                        "description": "Optional semantic notification source/type for the local notification record, such as reminder. Scheduled reminder tasks set this automatically."
                    },
                    "delivery_channel": {
                        "type": "string",
                        "description": "Optional delivery route for direct chat/API notification requests. Use preferred for the runtime fallback chain, in_app for local-only delivery, or a requested channel such as telegram or whatsapp. Scheduled tasks should use schedule_task.report_to instead."
                    }
                },
                "required": ["message"]
            }),
            capabilities: vec!["notify".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                channel_targets: vec![channel_target("delivery_channel", "preferred")],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        })
        .await;

        // Scheduler
        self.register_builtin_action(ActionDef {
            name: "schedule_task".to_string(),
            description: "Schedule or update durable recurring/one-time AgentArk task records whose execution is intentionally deferred until specified times or recurrences. The result is asynchronous task record(s) that run later and report through the selected delivery route; it is not a substitute for an immediate action that returns the requested result during the current turn. Do not use this for cadence that belongs inside a generated app, dashboard, page, or tool, such as its own refresh, polling, auto-update, or live-data display behavior; keep that behavior in the service artifact unless the user wants AgentArk to run or notify independently outside the artifact. Create the task directly from the task body, cadence, selected action/script, validation policy, and reporting route. When the user asks for multiple independent future notifications, reminders, appointments, or scheduled outcomes in one request, use `items` with one item per outcome instead of collapsing them into one task. Use `task_id` when changing an existing task from `list_tasks`; otherwise matching tasks are updated/reused unless allow_duplicate=true. Use cron for recurring schedules, at/scheduled_for for fully known one-time ISO timestamps, or local_time plus timezone for wall-clock times that should be resolved from runtime temporal context. Prefer local_time over manual date arithmetic when the user supplied a time without a full date. The schedule value is the run or notification time; for reminders before an event, schedule the notification at the lead-time offset before the event and keep the event details in task/action_arguments. A recurring cron has no expiry unless the user gives an end policy. If notification should happen only after a condition, material change, or trigger match, schedule a recurring action/script with validation and report only when that validation succeeds. If the exact schedule needed to honor the user's requested reminder cannot be inferred, ask for the missing timing detail instead of creating a guess.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                      "task": { "type": "string", "description": "Task description - what to do" },
                      "task_id": { "type": "string", "description": "Optional existing task ID to update. Use this after `list_tasks` or when the user explicitly references an existing routine/task." },
                      "cron": { "type": "string", "description": "Cron expression for recurring tasks. Minute granularity only for schedule_task. Format: 'minute hour day month weekday'. This is the time AgentArk runs or notifies, not necessarily the event start time. For advance reminders, schedule the reminder at the offset time before the event. Recurring cron schedules continue until the user cancels or changes them unless the task itself encodes a different policy. Examples: '0 9 * * *' = daily at 9am, '45 8 * * 1' = every Monday at 8:45am, '*/30 * * * *' = every 30 minutes" },
                      "at": { "type": "string", "description": "ISO 8601 timestamp for one-time task. This is the time AgentArk runs or notifies. For advance reminders, use the offset timestamp before the event. Example: '2026-02-06T09:00:00+05:30'" },
                      "scheduled_for": { "type": "string", "description": "ISO 8601 one-time schedule value returned by persisted scheduled-task records. Equivalent to at when updating or recreating a task from an existing task/status payload." },
                      "local_time": { "type": "string", "description": "Local wall-clock time for a one-time schedule, such as '00:22' or '12:22 AM'. Prefer this with timezone when the user gave a time without a full date; AgentArk resolves the date deterministically from the current temporal context and any existing task being updated." },
                      "local_date": { "type": "string", "description": "Optional local calendar date for local_time in YYYY-MM-DD. Omit for next occurrence, or when updating an existing one-time task and only the wall-clock time changed." },
                      "timezone": { "type": "string", "description": "IANA timezone for local_time/local_date, such as Asia/Kolkata. Defaults to the user's profile timezone when available." },
                      "timezone_offset_minutes": { "type": "integer", "description": "Optional numeric UTC offset in minutes for local_time/local_date when an IANA timezone is unavailable." },
                      "date_policy": { "type": "string", "enum": ["existing_local_date", "next_occurrence", "same_local_date"], "description": "Resolution policy for local_time without local_date. existing_local_date is default for task_id updates, next_occurrence is default for new tasks, same_local_date forces today's local date." },
                      "items": {
                          "type": "array",
                          "description": "Batch of independent scheduled outcomes. Use one item for each distinct future task/reminder/appointment requested in the same turn. Top-level report_to, action, action_arguments, script, script_language, context_from, workdir, validation, automation_policy, max_attempts, stall_timeout_secs, retry_backoff_secs, and allow_duplicate are inherited by items unless overridden.",
                          "items": {
                              "type": "object",
                              "properties": {
                                  "task": { "type": "string", "description": "Task description for this scheduled outcome" },
                                  "task_id": { "type": "string", "description": "Optional existing task ID to update for this item" },
                                  "cron": { "type": "string", "description": "Cron expression for this recurring task, at the run/notification time" },
                                  "at": { "type": "string", "description": "ISO 8601 timestamp for this one-time task, at the run/notification time" },
                                  "scheduled_for": { "type": "string", "description": "ISO 8601 one-time schedule value returned by persisted scheduled-task records. Equivalent to at." },
                                  "local_time": { "type": "string", "description": "Local wall-clock time for this one-time scheduled item" },
                                  "local_date": { "type": "string", "description": "Optional local date in YYYY-MM-DD for local_time" },
                                  "timezone": { "type": "string", "description": "IANA timezone for local_time/local_date" },
                                  "timezone_offset_minutes": { "type": "integer" },
                                  "date_policy": { "type": "string", "enum": ["existing_local_date", "next_occurrence", "same_local_date"] },
                                  "action": { "type": "string", "description": "Optional explicit action name for this item" },
                                  "action_arguments": { "type": "object", "description": "Optional explicit arguments for this item's action" },
                                  "script": { "type": "string", "description": "Optional script body to run for this item through code_execute" },
                                  "script_language": { "type": "string", "description": "Language for script, default python" },
                                  "context_from": { "type": "array", "items": { "type": "string" } },
                                  "workdir": { "type": "string" },
                                  "report_to": { "type": "string", "description": "Optional notification route override for this item" },
                                  "allow_duplicate": { "type": "boolean", "description": "Create this item separately even if a matching task already exists" },
                                  "validation": { "type": "object" },
                                  "max_attempts": { "type": "integer" },
                                  "stall_timeout_secs": { "type": "integer" },
                                  "retry_backoff_secs": { "type": "integer" },
                                  "automation_policy": { "type": "object" }
                              },
                              "oneOf": [
                                  { "required": ["task", "cron"] },
                                  { "required": ["task", "at"] },
                                  { "required": ["task", "scheduled_for"] },
                                  { "required": ["task", "local_time"] },
                                  { "required": ["task_id", "cron"] },
                                  { "required": ["task_id", "at"] },
                                  { "required": ["task_id", "scheduled_for"] },
                                  { "required": ["task_id", "local_time"] },
                                  { "required": ["action_arguments", "cron"] },
                                  { "required": ["action_arguments", "at"] },
                                  { "required": ["action_arguments", "scheduled_for"] },
                                  { "required": ["action_arguments", "local_time"] }
                              ]
                          }
                      },
                      "action": { "type": "string", "description": "Optional explicit action name to run for each task occurrence" },
                      "action_arguments": { "type": "object", "description": "Optional explicit arguments for the selected action" },
                    "script": { "type": "string", "description": "Optional script body to run at each scheduled occurrence through code_execute. Use when the durable job needs data collection or computation before reporting." },
                    "script_language": { "type": "string", "description": "Language for script, default python. Passed to code_execute when script is supplied." },
                    "context_from": { "type": "array", "items": { "type": "string" }, "description": "Optional context sources or artifact IDs the scheduled job should resolve at run time." },
                    "workdir": { "type": "string", "description": "Optional working directory hint for script-based scheduled jobs." },
                    "catchup_window_secs": { "type": "integer", "description": "Maximum lateness window for catching up missed scheduled runs." },
                    "report_to": { "type": "string", "description": "Notification route for results. Use 'preferred' for any connected channel. Use a named channel only when the user explicitly requests that target; AgentArk preserves the requested route and keeps results in app until the named channel is connected." },
                    "allow_duplicate": { "type": "boolean", "description": "Create a separate task even if a matching one already exists. Default false: matching tasks are updated/reused." },
                    "validation": {
                        "type": "object",
                        "description": "Optional generic validation policy for each run",
                        "properties": {
                            "mode": { "type": "string", "enum": ["none", "non_empty_result", "structured_success", "contains_text", "regex_match", "json_field_exists", "json_field_equals", "json_array_non_empty"] },
                            "text": { "type": "string" },
                            "field_path": { "type": "string" },
                            "expected": {},
                            "pattern": { "type": "string" }
                        }
                    },
                    "max_attempts": { "type": "integer", "description": "Maximum supervised retry attempts" },
                    "stall_timeout_secs": { "type": "integer", "description": "Maximum seconds a single run may take before timing out" },
                    "retry_backoff_secs": { "type": "integer", "description": "Base backoff before retrying failed runs" },
                    "automation_policy": {
                        "type": "object",
                        "description": "Advanced automation execution policy override",
                        "properties": {
                            "max_attempts": { "type": "integer" },
                            "stall_timeout_secs": { "type": "integer" },
                            "retry_backoff_secs": { "type": "integer" },
                            "validation": { "type": "object" }
                        }
                    }
                },
                  "oneOf": [
                      { "required": ["task", "cron"] },
                      { "required": ["task", "at"] },
                      { "required": ["task", "scheduled_for"] },
                      { "required": ["task", "local_time"] },
                      { "required": ["task_id", "cron"] },
                      { "required": ["task_id", "at"] },
                      { "required": ["task_id", "scheduled_for"] },
                      { "required": ["task_id", "local_time"] },
                      { "required": ["action_arguments", "cron"] },
                      { "required": ["action_arguments", "at"] },
                      { "required": ["action_arguments", "scheduled_for"] },
                      { "required": ["action_arguments", "local_time"] },
                      { "required": ["items"] }
                  ]
              }),
            capabilities: vec!["scheduler".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                channel_targets: vec![channel_target("report_to", "preferred")],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "work_manage".to_string(),
            description: "Inspect or modify ongoing AgentArk background work. Use this generic primitive for status, list, pause, resume, stop/cancel, delete, or notification-channel changes for durable work created by scheduled tasks, background executions, monitoring jobs, or long-running sessions. Resolve by work_id when known; otherwise pass a semantic reference in reference_text so AgentArk can match recent work. Use schedule_task to create future/recurring work, code_execute or shell with background=true for long-running execution, and service_manage for durable runnable apps/services.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["status", "list", "pause", "resume", "stop", "cancel", "delete", "update_delivery"],
                        "description": "Work-management operation. stop/cancel closes the work and cancels linked pending jobs; delete removes the work record and linked work records."
                    },
                    "work_id": {
                        "type": "string",
                        "description": "Optional exact work/background-session id."
                    },
                    "reference_text": {
                        "type": "string",
                        "description": "Semantic reference to the target background work when no id is supplied."
                    },
                    "delivery_channel": {
                        "type": "string",
                        "description": "Required for update_delivery. Use preferred, in_app, telegram, whatsapp, or another configured channel only when requested."
                    },
                    "include_closed": {
                        "type": "boolean",
                        "description": "Include completed/cancelled/failed work when listing or resolving."
                    }
                },
                "required": ["operation"]
            }),
            capabilities: vec![
                "background_work".to_string(),
                "scheduler".to_string(),
                "notification".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec![
                    "background_session".to_string(),
                    "scheduler".to_string(),
                ],
                channel_targets: vec![channel_target("delivery_channel", "preferred")],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        // Background watcher - poll an action until a condition is met, then act
        // Tunnel control for remote UI access
        self.register_builtin_action(ActionDef {
            name: "tunnel_control".to_string(),
            description: "Manage remote UI access. Use action=start to create an access URL, action=status to check the current URL, and action=stop to disable it. Optionally pass provider=cloudflare|tailscale_private|tailscale_funnel|ngrok|bore when starting.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["start", "stop", "status"], "description": "Tunnel operation" },
                    "provider": { "type": "string", "description": "Optional provider id for start: cloudflare, tailscale_private, tailscale_funnel, ngrok, or bore." },
                    "allow_duplicate": { "type": "boolean", "description": "Repeat an identical tunnel command in the same request. Default false." }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string(), "search".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;
        self.register_builtin_action(ActionDef {
            name: "watch".to_string(),
            description: "Spawn or update durable background watcher(s) that observe a semantic target at regular intervals until structured conditions are met, then execute follow-up instructions. Use this when the requested outcome is conditional monitoring, trigger-on-change detection, sub-minute polling, or a long-running watch that should notify later. Do not use this for polling or refresh cadence that belongs inside a generated app, dashboard, page, or tool's own UI/data flow; implement that in the artifact unless the user wants AgentArk to monitor or notify independently outside the artifact. Use schedule_task instead when the trigger is purely a known date/time or recurrence and no external condition needs polling. Create watcher records directly from the target description, condition, cadence, timeout, and notification policy; poll_action/script are optional advanced overrides when the caller intentionally supplies low-level poll plumbing. When the caller provides description, condition, and on_trigger without poll_action or script, AgentArk creates a semantic poll contract and resolves suitable read-only polling at runtime. When the user asks for multiple independent watches in one request, use `items` with one item per watcher so item-specific targets, conditions, timeouts, cadences, and notification routes are preserved. Use `watcher_id` when changing an existing watcher from `list_watchers`; otherwise matching watchers are updated/reused unless allow_duplicate=true. The watcher runs autonomously and notifies the user when triggered or timed out. Set repeat_on_match=true when the user's intended outcome is an ongoing monitor that should remain active after a match; leave it false for a one-shot alert. For sources that can return the same matched item repeatedly, choose a change-based condition so repeat watchers do not send duplicate notifications for unchanged output. Default duration is 24 hours; use until_stopped=true for watches with no expiry. If the semantic target, condition, cadence, trigger mode, or required delivery route is too vague to infer from the user's intent, ask for that missing item-specific detail instead of creating a guess.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                      "description": { "type": "string", "description": "What this watcher observes and why (shown in UI). With condition and on_trigger, this is enough for AgentArk to create a semantic watcher when no low-level poll_action/script is supplied." },
                      "watcher_id": { "type": "string", "description": "Optional existing watcher ID to update. Use this after `list_watchers` or when the user explicitly references an existing watcher." },
                      "poll_action": { "type": "string", "description": "Optional advanced override: exact action to poll when the watcher should use a known low-level read action." },
                      "poll_arguments": { "type": "object", "description": "Arguments for the optional low-level poll action" },
                      "script": { "type": "string", "description": "Optional advanced override: poll script body. When supplied without poll_action, AgentArk polls through code_execute with this script." },
                      "script_language": { "type": "string", "description": "Language for script, default python. Passed to code_execute when script is supplied." },
                      "context_from": { "type": "array", "items": { "type": "string" }, "description": "Optional context sources or artifact IDs the watcher poll should resolve at run time." },
                      "workdir": { "type": "string", "description": "Optional working directory hint for script-based watcher polls." },
                      "items": {
                          "type": "array",
                          "description": "Batch of independent watcher outcomes. Use one item for each distinct watch requested in the same turn. Top-level condition, interval_secs, timeout fields, notify_channel, repeat_on_match, on_trigger, validation, automation_policy, max_attempts, stall_timeout_secs, retry_backoff_secs, allow_duplicate, and optional low-level poll fields are inherited by items unless overridden.",
                          "items": {
                              "type": "object",
                              "properties": {
                                  "description": { "type": "string", "description": "What this watcher item observes and why (shown in UI). With condition and on_trigger, this is enough for AgentArk to create a semantic watcher item when no low-level poll_action/script is supplied." },
                                  "watcher_id": { "type": "string", "description": "Optional existing watcher ID to update for this item." },
                                  "poll_action": { "type": "string", "description": "Optional advanced override: exact action to poll for this item when the watcher should use a known low-level read action." },
                                  "poll_arguments": { "type": "object", "description": "Arguments for this item's optional low-level poll action." },
                                  "script": { "type": "string", "description": "Optional advanced override: poll script body for this item. When supplied without poll_action, AgentArk polls through code_execute with this script." },
                                  "script_language": { "type": "string", "description": "Language for this item's script, default python." },
                                  "context_from": { "type": "array", "items": { "type": "string" } },
                                  "workdir": { "type": "string" },
                                  "condition": { "type": "object", "description": "Structured trigger condition for this watcher item." },
                                  "on_trigger": { "type": "string", "description": "What to do when this watcher item's condition is met." },
                                  "interval_secs": { "type": "integer" },
                                  "timeout_secs": { "type": "integer" },
                                  "timeout_hours": { "type": "integer" },
                                  "timeout_days": { "type": "integer" },
                                  "until_stopped": { "type": "boolean" },
                                  "notify_channel": { "type": "string" },
                                  "repeat_on_match": { "type": "boolean" },
                                  "allow_duplicate": { "type": "boolean" },
                                  "validation": { "type": "object" },
                                  "max_attempts": { "type": "integer" },
                                  "stall_timeout_secs": { "type": "integer" },
                                  "retry_backoff_secs": { "type": "integer" },
                                  "automation_policy": { "type": "object" }
                              },
                              "oneOf": [
                                  { "required": ["description", "condition", "on_trigger"] },
                                  { "required": ["description", "poll_action", "condition", "on_trigger"] },
                                  { "required": ["description", "script", "condition", "on_trigger"] },
                                  { "required": ["watcher_id"] }
                              ]
                          }
                      },
                      "condition": {
                        "type": "object",
                        "description": "Structured trigger condition authored by the model. Include a human-readable `description` and an explicit matcher. Prefer `json_predicate` or `json_logic` for structured poll outputs; use `llm` only when the trigger cannot be expressed safely as a deterministic contract.",
                        "properties": {
                            "description": { "type": "string", "description": "Human-readable summary of what counts as a match. For change-detection watchers, state the material difference to compare against the previous successful poll." },
                            "evaluation_mode": { "type": "string", "enum": ["current_state", "change"], "description": "Use current_state when the present poll result alone should trigger. Use change when the watcher should first establish a baseline and only trigger when a later successful poll materially differs while still satisfying the matcher; prefer change for repeat watchers whose poll source may return already-seen matches." },
                            "type": { "type": "string", "enum": ["not_empty", "text_contains", "regex", "json_predicate", "json_logic", "llm"] },
                            "text": { "type": "string", "description": "Used by `text_contains`" },
                            "case_sensitive": { "type": "boolean", "description": "Optional flag for `text_contains`" },
                            "pattern": { "type": "string", "description": "Used by `regex`" },
                            "path": { "type": "string", "description": "Dot-path into the structured poll result. Use `$` or empty for the root object." },
                            "operator": { "type": "string", "enum": ["exists", "not_exists", "eq", "ne", "gt", "gte", "lt", "lte", "contains", "not_contains", "non_empty", "empty", "true", "false", "regex"] },
                            "value": { "description": "Comparison value for operators that require one" },
                            "logic": { "type": "string", "enum": ["all", "any"], "description": "Used by `json_logic` to combine rules" },
                            "rules": {
                                "type": "array",
                                "description": "Used by `json_logic`",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "path": { "type": "string" },
                                        "operator": { "type": "string", "enum": ["exists", "not_exists", "eq", "ne", "gt", "gte", "lt", "lte", "contains", "not_contains", "non_empty", "empty", "true", "false", "regex"] },
                                        "value": {}
                                    },
                                    "required": ["path", "operator"]
                                }
                            }
                        },
                        "required": ["description", "evaluation_mode", "type"]
                    },
                    "on_trigger": { "type": "string", "description": "What to do when condition is met - natural language instructions for the agent" },
                    "interval_secs": { "type": "integer", "description": "Seconds between polls, including sub-minute monitoring intervals (default: 60)" },
                    "timeout_secs": { "type": "integer", "description": "Max seconds to watch before giving up (default: 86400 = 24 hours)" },
                    "timeout_hours": { "type": "integer", "description": "Convenience timeout override in hours. Supports very large values." },
                    "timeout_days": { "type": "integer", "description": "Convenience timeout override in days. Supports very large values." },
                    "until_stopped": { "type": "boolean", "description": "Keep watching until the user stops it. Internally stored as a very large timeout." },
                    "notify_channel": { "type": "string", "description": "Notification route. Use 'preferred' by default so AgentArk can use any connected messaging channel. Use a named channel only when the user explicitly requested that target; AgentArk preserves the requested route and keeps updates in app until the named channel is connected. Use 'in_app' for web-only notifications." },
                    "repeat_on_match": { "type": "boolean", "description": "Keep polling after a matched condition and send later notifications for later matches. Set true for ongoing monitoring; set false when a single matching notification should complete the watcher." },
                    "allow_duplicate": { "type": "boolean", "description": "Create a separate watcher even if a matching one already exists. Default false: matching watchers are updated/reused." },
                    "validation": {
                        "type": "object",
                        "description": "Optional validation policy for successful poll results",
                        "properties": {
                            "mode": { "type": "string", "enum": ["none", "non_empty_result", "structured_success", "contains_text", "regex_match", "json_field_exists", "json_field_equals", "json_array_non_empty"] },
                            "text": { "type": "string" },
                            "field_path": { "type": "string" },
                            "expected": {},
                            "pattern": { "type": "string" }
                        }
                    },
                    "max_attempts": { "type": "integer", "description": "Maximum supervised retry attempts for the follow-up trigger action" },
                    "stall_timeout_secs": { "type": "integer", "description": "Maximum seconds the trigger follow-up may run before timing out" },
                    "retry_backoff_secs": { "type": "integer", "description": "Base backoff before retrying failed trigger follow-ups" },
                    "automation_policy": {
                        "type": "object",
                        "description": "Advanced automation execution policy override",
                        "properties": {
                            "max_attempts": { "type": "integer" },
                            "stall_timeout_secs": { "type": "integer" },
                            "retry_backoff_secs": { "type": "integer" },
                            "validation": { "type": "object" }
                        }
                    }
                },
                  "oneOf": [
                      { "required": ["description", "condition", "on_trigger"] },
                      { "required": ["description", "poll_action", "condition", "on_trigger"] },
                      { "required": ["description", "script", "condition", "on_trigger"] },
                      { "required": ["watcher_id"] },
                      { "required": ["items"] }
                  ]
              }),
            capabilities: vec!["watcher".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["watcher".to_string()],
                channel_targets: vec![channel_target("notify_channel", "preferred")],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "delegate".to_string(),
            description: "Coordinate a request across multiple specialized agent workstreams and synthesize one final answer. Use for work whose desired outcome benefits from independent research, implementation analysis, validation, risk review, or other parallel specialist perspectives before consolidation.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task": { "type": "string", "description": "The complete user request to delegate, including all required sub-questions, constraints, and desired final output." },
                    "tasks": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional explicit independent workstreams to delegate before synthesis. Use this instead of task when the request already contains separable subtasks."
                    },
                    "context": { "type": "string", "description": "Optional conversation or business context that delegated agents should use." },
                    "final_output": { "type": "string", "description": "Optional shape of the consolidated final answer, such as operator-ready plan, recommendation, launch plan, or risk report." }
                },
                "anyOf": [
                    { "required": ["task"] },
                    { "required": ["tasks"] }
                ]
            }),
            capabilities: vec![
                "swarm".to_string(),
                "delegate".to_string(),
                "multi_agent".to_string(),
                "agent_orchestration".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["swarm".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "capability_acquire".to_string(),
            description: "Scaffold a reusable capability when the needed capability does not already exist. HTTP/API capabilities are saved as custom API integrations so they appear in Settings > Integrations and register generated API actions. Prefer this for official HTTP, REST, and GraphQL provider APIs when no built-in or extension-pack integration is available, even if a community MCP wrapper exists. Do not create user skills for API integrations; use skill import/create only when the user is explicitly working with a skill source. Do not use this for extension-pack integrations or connector installs; use the extension_pack_* actions for those.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Stable capability/integration id. If omitted, it is derived from name." },
                    "name": { "type": "string", "description": "Action name to create in kebab-case" },
                    "description": { "type": "string", "description": "What the new capability should do" },
                    "kind": { "type": "string", "enum": ["rest_api", "oauth_api", "openapi", "web_automation"], "description": "Scaffold mode" },
                    "base_url": { "type": "string", "description": "Base URL for the provider/API" },
                    "method": { "type": "string", "enum": ["get", "post", "put", "patch", "delete"], "description": "Primary HTTP method" },
                    "path": { "type": "string", "description": "Primary path or endpoint path" },
                    "required_inputs": { "type": "array", "items": { "type": "string" }, "description": "Runtime inputs the generated action should require" },
                    "auth_type": { "type": "string", "enum": ["none", "bearer", "api_key_header", "api_key_query", "oauth2", "basic"], "description": "Primary auth strategy. Use api_key_header plus auth_header_name=Authorization for API keys sent directly in the Authorization header; use bearer only when the provider requires a Bearer-prefixed token." },
                    "auth_mode": { "type": "string", "enum": ["none", "bearer", "api_key_header", "api_key_query", "oauth2", "basic"], "description": "Alias for auth_type when updating from a saved custom API record." },
                    "auth_secret_name": { "type": "string", "description": "Optional provider credential label from docs. Custom API integrations store credentials under the integration connection; users do not need to know or enter an internal secret storage key." },
                    "auth_header_name": { "type": "string", "description": "Header name for api_key_header auth, such as Authorization when docs show Authorization: <API_KEY> without a Bearer prefix." },
                    "auth_header": { "type": "string", "description": "Alias for auth_header_name when updating from a saved custom API record." },
                    "auth_name": { "type": "string", "description": "Stored auth parameter/header name from a saved custom API record." },
                    "default_headers": { "type": "object", "description": "Static default headers" },
                    "default_query": { "type": "object", "description": "Static default query params" },
                    "body": { "description": "Default JSON request body for an operation only when copied from authoritative provider docs, OpenAPI, or another supplied source. Do not invent provider-specific bodies from memory or guesses." },
                    "body_template": { "description": "Request body template only when copied from authoritative provider docs, OpenAPI, or another supplied source. Do not invent provider-specific templates." },
                    "operation": { "type": "object", "description": "Single operation contract only when source-backed by docs/OpenAPI or an existing saved integration record." },
                    "operations": { "type": "array", "items": { "type": "object" }, "description": "Multiple operation contracts only when source-backed by docs/OpenAPI. Without source evidence, install/update must stop and request docs or reliable search configuration." },
                    "read_only": { "type": "boolean", "description": "Whether the generated operation only retrieves data and must not perform external writes. GraphQL read operations are validated at execution time." },
                    "pagination": { "type": "object", "description": "connector_request pagination configuration" },
                    "response_notes": { "type": "string", "description": "How the action should summarize/return results" },
                    "source_notes": { "type": "string", "description": "OpenAPI/docs notes to preserve in the scaffold" },
                    "source": { "type": "string", "description": "Optional user-supplied API source: URL, OpenAPI/Swagger document text, provider documentation text, or sample curl command. AgentArk classifies the source before saving operations." },
                    "curl_text": { "type": "string", "description": "Optional sample curl command to import." },
                    "openapi_url": { "type": "string", "description": "Optional URL to an OpenAPI/Swagger JSON document" },
                    "openapi_text": { "type": "string", "description": "Inline OpenAPI/Swagger JSON content" },
                    "docs_url": { "type": "string", "description": "Optional provider documentation URL" },
                    "docs_text": { "type": "string", "description": "Inline documentation or API notes" },
                    "force": { "type": "boolean", "description": "Request installation after non-blocking warnings. Blocking security findings still prevent loading." },
                    "allow_duplicate": { "type": "boolean", "description": "Create another matching capability scaffold instead of updating/reusing an existing one. Default false." }
                },
                "anyOf": [
                    { "required": ["name"] },
                    { "required": ["id"] }
                ]
            }),
            capabilities: vec!["integration_builder".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["capability_acquire".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "custom_api_request".to_string(),
            description: "Execute a saved read-only custom API operation using the credentials stored with that Custom API integration. Use after inspect_integration/list_integrations identifies the saved custom API id and operation. This is for reads/queries only; state-changing custom API operations must use their generated action and normal approval path.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Saved custom API id." },
                    "query": { "type": "string", "description": "Optional saved custom API id lookup text when id is unavailable." },
                    "operation": { "type": "string", "description": "Operation id, generated action name, method/path label, or operation name." },
                    "operation_id": { "type": "string", "description": "Operation id." },
                    "action_name": { "type": "string", "description": "Generated custom API action name." },
                    "body": { "description": "JSON request body for operations that require a body." },
                    "arguments": { "type": "object", "description": "Explicit generated action arguments. If omitted, remaining request fields become action arguments." }
                },
                "required": ["id"],
                "additionalProperties": true
            }),
            capabilities: vec!["custom_api".to_string(), "integration".to_string(), "network".to_string(), "external_read".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "custom_api_manage".to_string(),
            description: "Delete, enable, or disable an existing saved Custom API integration. Use only when the user intends to manage the saved integration record, not to call the provider API.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Saved custom API id." },
                    "operation": { "type": "string", "enum": ["delete", "enable", "disable"] }
                },
                "required": ["id", "operation"],
                "additionalProperties": false
            }),
            capabilities: vec!["custom_api".to_string(), "integration_builder".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["capability_acquire".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "capability_resolve".to_string(),
            description: "Inspect a user goal, attached files, and prior tool failures to choose the safest next capability path. Use before giving up when a request needs missing packages, binaries, codecs, file-type detection, media conversion/transcription, app/repo repair, connector scaffolding, downloads, or another acquired capability. Returns structured JSON with detected inputs, missing capabilities, a sandbox-first acquisition route, optional approval metadata, and suggested next tool calls; it does not run host installers itself.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "goal": { "type": "string", "description": "The user's actual goal or the blocked subgoal to resolve." },
                    "files": { "type": "array", "items": { "type": "string" }, "description": "Upload IDs returned by /api/upload. The resolver validates them and sniffs file bytes rather than trusting filename/content type." },
                    "file_payloads": {
                        "type": "array",
                        "description": "Inline file payloads for executor/control-plane callers.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "filename": { "type": "string" },
                                "content_type": { "type": "string" },
                                "bytes_b64": { "type": "string" }
                            },
                            "required": ["filename", "bytes_b64"]
                        }
                    },
                    "failure_output": { "type": "string", "description": "Raw stderr/stdout or tool output from a failed attempt. Use this to detect missing binaries/packages and choose the next route." },
                    "selected_action": { "type": "string", "description": "Optional exact action name already selected from the action catalog. This is a catalog signal, not a natural-language intent label." },
                    "requested_capability": { "type": "string", "description": "Optional opaque capability label from the model/action selector or a concrete missing binary/package name. The resolver records this as context but does not classify natural-language intent from it." }
                },
                "required": ["goal"]
            }),
            capabilities: vec![
                "file_read".to_string(),
                "capability_inventory".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // Generic connector scaffold: pagination + rate-limit + auth-refresh + retries.
        self.register_builtin_action(ActionDef {
            name: "connector_request".to_string(),
            description: "Run an explicit HTTP connector request for API/data collection workflows that need pagination, rate-limit spacing, auth-refresh callbacks, or retry/backoff behavior. Use http_request for a one-off user-requested API operation, especially when selected response fields must be persisted. Do not create extra state-changing resources merely to learn a response schema; use documentation, read-only calls, or the requested operation's actual result.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Target URL" },
                    "method": { "type": "string", "enum": ["get", "post", "put", "patch", "delete"], "description": "HTTP method (default: get)" },
                    "headers": { "type": "object", "description": "HTTP headers" },
                    "query": { "type": "object", "description": "Query params" },
                    "body": { "description": "Optional JSON body" },
                    "timeout_secs": { "type": "integer", "description": "Per-request timeout seconds (default: 30)" },
                    "rate_limit_ms": { "type": "integer", "description": "Min delay between requests/pages in ms" },
                    "retry": {
                        "type": "object",
                        "properties": {
                            "max_attempts": { "type": "integer" },
                            "initial_backoff_ms": { "type": "integer" },
                            "max_backoff_ms": { "type": "integer" },
                            "jitter_ratio": { "type": "number" },
                            "retry_on_status": { "type": "array", "items": { "type": "integer" } }
                        }
                    },
                    "pagination": {
                        "type": "object",
                        "properties": {
                            "mode": { "type": "string", "enum": ["none", "page", "cursor"] },
                            "page_param": { "type": "string" },
                            "cursor_param": { "type": "string" },
                            "items_path": { "type": "string" },
                            "next_cursor_path": { "type": "string" },
                            "start_page": { "type": "integer" },
                            "max_pages": { "type": "integer" },
                            "page_size_param": { "type": "string" },
                            "page_size": { "type": "integer" }
                        }
                    },
                    "auth_refresh": {
                        "type": "object",
                        "properties": {
                            "action": { "type": "string", "description": "Action to call on auth expiry (401/403)" },
                            "arguments": { "description": "Arguments for refresh action" },
                            "retry_statuses": { "type": "array", "items": { "type": "integer" } }
                        },
                        "required": ["action"]
                    }
                },
                "required": ["url"]
            }),
            capabilities: vec!["network".to_string(), "raw_http".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["browser_auto".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        // First-class pipeline DAG spec compiler.
        self.register_builtin_action(ActionDef {
            name: "pipeline_compile".to_string(),
            description: "Validate and compile a pipeline DAG spec (dependency checks + topological order). Optionally persist the spec for scheduled runs.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "spec": { "type": "object", "description": "Pipeline spec" },
                    "save": { "type": "boolean", "description": "Persist spec to storage (default: true)" }
                },
                "required": ["spec"]
            }),
            capabilities: vec!["orchestration".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // Execute a compiled pipeline with retry/idempotency guards.
        self.register_builtin_action(ActionDef {
            name: "pipeline_run".to_string(),
            description: "Run a pipeline DAG from inline spec or saved pipeline_name. Supports retry/backoff/idempotency per node, dependency-aware execution, and persisted run traces.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pipeline_name": { "type": "string", "description": "Saved pipeline name" },
                    "spec": { "type": "object", "description": "Inline pipeline spec (overrides pipeline_name)" },
                    "dry_run": { "type": "boolean", "description": "Validate/plan without executing" },
                    "context": { "type": "object", "description": "Template context values for node args/idempotency keys" },
                    "allow_privileged": { "type": "boolean", "description": "Allow privileged node actions (default: false)" }
                },
                "oneOf": [
                    { "required": ["pipeline_name"] },
                    { "required": ["spec"] }
                ]
            }),
            capabilities: vec!["orchestration".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // Typed signal ranking + consensus primitive.
        self.register_builtin_action(ActionDef {
            name: "signal_consensus".to_string(),
            description: "Rank and reconcile signals using typed scoring weights and optional reviewer perspectives. Returns top prioritized signals for daily decisioning.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "signals": { "type": "array", "items": { "type": "object" }, "description": "Signals with impact/confidence/effort + payload" },
                    "weights": {
                        "type": "object",
                        "properties": {
                            "impact": { "type": "number" },
                            "confidence": { "type": "number" },
                            "effort": { "type": "number" }
                        }
                    },
                    "perspectives": { "type": "array", "items": { "type": "object" }, "description": "Optional reviewer perspectives with custom weights" },
                    "top_k": { "type": "integer", "description": "Max ranked signals to return (default: 20)" }
                },
                "required": ["signals"]
            }),
            capabilities: vec!["analytics".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // Gmail scan
        self.register_builtin_action(ActionDef {
            name: "gmail_scan".to_string(),
            description: "Read and scan the user's Gmail inbox. Use when asked to check email, find emails, look for meetings/invites/receipts, or anything email-related. Supports three patterns: `recent` for the literal newest inbox emails in chronological order, `search` for exact Gmail query/filter matches, and `triage` for a smart importance scan across important, primary, recent, and starred messages. Leave mode as `auto` unless you know which behavior is needed.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "mode": { "type": "string", "enum": ["auto", "recent", "search", "triage"], "description": "How to interpret the request. `recent` returns the latest inbox emails exactly as they arrived. `search` returns exact matches for query/labels. `triage` runs the smart importance scan. `auto` picks search when query/labels are present, recent when only max_results is set, otherwise triage." },
                    "query": { "type": "string", "description": "Optional Gmail search query, for example 'from:sarah', 'subject:meeting', 'newer_than:2d', or 'label:promotions'. Best used with mode `search` or left with mode `auto` so the tool can infer search mode." },
                    "labels": { "type": "array", "items": { "type": "string" }, "description": "Optional Gmail label IDs such as INBOX, IMPORTANT, UNREAD, STARRED, SENT, DRAFT, SPAM, or TRASH. Supplying labels pushes auto mode into exact search/filter behavior." },
                    "max_results": { "type": "integer", "description": "Number of emails to return. In auto mode, setting only max_results requests the literal newest inbox emails. In search mode, it limits exact matches. In triage mode, it is ignored." }
                }
            }),
            capabilities: vec!["gmail".to_string(), "google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: read_only_authorization(integration_authorization("gmail")),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "gmail_reply".to_string(),
            description: "Send an email or reply via the user's Gmail. Use when asked to send, reply to, compose, or draft an email. Can reply to existing threads using thread_id.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "to": { "type": "string", "description": "Recipient email address" },
                    "subject": { "type": "string", "description": "Email subject line" },
                    "body": { "type": "string", "description": "Email body text (plain text)" },
                    "thread_id": { "type": "string", "description": "Gmail thread ID to reply to (from gmail_scan results)" },
                    "html_body": { "type": "string", "description": "Optional HTML body for multipart email delivery" },
                    "from": { "type": "string", "description": "Optional sender mailbox address. Defaults to the connected Gmail profile." },
                    "delivery_source": { "type": "string", "enum": ["auto", "gmail", "google_workspace"], "description": "Choose which connected Gmail backend to send through. Leave as auto unless a specific backend is required." }
                },
                "required": ["to", "subject", "body"]
            }),
            capabilities: vec!["gmail".to_string(), "google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: integration_authorization("gmail"),
        }).await;

        // Web search
        self.register_builtin_action(ActionDef {
            name: "web_search".to_string(),
            description: "Search the web for external information needed in the current answer or as a required input to another action. Use the semantic temporal scope to distinguish current/recent information from historical or timeless lookup. Do not use this as a prerequisite baseline for durable scheduled work or watchers when the durable object can perform its own later poll.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "x-agentark-semantic-query": true, "description": "Search query. Preserve the user's topic and any explicit date or range. For current/recent scope, include the runtime date or year when it improves freshness; for historical scope, preserve the historical period." },
                    "num_results": { "type": "integer", "description": "Number of results (default 5)" },
                    "backend": { "type": "string", "description": "Search backend override: serper, brave, brave_api, exa, tavily, perplexity, firecrawl, searxng, playwright, lightpanda, duckduckgo, bing_rss" },
                    "time_scope": { "type": "string", "enum": ["current", "recent", "historical", "timeless"], "description": "Semantic temporal intent of the lookup. Use current/recent when the answer depends on now, latest state, news, or recent changes; historical when the user gives or implies a past period; timeless for stable background/reference lookup." }
                },
                "required": ["query"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // Research
        self.register_builtin_action(ActionDef {
            name: "research".to_string(),
            description: "Conduct deep research on a topic by gathering diverse source sets, fetching and comparing evidence, surfacing contradictions and open questions, and returning a citation-backed synthesis. Use for complex current-answer questions that need thorough investigation beyond a simple web search. Do not use this as a prerequisite baseline for durable scheduled work or watchers when the durable object can perform its own later poll.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "x-agentark-semantic-query": true, "description": "Research topic or question. For current or recent questions, anchor the query to the runtime date/current year. For explicit historical periods, preserve the user's date or range instead of making it current." },
                    "max_sources": { "type": "integer", "description": "Maximum sources to examine (default 5, or 12 when depth='deep')" },
                    "backend": { "type": "string", "description": "Optional search backend override: serper, brave, brave_api, exa, tavily, perplexity, firecrawl, searxng, playwright, lightpanda, duckduckgo, bing_rss" },
                    "depth": { "type": "string", "description": "Research depth: quick, standard, deep" },
                    "include_sources": { "type": "boolean", "description": "Include source URLs" },
                    "min_primary_sources": { "type": "integer", "description": "Minimum number of primary-source-like results to include when available. Deep research defaults to 2." },
                    "freshness_window_days": { "type": "integer", "description": "Optional freshness window in days for preferring dated, recent evidence." },
                    "followup_rounds": { "type": "integer", "description": "Extra follow-up search rounds to close evidence gaps, fetch primary sources, and investigate contradictions. Deep research defaults to 2." }
                },
                "required": ["query"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_workflow_action(
            ActionDef {
                name: "research_report_compose".to_string(),
                description: "Compose gathered research evidence into a polished report with clean sections, citations, tables, and chart blocks when the evidence supports them. Use after sources or evidence have already been collected, when the user wants a formal report, brief, memo, landscape, or implementation analysis rather than another search.".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "evidence": { "type": "string", "description": "Source notes, excerpts, prior research output, or structured evidence to synthesize into a report." },
                        "audience": { "type": "string", "description": "Optional reader or stakeholder group for tone and detail level." },
                        "report_type": { "type": "string", "description": "Optional report type such as policy brief, market landscape, technical comparison, implementation plan, investment memo, or literature review." },
                        "output_format": { "type": "string", "enum": ["markdown", "report", "brief"], "description": "Preferred output shape. Defaults to report." },
                        "include_charts": { "type": "boolean", "description": "Include agentark-chart blocks when the evidence contains comparable numeric values." }
                    },
                    "required": ["evidence"]
                }),
                capabilities: vec!["document_generation".to_string()],
                sandbox_mode: Some(SandboxMode::Native),
                source: ActionSource::System,
                file_path: None,
                authorization: Default::default(),
            },
            Self::research_report_composer_workflow().to_string(),
        )
        .await;

        // Code execution sandbox
        self.register_builtin_action(ActionDef {
            name: "code_execute".to_string(),
            description: "Dominant execution primitive for tasks that require computation, scripting, live debugging, file generation, dependency bootstrap, network retrieval, data processing, validation, or iterative repair when no auth-bound shortcut is clearly required. Write a small program, run it, inspect stdout/stderr/exit status/generated files, and iterate on concrete errors. For app or repo repairs, ordered probes such as version checks, installed-package checks, logs, builds, and tests are supported and should guide the smallest patch/restart/redeploy needed. Use direct integration actions for OAuth-held services or guarded product capabilities, but otherwise prefer this over narrow catalog actions. Supports Python, JavaScript/TypeScript, Bash, notebooks, compiled languages, dependency installation, uploaded files at /data/<filename>, optional env vars, generated output files, and explicit network egress when the task needs live connectivity.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "language": {
                        "type": "string",
                        "enum": ["python", "javascript", "typescript", "bash", "ruby", "php", "perl", "lua", "r", "java", "c", "cpp", "go", "rust", "swift", "kotlin", "jupyter"],
                        "description": "Programming language. Use 'jupyter' for EDA/ML notebooks with visualizations."
                    },
                    "code": { "type": "string", "description": "Code to execute. For jupyter: provide valid .ipynb JSON content (notebook format). For other languages: plain code. Can include dependency installation. When files are provided, access them at /data/<filename>." },
                    "backend": {
                        "type": "string",
                        "enum": ["auto", "docker", "native", "executor_server"],
                        "description": "Compatibility shortcut for execution backend. Defaults to auto. Prefer backend_preference for new calls."
                    },
                    "backend_preference": {
                        "type": "object",
                        "description": "Typed backend preference contract. AgentArk tries preferred backends in order when fallback_policy is auto_degrade. Use require_exact when the user or workflow requires one specific backend.",
                        "properties": {
                            "preferred": {
                                "type": "array",
                                "items": { "type": "string", "enum": ["docker", "native", "remote_executor"] },
                                "description": "Backends to try in order."
                            },
                            "fallback_policy": {
                                "type": "string",
                                "enum": ["auto_degrade", "require_exact", "ask_user"],
                                "description": "How to handle unavailable preferred backends. Defaults to auto_degrade for explicit preference lists."
                            }
                        },
                        "additionalProperties": false
                    },
                    "network_access": { "type": "boolean", "description": "Whether this sandbox execution may use outbound network access. Default: false. Leave disabled unless the code genuinely needs egress." },
                    "timeout_secs": { "type": "integer", "description": "Optional execution timeout in seconds. Defaults are chosen by runtime: 60s for scripts, 120s for compiled builds, 600s for notebooks, and longer for dependency bootstrap. Max 600s for scripts/builds and 900s for notebooks." },
                    "execution_contract": {
                        "type": "object",
                        "description": "Optional structured execution contract for multi-step automations. Use exact phase values `bootstrap`, `validate`, or `poll`. For validation/polling steps that prove the monitor is ready, set `target_validated_when_successful=true` and `ready_for_watch_when_successful=true` so AgentArk can chain follow-up actions without guessing from source text.",
                        "properties": {
                            "phase": { "type": "string", "enum": ["bootstrap", "validate", "poll"] },
                            "target_validated_when_successful": { "type": "boolean" },
                            "ready_for_watch_when_successful": { "type": "boolean" },
                            "target_connectivity_required": { "type": "boolean", "description": "Set true when this step must reach a live target such as a URL, LAN device, network stream, API, or other device endpoint. AgentArk will enable sandbox network access for the step." }
                        }
                    },
                    "env": { "type": "object", "description": "Optional environment variables (values may include {{secret:...}} / {{env:...}} placeholders).", "additionalProperties": { "type": "string" } },
                    "files": {
                        "type": "array",
                        "items": {
                            "oneOf": [
                                { "type": "string" },
                                { "type": "object" }
                            ]
                        },
                        "description": "Input files injected into the sandbox at /data/<filename>. Entries may be upload IDs, ResourceRef objects, structured resource payloads returned by tools, resource ids, or allowed workspace/data file paths."
                    },
                    "background": { "type": "boolean", "description": "Queue this execution as durable background work instead of blocking the current turn. The same code_execute arguments are run later with background=false." },
                    "notify_on_complete": { "type": "boolean", "description": "When background=true, record that the user should be notified when the queued task completes." }
                },
                "required": ["language", "code"]
            }),
            capabilities: vec!["code_execute".to_string()],
            sandbox_mode: Some(SandboxMode::Docker),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // List tasks/goals/routines
        self.register_builtin_action(ActionDef {
            name: "list_tasks".to_string(),
            description: "List pending tasks, goals, routines, and scheduled items, including IDs that can be passed back to schedule_task.task_id for updates. Use when the user asks about their pending goals, tasks, agenda, or what's scheduled.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "filter": { "type": "string", "description": "Filter: 'all', 'pending', 'goals', 'routines', 'completed', 'failed'. Default: 'pending'" }
                }
            }),
            capabilities: vec!["skill_management".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "list_watchers".to_string(),
            description: "List background watchers and their live status, IDs, poll counts, conditions, and next poll timing. Use watcher IDs with watch.watcher_id when updating an existing watcher. Use when the user asks what the agent is watching, which watchers are active, or whether a watcher has triggered/paused/failed.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "filter": {
                        "type": "string",
                        "enum": ["active", "paused", "triggered", "failed", "timed_out", "cancelled", "all"],
                        "description": "Watcher status filter (default: active)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum watchers to return (default: 20)"
                    }
                }
            }),
            capabilities: vec!["watcher_inventory".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "watcher_delete".to_string(),
            description: "Delete an existing AgentArk background watcher by watcher_id and remove its linked background-session and Reflect records. Use after list_watchers identifies the watcher to remove. This action mutates durable AgentArk runtime state and requires explicit user approval.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "watcher_id": {
                        "type": "string",
                        "description": "Exact watcher UUID to delete. Get it from list_watchers when the user refers to a watcher by description."
                    }
                },
                "required": ["watcher_id"]
            }),
            capabilities: vec!["watcher_management".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: ActionAuthorization {
                risk_level: ActionRiskLevel::Medium,
                requires_auth: true,
                human_approval: crate::actions::ActionHumanApproval { required: true },
                ..Default::default()
            },
        }).await;

        self.register_builtin_action(ActionDef {
            name: "background_session_manage".to_string(),
            description: "Inspect or modify a durable AgentArk background session and its linked tasks/watchers. Use when the user refers to an existing background work/session and wants status, pause, resume, stop/cancel, deletion, or a delivery-channel change for that session as a whole. Resolve by background_session_id when available; otherwise provide the user's reference in reference_text so AgentArk can resolve against recent session context. This action is for AgentArk-owned background work, not app-internal refresh/poll cadence.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["status", "list", "pause", "resume", "stop", "cancel", "delete", "update_delivery"],
                        "description": "Session-level operation. stop and cancel close the session and cancel linked pending work; delete removes the session and linked work records."
                    },
                    "background_session_id": {
                        "type": "string",
                        "description": "Optional exact background session id. Omit only when the current conversation context clearly identifies one session."
                    },
                    "reference_text": {
                        "type": "string",
                        "description": "User's semantic reference to the target background work when no id is supplied."
                    },
                    "delivery_channel": {
                        "type": "string",
                        "description": "Required for update_delivery. Use preferred, in_app, telegram, whatsapp, or another configured channel only when requested."
                    },
                    "include_closed": {
                        "type": "boolean",
                        "description": "Include completed/cancelled/failed sessions when listing or resolving. Default false except status by exact id."
                    }
                },
                "required": ["operation"]
            }),
            capabilities: vec![
                "background_session".to_string(),
                "scheduler".to_string(),
                "watcher".to_string(),
                "notification".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec![
                    "background_session".to_string(),
                    "watcher".to_string(),
                    "scheduler".to_string(),
                ],
                channel_targets: vec![channel_target("delivery_channel", "preferred")],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        self.register_builtin_action(ark_inspect::action_def())
            .await;

        self.register_builtin_action(ActionDef {
            name: "goal_manage".to_string(),
            description: "Create, update, list, delete, or report on goals. Use when the user asks about goals, deadlines, progress toward a goal, or wants to save or change a goal for later tracking.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["create", "update", "list", "delete", "report"],
                        "description": "Goal operation to perform"
                    },
                    "goal": {
                        "type": "string",
                        "description": "Goal description. Required for create. May also be used to delete a goal by exact text."
                    },
                    "goal_id": {
                        "type": "string",
                        "description": "Specific goal identifier for update, delete, or report."
                    },
                    "new_goal": {
                        "type": "string",
                        "description": "Replacement goal description for update when locating the target by goal_id or goal."
                    },
                    "due_date": {
                        "type": "string",
                        "description": "Optional due date for create. Accepts YYYY-MM-DD or RFC3339 timestamp."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "description": "Maximum number of goals to list (default 10)."
                    },
                    "allow_duplicate": {
                        "type": "boolean",
                        "description": "Create another matching goal-management item instead of updating/reusing an existing one. Default false."
                    }
                },
                "required": ["operation"]
            }),
            capabilities: vec!["goal_management".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "curator".to_string(),
            description: "Inspect or control the filesystem skill curator. The curator tracks skill usage, marks agent-created skills stale, archives only unpinned agent-created stale skills, and queues recurring successful patterns for review.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["status", "pause", "resume"],
                        "description": "Curator operation. status reports usage/review files; pause/resume toggles the background loop via a filesystem flag."
                    }
                },
                "required": ["operation"]
            }),
            capabilities: vec!["skill_management".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "skill_view".to_string(),
            description: "List or read filesystem skills stored as SKILL.md recipes. Use to inspect reusable local procedures before applying one in the current turn.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": { "type": "string", "enum": ["list", "read"], "description": "List skills or read one skill." },
                    "name": { "type": "string", "description": "Skill name for operation=read." }
                },
                "required": ["operation"]
            }),
            capabilities: vec!["skill_management".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "skill_manage".to_string(),
            description: "Create, update, pin, unpin, archive, or restore filesystem skills as SKILL.md recipes. Skills are text procedures; executable snippets inside them should be run later through code_execute or other available tools.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": { "type": "string", "enum": ["create", "update", "pin", "unpin", "archive", "restore"], "description": "Skill management operation." },
                    "name": { "type": "string", "description": "Stable skill directory name." },
                    "markdown": { "type": "string", "description": "Complete SKILL.md body for create/update." }
                },
                "required": ["operation", "name"]
            }),
            capabilities: vec!["skill_management".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        }).await;

        // Browser automation - fetch and extract content from web pages
        self.register_builtin_action(ActionDef {
            name: "browse".to_string(),
            description: "Fetch a web page and extract content. Use when asked to visit a URL, read a web page, scrape content, or check a website. Returns extracted text, links, or page title depending on the 'extract' parameter.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to fetch (must include http:// or https://)" },
                    "extract": { "type": "string", "description": "What to extract: 'text' (default, main text content), 'links' (all hyperlinks), 'title' (page title), 'all' (text + links + title)" }
                },
                "required": ["url"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // Image generation
        self.register_builtin_action(ActionDef {
            name: "generate_image".to_string(),
            description: "Generate an image using AI. Use when asked to create, generate, draw, or make an image, picture, illustration, or visual.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Description of the image to generate" },
                    "negative_prompt": { "type": "string", "description": "What NOT to include (optional)" },
                    "width": { "type": "integer", "description": "Image width in pixels (default 1024)" },
                    "height": { "type": "integer", "description": "Image height in pixels (default 1024)" },
                    "style": { "type": "string", "description": "Art style (optional)" }
                },
                "required": ["prompt"]
            }),
            capabilities: vec!["image_generation".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: integration_authorization("media_gen"),
        }).await;

        // Action management - create/update/delete/list custom actions via chat
        // Home Assistant read-only state access.
        self.register_builtin_action(ActionDef {
            name: "home_assistant".to_string(),
            description: "Read Home Assistant state, services, and entities from the configured Home Assistant instance.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["list_entities", "search_entities", "get_state", "get_services"],
                        "description": "Read operation to run"
                    },
                    "entity_id": { "type": "string", "description": "Entity id for get_state, such as light.kitchen" },
                    "domain": { "type": "string", "description": "Optional entity domain filter such as light, sensor, switch, climate" },
                    "query": { "type": "string", "description": "Optional entity search query" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Maximum entities to return" }
                },
                "required": ["operation"]
            }),
            capabilities: vec!["home_assistant".to_string(), "local_network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: integration_authorization("home_assistant"),
        }).await;

        let mut home_assistant_control_auth = integration_authorization("home_assistant");
        home_assistant_control_auth.risk_level = ActionRiskLevel::High;
        home_assistant_control_auth.human_approval =
            crate::actions::ActionHumanApproval { required: true };
        home_assistant_control_auth.outbound.outbound_write = true;
        self.register_builtin_action(ActionDef {
            name: "home_assistant_call_service".to_string(),
            description: "Call a Home Assistant service on configured devices. Requires explicit user approval because it can change the physical environment.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "domain": { "type": "string", "description": "Home Assistant service domain, such as light, switch, climate, media_player" },
                    "service": { "type": "string", "description": "Service name in the selected domain, such as turn_on, turn_off, set_temperature" },
                    "entity_id": { "type": "string", "description": "Optional target entity id" },
                    "target": { "type": "object", "description": "Optional Home Assistant target object" },
                    "service_data": { "type": "object", "description": "Optional Home Assistant service data" }
                },
                "required": ["domain", "service"]
            }),
            capabilities: vec!["home_assistant_control".to_string(), "local_network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: home_assistant_control_auth,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "manage_actions".to_string(),
            description: "Create, update, delete, or list user-added actions/skills/workflows. Use when the user wants to inspect their installed skills, add a new action, or modify the action library.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "resource": {
                        "type": "string",
                        "enum": ["skill", "skill_marketplace"],
                        "description": "Managed Skills surface resource. Use skill for reusable AgentArk procedures/capabilities and skill_marketplace for sources that list installable skills."
                    },
                    "operation": {
                        "type": "string",
                        "enum": ["create", "import", "install", "update", "delete", "list", "read", "status", "enable", "disable", "refresh", "test"],
                        "description": "Operation to perform"
                    },
                    "name": {
                        "type": "string",
                        "description": "Skill or marketplace display name. For skills, use a stable kebab-case action name when creating."
                    },
                    "id": {
                        "type": "string",
                        "description": "Existing skill or marketplace identifier when known."
                    },
                    "url": {
                        "type": "string",
                        "description": "Raw SKILL.md URL for skill import, or marketplace manifest URL for skill_marketplace create/update."
                    },
                    "content": {
                        "type": "string",
                        "description": "Complete SKILL.md content with YAML frontmatter. Required for skill create/update unless importing from url. Format:\n---\nname: action-name\ndescription: What this action does\nversion: \"1.0.0\"\n---\n\n# Action Title\n\n## Steps\n..."
                    },
                    "markdown": {
                        "type": "string",
                        "description": "Alias for complete SKILL.md content."
                    },
                    "enabled": {
                        "type": "boolean",
                        "description": "Enable or disable the skill or marketplace after the operation when supported."
                    },
                    "security_confirmed": {
                        "type": "boolean",
                        "description": "Set only after the user confirms a non-blocking skill security review warning surfaced by the preview result. Blocking security findings still prevent saving."
                    },
                    "arguments": {
                        "type": "object",
                        "description": "Arguments for operation=test on a skill."
                    },
                    "allow_duplicate": {
                        "type": "boolean",
                        "description": "Repeat an identical action-management operation in the same request. Default false."
                    }
                },
                "required": ["operation"]
            }),
            capabilities: vec!["skill_management".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "list_integrations".to_string(),
            description: "Return a compact inventory of every AgentArk external surface: built-in integrations, messaging channels, notification channels, custom APIs, webhooks, companion devices, extension packs, plugins, and MCP servers. Use for overview or lightweight connected/authenticated checks; use inspect_integration for one detailed surface record.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Optional semantic target to resolve across all external surfaces, such as a provider, connector, notification channel, custom API, extension pack, plugin, or MCP server."
                    },
                    "include_disabled": {
                        "type": "boolean",
                        "description": "Include integrations that are currently disabled for agent dispatch. Default true."
                    },
                    "only_connected": {
                        "type": "boolean",
                        "description": "Only show integrations that are currently connected. Default false."
                    },
                    "include_details": {
                        "type": "boolean",
                        "description": "Include full per-surface records. Default false; prefer inspect_integration for detail."
                    }
                }
            }),
            capabilities: vec![
                "integration_inventory".to_string(),
                "surface_inventory".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "integration_catalog_list".to_string(),
            description: "Return the local integration registry as normalized provider/action entries. Use to discover available built-in connectors and manifest-backed packs, their auth mode, connected/enabled state, required scopes, capabilities, and callable action names before choosing an integration.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional stable integration or pack ids to return. Omit to list the full local registry."
                    },
                    "only_connected": {
                        "type": "boolean",
                        "description": "Only return entries that are connected and enabled. Default false."
                    },
                    "source_kind": {
                        "type": "string",
                        "enum": ["native", "extension_pack", "catalog_extension_pack"],
                        "description": "Optional normalized source kind filter."
                    }
                }
            }),
            capabilities: vec![
                "integration_inventory".to_string(),
                "integration_registry".to_string(),
                "capability_inventory".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "integration_catalog_describe".to_string(),
            description: "Describe one local integration registry entry by stable id. Returns the normalized provider/action metadata needed to decide whether and how an agent can use it.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Exact integration or pack id when known."
                    }
                },
                "required": ["id"]
            }),
            capabilities: vec![
                "integration_inventory".to_string(),
                "integration_registry".to_string(),
                "capability_inventory".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "integration_catalog_status".to_string(),
            description: "Return concise readiness status for one local integration registry entry by stable id, including connected/enabled flags, auth mode, required scopes, connection requirement, and action names.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Exact integration or pack id when known."
                    }
                },
                "required": ["id"]
            }),
            capabilities: vec![
                "integration_inventory".to_string(),
                "integration_registry".to_string(),
                "capability_inventory".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "inspect_integration".to_string(),
            description: "Inspect one AgentArk external surface by structured surface id and item id from list_integrations. Supports companion devices, built-in integrations, messaging/notification channels, custom APIs, webhooks, extension packs, plugins, and MCP servers. Returns detailed status without broad catalog output.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "surface": {
                        "type": "string",
                        "description": "Surface id from list_integrations, such as companion_devices, integrations, messaging_channels, notification_channels, custom_apis, webhook_sources, extension_packs, plugins, or mcp_servers."
                    },
                    "id": {
                        "type": "string",
                        "description": "Item id from list_integrations."
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional generic fallback search across ids and display names when id is not known."
                    },
                    "run_check": {
                        "type": "boolean",
                        "description": "Run a safe live/readiness check when the surface supports one. Default false."
                    }
                }
            }),
            capabilities: vec!["integration_inventory".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "mcp_server_manage".to_string(),
            description: "Create, update, delete, refresh, list, or inspect AgentArk MCP server configurations. Use only when the requested integration substrate is explicitly an MCP server for AgentArk itself, such as a provider MCP endpoint, stdio command, or MCP server configuration. Do not use this as the default installer for providers with official HTTP, REST, or GraphQL APIs; use custom API or extension-pack setup for those. Save non-secret transport/auth configuration immediately; store supplied secrets encrypted, or return a credential-needed status when the server requires a token/password that was not provided.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["create", "update", "delete", "refresh", "list", "status", "install", "connect"],
                        "description": "Configuration operation. install/connect are accepted as semantic aliases for creating or updating the server configuration."
                    },
                    "id": {
                        "type": "string",
                        "description": "Stable MCP server id. For create, omit to derive one from name or endpoint; existing servers with the same endpoint are reused unless allow_duplicate=true."
                    },
                    "query": {
                        "type": "string",
                        "description": "Lookup text for list/status when id is unknown."
                    },
                    "name": {
                        "type": "string",
                        "description": "Display name for the MCP server."
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional description from the provider docs."
                    },
                    "transport": {
                        "type": "object",
                        "description": "Transport object. For HTTP use {type:'http', url:'https://...'}; for local stdio use {type:'stdio', command:'...', args:[], working_dir:null, env:{...}}."
                    },
                    "url": {
                        "type": "string",
                        "description": "Convenience HTTP MCP endpoint URL when transport is omitted."
                    },
                    "command": {
                        "type": "string",
                        "description": "Convenience stdio command when transport is omitted."
                    },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Convenience stdio args when transport is omitted."
                    },
                    "working_dir": {
                        "type": "string",
                        "description": "Optional stdio working directory."
                    },
                    "env": {
                        "type": "object",
                        "description": "Optional stdio environment secrets. Values are stored encrypted; config stores only env keys."
                    },
                    "auth": {
                        "type": "object",
                        "description": "Optional auth object: {type:'none'|'bearer'|'basic'|'header'|'query', header/name, token/value/username/password, clear:false}."
                    },
                    "auth_type": {
                        "type": "string",
                        "enum": ["none", "bearer", "basic", "header", "query"],
                        "description": "Convenience auth type when auth object is omitted."
                    },
                    "auth_header_name": {
                        "type": "string",
                        "description": "Header name for bearer/header auth. Defaults to Authorization for bearer."
                    },
                    "auth_name": {
                        "type": "string",
                        "description": "Header or query parameter name for header/query auth."
                    },
                    "auth_value": {
                        "type": "string",
                        "description": "Secret token/value to store encrypted. Do not echo this in final answers."
                    },
                    "auth_username": {
                        "type": "string",
                        "description": "Basic auth username to store encrypted when supplied."
                    },
                    "auth_password": {
                        "type": "string",
                        "description": "Basic auth password to store encrypted when supplied."
                    },
                    "auth_profile_id": {
                        "type": "string",
                        "description": "Optional existing AgentArk auth profile id for HTTP MCP auth instead of inline MCP secrets."
                    },
                    "enabled": {
                        "type": "boolean",
                        "description": "Whether tools should be registered. Default true."
                    },
                    "resources_enabled": {
                        "type": "boolean",
                        "description": "Whether MCP resources should be registered. Default false."
                    },
                    "tool_allowlist": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "tool_blocklist": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "resource_allowlist": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "minimum": 1
                    },
                    "max_response_bytes": {
                        "type": "integer",
                        "minimum": 1024
                    },
                    "allow_duplicate": {
                        "type": "boolean",
                        "description": "Create another server even when an existing server has the same endpoint. Default false."
                    }
                },
                "required": ["operation"]
            }),
            capabilities: vec![
                "integration_builder".to_string(),
                "integration_inventory".to_string(),
                "mcp_server_management".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["capability_acquire".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        // PDF generation - creates PDF documents from content
        // Generic extension-pack control plane
        self.register_builtin_action(ActionDef {
            name: "postgres_schema_inspect".to_string(),
            description: "Inspect the live AgentArk Postgres public schema and return valid table and column names for follow-up diagnostics. Use before structured database reads or when a DB-backed internal question needs schema discovery.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "table_filter": {
                        "type": "string",
                        "description": "Optional case-insensitive substring filter for table names."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum tables to return (default: 25)."
                    }
                }
            }),
            capabilities: vec!["database_readonly".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "postgres_query_readonly".to_string(),
            description: "Run a structured, read-only table query against the live AgentArk Postgres database. Supply a public table name, optional columns, filters, sorting, and limit. Do not pass raw SQL. If a table or column is rejected, inspect the schema and retry.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "table": {
                        "type": "string",
                        "description": "Public AgentArk table name from postgres_schema_inspect."
                    },
                    "columns": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional list of columns to return. Default: all readable columns."
                    },
                    "filters": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "column": { "type": "string" },
                                "op": {
                                    "type": "string",
                                    "enum": ["eq", "neq", "gt", "gte", "lt", "lte", "contains", "starts_with", "ends_with", "in", "is_null", "not_null"]
                                },
                                "value": {}
                            },
                            "required": ["column", "op"]
                        }
                    },
                    "order_by": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "column": { "type": "string" },
                                "direction": {
                                    "type": "string",
                                    "enum": ["asc", "desc"]
                                }
                            },
                            "required": ["column"]
                        }
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum rows to return (default: 50, max: 200)."
                    }
                },
                "required": ["table"]
            }),
            capabilities: vec!["database_readonly".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "extension_pack_list".to_string(),
            description: "List installed and catalog extension packs only. Use for manifest-based extension pack discovery or install/source resolution. For connected/authenticated readiness across built-in connectors, bundled notification channels, custom APIs, custom messaging channels, extension packs, plugins, and MCP servers, use list_integrations instead.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "description": "Optional pack kind filter such as integration or messaging_channel." },
                    "query": { "type": "string", "description": "Optional search query." }
                }
            }),
            capabilities: vec![
                "integration_inventory".to_string(),
                "extension_pack_lifecycle".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "extension_pack_search".to_string(),
            description: "Search installed and catalog extension packs only, including pack-declared integrations or pack-declared messaging channels. Use this after list_integrations when the target is not already available as a built-in connector, bundled notification channel, custom API, custom messaging channel, plugin, MCP server, or installed extension pack.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Pack search query." },
                    "kind": { "type": "string", "description": "Optional kind filter such as integration or messaging_channel." }
                },
                "required": ["query"]
            }),
            capabilities: vec![
                "integration_inventory".to_string(),
                "extension_pack_lifecycle".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "extension_pack_install".to_string(),
            description: "Install a bundled, linked, or inline-manifest extension pack after pack search or source resolution. Use for install requests that can apply to integrations, messaging channels, or other pack types; this is not a runtime installer for already-installed packs.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pack_id": { "type": "string" },
                    "source_url": { "type": "string" },
                    "source_path": { "type": "string" },
                    "manifest_text": { "type": "string" },
                    "manifest": { "type": "object" },
                    "trust_unverified": { "type": "boolean" }
                }
            }),
            capabilities: vec![
                "integration_admin".to_string(),
                "extension_pack_lifecycle".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "extension_pack_scaffold".to_string(),
            description: "Scaffold a draft local extension pack from chat intent, docs, OpenAPI, or curl details. Use when the needed integration, messaging channel, or other extension pack does not exist yet.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "kind": { "type": "string" },
                    "description": { "type": "string" },
                    "docs_url": { "type": "string" },
                    "openapi_url": { "type": "string" },
                    "openapi_text": { "type": "string" },
                    "curl_text": { "type": "string" },
                    "auth_mode": { "type": "string", "enum": ["none", "api_key", "basic", "oauth2_external"] },
                    "desired_features": { "type": "array", "items": { "type": "string" } },
                    "read_only": { "type": "boolean" },
                    "binding_kind": { "type": "string" },
                    "publisher": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["name"]
            }),
            capabilities: vec![
                "integration_admin".to_string(),
                "extension_pack_lifecycle".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "extension_pack_connect".to_string(),
            description: "Create or update a pack connection. For OAuth-style packs this returns a browser connect URL when supported. For secret-based packs, omit the secret when AgentArk should collect credentials securely through the UI; do not ask users to paste raw secrets into normal chat.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pack_id": { "type": "string" },
                    "connection_id": { "type": "string" },
                    "name": { "type": "string" },
                    "enabled": { "type": "boolean" },
                    "metadata": { "type": "object" },
                    "secret": {},
                    "clear_secret": { "type": "boolean" },
                    "redirect_uri": { "type": "string", "description": "Optional explicit redirect URI for OAuth connect URL generation." }
                },
                "required": ["pack_id"]
            }),
            capabilities: vec![
                "integration_admin".to_string(),
                "extension_pack_lifecycle".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "custom_messaging_channel_upsert".to_string(),
            description: "Create or update a reusable custom messaging channel for outbound AgentArk notifications using a declared HTTP send spec. Use when the user wants AgentArk to deliver messages through a non-bundled channel, webhook, internal notification service, or provider-specific messaging API. Declare credential fields and {{secret:KEY}} placeholders only; never include raw credential values in this action.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Optional stable slug. If omitted, derived from name." },
                    "name": { "type": "string", "description": "User-facing channel name." },
                    "description": { "type": "string" },
                    "enabled": { "type": "boolean" },
                    "docs_url": { "type": "string" },
                    "auth_profile_id": { "type": "string", "description": "Optional reusable auth profile id for OAuth or advanced auth handled outside direct secret fields." },
                    "credential_fields": {
                        "type": "array",
                        "description": "Credential fields to collect securely. Do not include values.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "key": { "type": "string" },
                                "label": { "type": "string" },
                                "placeholder": { "type": "string" },
                                "help": { "type": "string" },
                                "input_type": { "type": "string", "enum": ["password", "text", "textarea"] },
                                "required": { "type": "boolean" }
                            },
                            "required": ["key"]
                        }
                    },
                    "auth_manifest": {
                        "type": "object",
                        "description": "Optional advanced IntegrationAuthManifest for multi-field, OAuth2 code, device code, or hybrid auth. Storage targets are normalized by AgentArk."
                    },
                    "send": {
                        "type": "object",
                        "description": "HTTP send template. Supported placeholders are {{text}}, {{subject}}, {{to}}, {{conversation_id}}, and {{secret:KEY}}.",
                        "properties": {
                            "method": { "type": "string", "enum": ["post", "put", "patch", "get", "delete"] },
                            "url_template": { "type": "string" },
                            "headers": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "name": { "type": "string" },
                                        "value_template": { "type": "string" }
                                    },
                                    "required": ["name", "value_template"]
                                }
                            },
                            "body_template": { "type": "string" },
                            "content_type": { "type": "string" },
                            "auth": {
                                "type": "object",
                                "description": "Auth transport binding. Examples: {kind:'none'}, {kind:'bearer', secret_key:'token'}, {kind:'custom_header', name:'X-Api-Key', value_template:'{{secret:api_key}}'}, {kind:'basic', username_key:'username', password_key:'password'}, {kind:'query_param', name:'key', value_template:'{{secret:api_key}}'}."
                            },
                            "expect_status": { "type": "array", "items": { "type": "integer" } }
                        },
                        "required": ["url_template"]
                    }
                },
                "required": ["name", "send"]
            }),
            capabilities: vec!["integration_admin".to_string(), "notify".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: ActionAuthorization {
                risk_level: ActionRiskLevel::High,
                requires_auth: true,
                rate_limit: Some(crate::actions::ActionRateLimit {
                    max_calls: 5,
                    window_seconds: 300,
                }),
                human_approval: crate::actions::ActionHumanApproval { required: true },
                outbound: crate::actions::ActionEgressPolicy {
                    outbound_write: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        }).await;

        self.register_builtin_action(ActionDef {
            name: "custom_messaging_channel_manage".to_string(),
            description: "Delete, enable, disable, or test an existing reusable custom messaging channel. Use only to manage the saved AgentArk channel record, not to send an ordinary notification.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Saved custom messaging channel id." },
                    "operation": { "type": "string", "enum": ["delete", "enable", "disable", "test"] }
                },
                "required": ["id", "operation"],
                "additionalProperties": false
            }),
            capabilities: vec!["integration_admin".to_string(), "notify".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: ActionAuthorization {
                risk_level: ActionRiskLevel::High,
                requires_auth: true,
                rate_limit: Some(crate::actions::ActionRateLimit {
                    max_calls: 5,
                    window_seconds: 300,
                }),
                human_approval: crate::actions::ActionHumanApproval { required: true },
                outbound: crate::actions::ActionEgressPolicy {
                    outbound_write: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        }).await;

        self.register_builtin_action(ActionDef {
            name: "extension_pack_set_enabled".to_string(),
            description: "Enable or disable an installed extension pack so its registered actions can be used by the agent.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pack_id": { "type": "string" },
                    "enabled": { "type": "boolean" }
                },
                "required": ["pack_id", "enabled"]
            }),
            capabilities: vec![
                "integration_admin".to_string(),
                "extension_pack_lifecycle".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "extension_pack_delete".to_string(),
            description: "Delete an installed non-bundled extension pack and optionally remove its saved connections and auth profiles. Bundled AgentArk packs can be disabled but not deleted.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pack_id": { "type": "string" },
                    "remove_connections": {
                        "type": "boolean",
                        "description": "Remove saved connections and owned auth profiles for this pack. Default true."
                    }
                },
                "required": ["pack_id"]
            }),
            capabilities: vec![
                "integration_admin".to_string(),
                "extension_pack_lifecycle".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        }).await;

        for (name, description) in [
            (
                "extension_pack_runtime_install",
                "Install or verify the local runtime declared by an extension pack only after live inventory shows that pack is installed and runtime_required is true.",
            ),
            (
                "extension_pack_runtime_verify",
                "Verify the local runtime declared by an extension pack only after live inventory shows that pack is installed and runtime_required is true.",
            ),
            (
                "extension_pack_runtime_update",
                "Update the local runtime declared by an extension pack only after live inventory shows that pack is installed and runtime_required is true.",
            ),
            (
                "extension_pack_runtime_uninstall",
                "Uninstall the local runtime declared by an extension pack only after live inventory shows that pack is installed and runtime_required is true.",
            ),
        ] {
            self.register_builtin_action(ActionDef {
                name: name.to_string(),
                description: description.to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "pack_id": { "type": "string" }
                    },
                    "required": ["pack_id"]
                }),
                capabilities: vec![
                    "integration_admin".to_string(),
                    "integration_runtime_lifecycle".to_string(),
                    "extension_pack_lifecycle".to_string(),
                ],
                sandbox_mode: Some(SandboxMode::Native),
                source: ActionSource::System,
                file_path: None,
                authorization: Default::default(),
            })
            .await;
        }

        self.register_builtin_action(ActionDef {
            name: "extension_pack_test_connection".to_string(),
            description: "Run a pack connection health test when available. If connection_id is omitted, AgentArk tests the preferred saved connection for that pack.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pack_id": { "type": "string" },
                    "connection_id": { "type": "string" }
                },
                "required": ["pack_id"]
            }),
            capabilities: vec![
                "integration_inventory".to_string(),
                "extension_pack_lifecycle".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "extension_pack_list_events".to_string(),
            description:
                "List recent inbound webhook/event records for an installed extension pack."
                    .to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pack_id": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100 }
                },
                "required": ["pack_id"]
            }),
            capabilities: vec![
                "integration_inventory".to_string(),
                "extension_pack_lifecycle".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "extension_pack_invoke".to_string(),
            description: "Invoke one feature from an installed extension pack. Use when the user wants to use a pack capability directly instead of going through a legacy built-in action.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pack_id": { "type": "string" },
                    "connection_id": { "type": "string" },
                    "feature_id": { "type": "string" },
                    "arguments": { "type": "object" }
                },
                "required": ["feature_id"]
            }),
            capabilities: vec![
                "integration_inventory".to_string(),
                "extension_pack_lifecycle".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "pdf_generate".to_string(),
            description: "Generate a paginated PDF file from supplied text content, with simple report, letter, invoice, or plain layouts. The result is a PDF artifact for reading, printing, or sharing rather than a runnable interface or hosted application.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string", "description": "Text content for the PDF" },
                    "title": { "type": "string", "description": "Document title (optional)" },
                    "filename": { "type": "string", "description": "Output filename (default: output.pdf)" },
                    "style": { "type": "string", "enum": ["report", "letter", "invoice", "plain"], "description": "PDF style/template (default: plain)" }
                },
                "required": ["content"]
            }),
            capabilities: vec![
                "file_write".to_string(),
                "pdf_generation".to_string(),
                "document_generation".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // Expense tracking - add, list, summarize, delete expenses
        self.register_builtin_action(ActionDef {
            name: "expense".to_string(),
            description: "Track expenses and spending. Actions: add (record expense), list (view expenses with optional date/category filter), summary (spending summary by category), delete (remove expense by ID). Use when the user mentions spending, costs, expenses, budget, or purchases.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["add", "list", "summary", "delete"], "description": "Operation to perform" },
                    "amount": { "type": "number", "description": "Amount spent (for add)" },
                    "currency": { "type": "string", "description": "Currency code, e.g. USD, INR (default: USD)" },
                    "category": { "type": "string", "description": "Category: food, transport, shopping, bills, entertainment, health, education, other" },
                    "description": { "type": "string", "description": "What was purchased" },
                    "date": { "type": "string", "description": "Date (YYYY-MM-DD). Default: today" },
                    "vendor": { "type": "string", "description": "Store/vendor name (optional)" },
                    "payment_method": { "type": "string", "description": "cash, card, upi, etc. (optional)" },
                    "tags": { "type": "string", "description": "Comma-separated tags (optional)" },
                    "id": { "type": "string", "description": "Expense ID (for delete)" },
                    "from_date": { "type": "string", "description": "Start date filter (YYYY-MM-DD, for list/summary)" },
                    "to_date": { "type": "string", "description": "End date filter (YYYY-MM-DD, for list/summary)" },
                    "filter_category": { "type": "string", "description": "Category filter (for list)" }
                },
                "required": ["action"]
            }),
            capabilities: vec![],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // Security logs - query security events from DB
        self.register_builtin_action(ActionDef {
            name: "security_logs".to_string(),
            description: "View security event logs. Shows recent security events like injection attempts, auth failures, rate limit breaches. Use when the user asks about security events, attack attempts, or system security status.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Max entries to return (default: 50)" }
                }
            }),
            capabilities: vec![],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // Audio transcription
        self.register_builtin_action(ActionDef {
            name: "transcribe_audio".to_string(),
            description: "Transcribe audio/video files to text using Whisper. Use when asked to transcribe, convert speech to text, or extract text from audio/video.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "Path to audio/video file" },
                    "language": { "type": "string", "description": "Language code (e.g. en, hi). Default: auto-detect" },
                    "model": { "type": "string", "enum": ["tiny", "base", "small", "medium", "large"], "description": "Whisper model size (default: base)" }
                },
                "required": ["file_path"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // Weekly review
        self.register_builtin_action(ActionDef {
            name: "weekly_review".to_string(),
            description: "Generate a weekly review summarizing completed tasks, key conversations, and progress. Use when asked for a weekly review, weekly summary, or progress report.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "period_days": { "type": "integer", "description": "Number of days to review (default: 7)" }
                }
            }),
            capabilities: vec![],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // === Integration-backed actions ===

        // GitHub
        self.register_builtin_action(ActionDef {
            name: "github".to_string(),
            description: "Interact with GitHub repositories, issues, and pull requests. Actions: list_repos, create_issue, list_issues, list_prs, create_pr, search. Use when the user mentions GitHub, repos, issues, pull requests, or PRs.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list_repos", "create_issue", "list_issues", "list_prs", "create_pr", "search"], "description": "GitHub action to perform" },
                    "owner": { "type": "string", "description": "Repository owner (username or org)" },
                    "repo": { "type": "string", "description": "Repository name" },
                    "title": { "type": "string", "description": "Issue/PR title (for create)" },
                    "body": { "type": "string", "description": "Issue/PR body (for create)" },
                    "labels": { "type": "string", "description": "Comma-separated labels (for create_issue)" },
                    "head": { "type": "string", "description": "Head branch (for create_pr)" },
                    "base": { "type": "string", "description": "Base branch (for create_pr, default: main)" },
                    "query": { "type": "string", "description": "Search query (for search)" },
                    "state": { "type": "string", "enum": ["open", "closed", "all"], "description": "Filter by state (for list)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: integration_authorization("github"),
        }).await;

        // Notion
        self.register_builtin_action(ActionDef {
            name: "notion".to_string(),
            description: "Interact with Notion pages, databases, and blocks. Actions: search, create_page, update_page, get_page, append_blocks. Use when the user mentions Notion, notes, wiki, or knowledge base.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["search", "create_page", "update_page", "get_page", "append_blocks"], "description": "Notion action to perform" },
                    "query": { "type": "string", "description": "Search query (for search)" },
                    "page_id": { "type": "string", "description": "Page ID (for get/update/append)" },
                    "parent_id": { "type": "string", "description": "Parent page or database ID (for create)" },
                    "title": { "type": "string", "description": "Page title (for create)" },
                    "content": { "type": "string", "description": "Page content as markdown (for create/append)" },
                    "properties": { "type": "object", "description": "Page properties to update (for update)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: integration_authorization("notion"),
        }).await;

        // Twitter/X
        self.register_builtin_action(ActionDef {
            name: "twitter".to_string(),
            description: "Read tweets, search Twitter/X, view bookmarks, and get user profiles. Actions: bookmarks, list_tweets, search, get_user. Use when the user mentions Twitter, X, tweets, or bookmarks.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["bookmarks", "list_tweets", "search", "get_user"], "description": "Twitter action to perform" },
                    "query": { "type": "string", "description": "Search query (for search)" },
                    "username": { "type": "string", "description": "Twitter username (for get_user, list_tweets)" },
                    "max_results": { "type": "integer", "description": "Maximum results to return (default: 10)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: integration_authorization("twitter"),
        }).await;

        // 1Password
        self.register_builtin_action(ActionDef {
            name: "onepassword".to_string(),
            description: "Access 1Password vault for secure credential management. Actions: list_vaults, get_item (metadata only), search, create_item. Never exposes raw secrets to the LLM.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list_vaults", "get_item", "search", "create_item"], "description": "1Password action to perform" },
                    "vault_id": { "type": "string", "description": "Vault ID (optional filter)" },
                    "item_id": { "type": "string", "description": "Item ID (for get_item)" },
                    "query": { "type": "string", "description": "Search query (for search)" },
                    "title": { "type": "string", "description": "Item title (for create)" },
                    "category": { "type": "string", "description": "Item category: login, password, note, etc. (for create)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: integration_authorization("onepassword"),
        }).await;

        // Google Places
        self.register_builtin_action(ActionDef {
            name: "places".to_string(),
            description: "Search for places, find nearby locations, get place details, and get directions using Google Places/Maps. Actions: search, nearby, details, directions. Use when the user asks about restaurants, shops, locations, directions, or nearby places.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["search", "nearby", "details", "directions"], "description": "Places action to perform" },
                    "query": { "type": "string", "description": "Search query (for search)" },
                    "latitude": { "type": "number", "description": "Latitude (for nearby)" },
                    "longitude": { "type": "number", "description": "Longitude (for nearby)" },
                    "radius": { "type": "integer", "description": "Search radius in meters (for nearby, default: 1000)" },
                    "place_id": { "type": "string", "description": "Place ID (for details)" },
                    "origin": { "type": "string", "description": "Origin address (for directions)" },
                    "destination": { "type": "string", "description": "Destination address (for directions)" },
                    "type": { "type": "string", "description": "Place type filter: restaurant, cafe, hospital, atm, etc." }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: integration_authorization("google_places"),
        }).await;

        // Twilio (Voice & SMS)
        self.register_builtin_action(ActionDef {
            name: "twilio".to_string(),
            description: "Make phone calls and send SMS messages via Twilio. Actions: call, sms, list_calls, list_messages. Use when the user wants to call someone, send a text message, or check call/message history.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["call", "sms", "list_calls", "list_messages"], "description": "Twilio action to perform" },
                    "to": { "type": "string", "description": "Phone number to call/text (E.164 format: +1234567890)" },
                    "message": { "type": "string", "description": "Message body (for sms)" },
                    "twiml": { "type": "string", "description": "TwiML instructions for the call (for call)" },
                    "limit": { "type": "integer", "description": "Number of records to return (for list, default: 20)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: integration_authorization("twilio"),
        }).await;

        // Ordering & Purchasing
        self.register_builtin_action(ActionDef {
            name: "ordering".to_string(),
            description: "Search products and place orders via Shopify or custom webhook. Actions: search_products, create_order, order_status, list_orders. Use when the user wants to buy, order, or shop for something.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["search_products", "create_order", "order_status", "list_orders"], "description": "Ordering action to perform" },
                    "query": { "type": "string", "description": "Product search query (for search_products)" },
                    "product_id": { "type": "string", "description": "Product ID (for create_order)" },
                    "quantity": { "type": "integer", "description": "Quantity to order (default: 1)" },
                    "order_id": { "type": "string", "description": "Order ID (for order_status)" },
                    "shipping_address": { "type": "object", "description": "Shipping address (for create_order)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: integration_authorization("ordering"),
        }).await;

        // Browser automation - full headless browser control with human-in-the-loop
        // Curated connectors
        // Garmin
        self.register_builtin_action(ActionDef {
            name: "garmin".to_string(),
            description: "Retrieve Garmin fitness data. Actions: daily_summary, activities.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["daily_summary", "activities"], "description": "Garmin action to perform" },
                    "date": { "type": "string", "description": "Date in YYYY-MM-DD (daily_summary)" },
                    "start_date": { "type": "string", "description": "Start date in YYYY-MM-DD (activities)" },
                    "end_date": { "type": "string", "description": "End date in YYYY-MM-DD (activities)" },
                    "limit": { "type": "integer", "description": "Maximum records (default: 50)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: integration_authorization("garmin"),
        }).await;

        // WHOOP
        self.register_builtin_action(ActionDef {
            name: "whoop".to_string(),
            description: "Retrieve WHOOP performance data. Actions: profile, recovery, sleep, workouts.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["profile", "recovery", "sleep", "workouts"], "description": "WHOOP action to perform" },
                    "limit": { "type": "integer", "description": "Maximum records (default: 25)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: integration_authorization("whoop"),
        }).await;

        // GA4
        self.register_builtin_action(ActionDef {
            name: "ga4".to_string(),
            description: "Run GA4 Data API reports. Action: run_report.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["run_report"], "description": "GA4 action to perform" },
                    "property_id": { "type": "string", "description": "GA4 property ID" },
                    "dimensions": { "type": "array", "items": { "type": "string" }, "description": "Dimension names" },
                    "metrics": { "type": "array", "items": { "type": "string" }, "description": "Metric names" },
                    "date_ranges": {
                        "type": "array",
                        "description": "GA4 date ranges payload",
                        "items": {
                            "type": "object",
                            "properties": {
                                "startDate": { "type": "string", "description": "Start date (e.g. 7daysAgo or YYYY-MM-DD)" },
                                "endDate": { "type": "string", "description": "End date (e.g. today or YYYY-MM-DD)" }
                            }
                        }
                    },
                    "limit": { "type": "integer", "description": "Maximum rows (default: 1000)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: integration_authorization("ga4"),
        }).await;

        // GSC
        self.register_builtin_action(ActionDef {
            name: "gsc".to_string(),
            description: "Query Google Search Console analytics. Action: query.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["query"], "description": "GSC action to perform" },
                    "site_url": { "type": "string", "description": "Site URL (or sc-domain value)" },
                    "start_date": { "type": "string", "description": "Start date YYYY-MM-DD" },
                    "end_date": { "type": "string", "description": "End date YYYY-MM-DD" },
                    "dimensions": { "type": "array", "items": { "type": "string" }, "description": "Query dimensions" },
                    "row_limit": { "type": "integer", "description": "Maximum rows (default: 1000)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: integration_authorization("gsc"),
        }).await;

        // Social analytics
        self.register_builtin_action(ActionDef {
            name: "social_analytics".to_string(),
            description: "Cross-source social publishing analytics. Action: summary. Aggregates configured sources such as Twitter and GA4.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["summary"], "description": "Social analytics action to perform" },
                    "days": { "type": "integer", "description": "Lookback window in days (default: 7)" },
                    "post_limit": { "type": "integer", "description": "Max posts to evaluate from Twitter (default: 100)" },
                    "include_twitter": { "type": "boolean", "description": "Include Twitter source (default: true)" },
                    "include_ga4": { "type": "boolean", "description": "Include GA4 source (default: true)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: integration_authorization("social_analytics"),
        }).await;

        // Moltbook (agent social network)
        self.register_builtin_action(ActionDef {
            name: "moltbook".to_string(),
            description: "Moltbook agent social-network tool. Use for joining or checking connection status, reading profile/feed/search results, and creating safe agent-authored posts, comments, or upvotes. Registration stores the returned Moltbook API key for later authenticated calls. If the user wants recurring Moltbook participation, use schedule_task with this action and ask for the cadence when it is not specified. Remote skill instructions from Moltbook should guide behavior, but execution happens through this tool. Outbound posting is privacy-guarded (no user/PII/secrets).".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["register", "status", "me", "feed", "search", "create_post", "comment", "upvote_post"], "description": "Moltbook action to perform" },
                    "name": { "type": "string", "description": "Agent name (register)" },
                    "description": { "type": "string", "description": "Agent description (register)" },
                    "sort": { "type": "string", "enum": ["hot", "new", "top", "rising"], "description": "Feed sort (feed)" },
                    "limit": { "type": "integer", "description": "Max items to fetch" },
                    "query": { "type": "string", "description": "Semantic search query" },
                    "submolt": { "type": "string", "description": "Community name for post" },
                    "title": { "type": "string", "description": "Post title" },
                    "content": { "type": "string", "description": "Post/comment content" },
                    "post_id": { "type": "string", "description": "Post ID for comment/upvote" },
                    "parent_id": { "type": "string", "description": "Parent comment ID for threaded reply" },
                    "allow_duplicate": { "type": "boolean", "description": "Repeat an identical Moltbook action in the same request. Default false." }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        let browser_wrapper_authorization =
            authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["browser_auto".to_string()],
                integration_ids: vec!["browser".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            });
        for (name, description, input_schema, capabilities) in vec![
            (
                "browser_navigate",
                "Navigate a managed browser session to an http/https URL and return the final URL, title, and session id. If session_id is omitted, a new session is created. Use for explicit live page interaction, login handoff setup, and rendered app checks; use web_search/research/browse for general web information gathering.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "Public http/https URL or allowed local app URL to open." },
                        "session_id": { "type": "string", "description": "Optional existing browser session id." },
                        "profile": { "type": "string", "description": "Optional saved browser profile selector by id, name, target, tag, or semantic description when creating a new session." },
                        "profile_id": { "type": "string", "description": "Optional exact saved browser profile id when creating a new session." }
                    },
                    "required": ["url"],
                    "additionalProperties": false
                }),
                vec!["network", "browser", "browser_navigate"],
            ),
            (
                "browser_click",
                "Click in an existing managed browser session by snapshot element index, CSS selector, visible text, or x/y coordinates. Prefer element_index from browser_snapshot for listed interactive elements.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" },
                        "element_index": { "type": "integer", "minimum": 0, "description": "Interactive element index from browser_snapshot." },
                        "selector": { "type": "string", "description": "CSS selector to click." },
                        "text": { "type": "string", "description": "Visible text target to click." },
                        "x": { "type": "integer", "description": "Viewport x coordinate." },
                        "y": { "type": "integer", "description": "Viewport y coordinate." }
                    },
                    "required": ["session_id"],
                    "additionalProperties": false
                }),
                vec!["network", "browser", "browser_click"],
            ),
            (
                "browser_type",
                "Type text into an existing managed browser session by snapshot element index, CSS selector, or the currently focused editable element. Prefer element_index from browser_snapshot for listed editable textboxes and rich text fields.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" },
                        "text": { "type": "string" },
                        "element_index": { "type": "integer", "minimum": 0, "description": "Editable element index from browser_snapshot." },
                        "selector": { "type": "string", "description": "Optional CSS selector to fill." },
                        "clear": { "type": "boolean", "default": false, "description": "Clear the target before typing." }
                    },
                    "required": ["session_id", "text"],
                    "additionalProperties": false
                }),
                vec!["network", "browser", "browser_type"],
            ),
            (
                "browser_scroll",
                "Scroll an existing managed browser session up or down by a pixel amount.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" },
                        "direction": { "type": "string", "enum": ["up", "down"], "default": "down" },
                        "amount": { "type": "integer", "minimum": 1, "default": 500 }
                    },
                    "required": ["session_id"],
                    "additionalProperties": false
                }),
                vec!["network", "browser", "browser_scroll"],
            ),
            (
                "browser_snapshot",
                "Read the current page title, URL, visible body text, interactive elements, and diagnostics from an existing managed browser session. Use for DOM-level app validation and deciding the next browser action.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" },
                        "include_text": { "type": "boolean", "default": true },
                        "include_elements": { "type": "boolean", "default": true },
                        "element_limit": { "type": "integer", "minimum": 0, "maximum": 50, "default": 50 }
                    },
                    "required": ["session_id"],
                    "additionalProperties": false
                }),
                vec!["network", "browser", "browser_snapshot"],
            ),
            (
                "browser_screenshot",
                "Capture the current viewport of an existing managed browser session as a PNG image encoded in base64.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" }
                    },
                    "required": ["session_id"],
                    "additionalProperties": false
                }),
                vec!["network", "browser", "browser_screenshot"],
            ),
            (
                "browser_back",
                "Navigate an existing managed browser session back in its history and return the resulting page state.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" }
                    },
                    "required": ["session_id"],
                    "additionalProperties": false
                }),
                vec!["network", "browser", "browser_back"],
            ),
            (
                "browser_press",
                "Press a keyboard key in an existing managed browser session, such as Enter, Escape, Tab, or ArrowDown.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" },
                        "key": { "type": "string" }
                    },
                    "required": ["session_id", "key"],
                    "additionalProperties": false
                }),
                vec!["network", "browser", "browser_press"],
            ),
            (
                "browser_console",
                "Return recent console, page error, failed request, and HTTP error diagnostics captured for an existing managed browser session.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" },
                        "severity": { "type": "string", "description": "Optional severity filter such as error, warning, or info." },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 80, "default": 40 }
                    },
                    "required": ["session_id"],
                    "additionalProperties": false
                }),
                vec!["network", "browser", "browser_console"],
            ),
        ] {
            self.register_builtin_action(ActionDef {
                name: name.to_string(),
                description: description.to_string(),
                version: "1.0.0".to_string(),
                input_schema,
                capabilities: capabilities
                    .into_iter()
                    .map(str::to_string)
                    .collect::<Vec<_>>(),
                sandbox_mode: Some(SandboxMode::Native),
                source: ActionSource::System,
                file_path: None,
                authorization: browser_wrapper_authorization.clone(),
            })
            .await;
        }

        self.register_builtin_action(ActionDef {
            name: "browser_profile_manage".to_string(),
            description: "Manage saved browser login profiles as durable AgentArk resources. Supports listing available profiles, resolving a selector, launching a profile for login repair, closing and saving live profile sessions, and creating, updating, or deleting profile records. Browser tasks can then reuse these profiles through browser_auto profile/profile_id fields.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["create", "read", "update", "delete", "list", "status", "launch", "close", "resolve"],
                        "description": "Browser profile resource operation."
                    },
                    "id": { "type": "string", "description": "Exact browser profile id." },
                    "profile_id": { "type": "string", "description": "Exact browser profile id." },
                    "profile": { "type": "string", "description": "Browser profile selector by id, name, target, tag, or semantic description." },
                    "query": { "type": "string", "description": "Browser profile selector for read/resolve/status." },
                    "name": { "type": "string", "description": "Profile display name for create/update." },
                    "description": { "type": "string", "description": "Profile description for create/update." },
                    "browser": { "type": "string", "description": "Browser engine preference, such as chrome." },
                    "managed": { "type": "boolean", "description": "Whether the profile is an AgentArk-managed sandbox profile." },
                    "enabled": { "type": "boolean", "description": "Whether the profile can be selected for browser tasks." },
                    "target_kind": { "type": "string", "enum": ["sandbox", "host", "remote_cdp"], "description": "Profile storage/control target." },
                    "target_endpoint": { "type": "string", "description": "Remote browser endpoint when target_kind is remote_cdp." },
                    "target_profile_path": { "type": "string", "description": "Host profile path when explicitly configured." },
                    "target_workspace": { "type": "string", "description": "Optional workspace/scope label for the profile." },
                    "login_state": { "type": "string", "enum": ["unknown", "logged_out", "logged_in", "needs_mfa", "expired", "error"], "description": "Known login state for the profile." },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Profile tags for semantic selection." },
                    "metadata": { "type": "object", "description": "Non-sensitive browser profile metadata." }
                },
                "required": ["operation"],
                "additionalProperties": false
            }),
            capabilities: vec![
                "browser".to_string(),
                "browser_profile".to_string(),
                "capability_inventory".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "browser_auto".to_string(),
            description: "Start a managed background browser session for website interaction. Use when the task is high-level live web work such as going to a site, logging in, filling a form, or handing off MFA/CAPTCHA steps to the user. For explicit step-by-step browser control, use browser_navigate, browser_snapshot, browser_click, browser_type, browser_scroll, browser_press, browser_back, browser_screenshot, and browser_console.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["start_session"],
                        "description": "Starts a managed browser session."
                    },
                    "task": { "type": "string", "description": "High-level description of what to accomplish (for start_session)" },
                    "url": { "type": "string", "description": "Optional page to open when starting the browser session." },
                    "profile": { "type": "string", "description": "Optional saved browser profile selector by id, name, target, tag, or semantic description." },
                    "profile_id": { "type": "string", "description": "Optional exact saved browser profile id." },
                    "channel": { "type": "string", "description": "Channel to notify on (telegram, whatsapp, web)" },
                    "chat_id": { "type": "string", "description": "Optional channel chat identifier for notifications" },
                    "conversation_id": { "type": "string", "description": "Optional conversation id to append browser handoff updates into chat" }
                },
                "required": ["action"]
            }),
            capabilities: vec![
                "network".to_string(),
                "browser".to_string(),
                "browser_profile".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // Google Calendar - list, create, find free time
        self.register_builtin_action(ActionDef {
            name: "calendar_today".to_string(),
            description: "List today's calendar events. Use when the user asks 'what's on my calendar today', 'do I have any meetings', etc.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: read_only_authorization(integration_authorization("google_calendar")),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "calendar_list".to_string(),
            description: "List calendar events in a date range. Use when asked about upcoming events, schedule for a specific date, etc.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "start": { "type": "string", "description": "Start datetime (ISO 8601). Defaults to now." },
                    "end": { "type": "string", "description": "End datetime (ISO 8601). Defaults to 7 days from now." }
                }
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: read_only_authorization(integration_authorization("google_calendar")),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "calendar_create".to_string(),
            description: "Create a new Google Calendar event. Use only when the user wants an external calendar entry, meeting invite, appointment, or blocked time. For plain reminders or date notifications, use `schedule_task` with `notify_user` instead. AgentArk schedules its own default push reminder separately unless the user says not to remind them.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "summary": { "type": "string", "description": "Event title" },
                    "start": { "type": "string", "description": "Start datetime (ISO 8601)" },
                    "end": { "type": "string", "description": "End datetime (ISO 8601)" },
                    "description": { "type": "string", "description": "Event description/notes" },
                    "location": { "type": "string", "description": "Event location" },
                    "attendees": { "type": "array", "items": { "type": "string" }, "description": "List of attendee email addresses" },
                    "agentark_reminder": {
                        "type": ["boolean", "object"],
                        "description": "AgentArk push reminder control. Omit for the default 15-minute push reminder. Use false only when the user explicitly opts out. Use {\"enabled\": true, \"minutes_before\": N} when the user requests a different AgentArk reminder lead time.",
                        "properties": {
                            "enabled": { "type": "boolean" },
                            "minutes_before": { "type": "integer", "minimum": 1, "maximum": 1440 }
                        }
                    }
                },
                "required": ["summary", "start", "end"]
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["calendar_write".to_string()],
                integration_ids: vec!["google_calendar".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "calendar_free".to_string(),
            description: "Find free time slots in the calendar. Use when asked 'when am I free', 'find time for a meeting', etc.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "start": { "type": "string", "description": "Start of range (ISO 8601). Defaults to now." },
                    "end": { "type": "string", "description": "End of range (ISO 8601). Defaults to end of today." },
                    "min_duration_minutes": { "type": "integer", "description": "Minimum free slot duration in minutes (default: 30)" }
                }
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: read_only_authorization(integration_authorization("google_calendar")),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_drive_search".to_string(),
            description: "Search or list Google Drive files using the connected Google Workspace account with read-only Drive access. Use when the requested outcome depends on visible Drive file metadata or contents. If the user has not provided a narrower target, omit `query` and return a small default/recent listing instead of asking them to restate the source.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Optional Google Drive query, such as name contains 'roadmap' or mimeType='application/vnd.google-apps.spreadsheet'. Omit when a general/default file listing is the useful read-only outcome." },
                    "page_size": { "type": "integer", "description": "Max number of files to return (default 10)." }
                }
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: google_workspace_bundle_read_authorization("drive"),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_docs_read".to_string(),
            description: "Read the text content of a Google Doc by document ID. Use when the user provides a Google Doc link or ID and wants the content summarized or inspected.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "document_id": { "type": "string", "description": "Google Doc document ID." }
                },
                "required": ["document_id"]
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: google_workspace_bundle_read_authorization("docs"),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_sheets_read".to_string(),
            description: "Read a range from Google Sheets. Use when the user provides a spreadsheet ID and range or asks for values from a connected Google Sheet.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "spreadsheet_id": { "type": "string", "description": "Google Sheets spreadsheet ID." },
                    "range": { "type": "string", "description": "A1 range notation, such as Sheet1!A1:D20." }
                },
                "required": ["spreadsheet_id", "range"]
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: google_workspace_bundle_read_authorization("sheets"),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_chat_list_spaces".to_string(),
            description: "List the Google Chat spaces visible to the connected Google Workspace account with read-only Chat access.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "page_size": { "type": "integer", "description": "Max number of spaces to return (default 20)." }
                }
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: google_workspace_bundle_read_authorization("chat"),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_admin_list_users".to_string(),
            description: "List Google Workspace users from the Admin Directory with read-only directory access. Use when the user asks about Workspace users, seats, or directory accounts.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "customer": { "type": "string", "description": "Optional Google customer ID. Defaults to my_customer." },
                    "domain": { "type": "string", "description": "Optional domain filter if customer is not provided." },
                    "max_results": { "type": "integer", "description": "Max number of users to return (default 20)." }
                }
            }),
            capabilities: vec!["google_workspace".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: google_workspace_bundle_read_authorization("admin"),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_workspace_gws_help".to_string(),
            description: "Inspect help text for the underlying Google Workspace CLI. This is a support tool for understanding CLI syntax when a direct typed action does not cover the requested Workspace operation.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "argv": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional arguments after `gws`. Leave empty for top-level help."
                    }
                }
            }),
            capabilities: vec!["google_workspace".to_string(), "tool_documentation".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: read_only_authorization(google_workspace_authorization()),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_workspace_gws_schema".to_string(),
            description: "Inspect request and response schema metadata for an underlying Google Workspace CLI method. This is a support tool for preparing advanced CLI operations when a direct typed action does not cover the requested Workspace operation.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "The CLI method path whose request and response schema should be inspected."
                    }
                },
                "required": ["target"]
            }),
            capabilities: vec!["google_workspace".to_string(), "schema_inspection".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: read_only_authorization(google_workspace_authorization()),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_workspace_gws_skills".to_string(),
            description: "List or read generated Google Workspace CLI skill documentation for the currently granted Workspace bundles. This is a support tool that returns documentation about available CLI helpers, not user Workspace data.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Optional generated CLI skill name to open."
                    },
                    "filter": {
                        "type": "string",
                        "description": "Optional text filter to narrow the catalog by skill name, description, or cli help."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of catalog entries to list when name is omitted. Default: 80."
                    }
                }
            }),
            capabilities: vec!["google_workspace".to_string(), "tool_documentation".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: read_only_authorization(google_workspace_authorization()),
        }).await;

        self.register_builtin_action(ActionDef {
            name: "google_workspace_gws_command".to_string(),
            description: "Execute an advanced Google Workspace CLI command against the connected account within granted Workspace bundles. This is a generic executor for Workspace operations that are not covered by a direct typed action and may require approval.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "argv": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Arguments after `gws`. Do not include the `gws` binary itself."
                    },
                    "required_bundles": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional canonical Workspace bundle IDs required by the command."
                    }
                },
                "required": ["argv"]
            }),
            capabilities: vec!["google_workspace".to_string(), "generic_tool_executor".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["google_workspace_command".to_string()],
                integration_ids: vec!["google_workspace".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        // SSH - remote server execution (behind feature flag)
        #[cfg(feature = "ssh")]
        {
            self.register_builtin_action(ActionDef {
                name: "ssh".to_string(),
                description: "Execute a command on a configured remote server via SSH. Use when asked to check server status, deploy, manage services, run remote commands, or anything involving a remote server.".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "connection": { "type": "string", "description": "Name of the SSH connection to use (from configured connections)" },
                        "command": { "type": "string", "description": "Shell command to execute on the remote server" }
                    },
                    "required": ["connection", "command"]
                }),
                capabilities: vec!["network".to_string(), "ssh".to_string()],
                sandbox_mode: Some(SandboxMode::Native),
                source: ActionSource::System,
                file_path: None,
                authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                    permission_ids: vec!["ssh".to_string()],
                    requires_ssh_connection: true,
                    ..crate::actions::ActionAccessMetadata::default()
                }),
            }).await;

            self.register_builtin_action(ActionDef {
                name: "ssh_connections".to_string(),
                description: "List available SSH connections. Use before ssh to know which servers are configured.".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
                capabilities: vec![],
                sandbox_mode: Some(SandboxMode::Native),
                source: ActionSource::System,
                file_path: None,
            authorization: Default::default(),
            }).await;
        }

        // Generic managed service primitive. The model sees this instead of
        // app-specific lifecycle compatibility actions.
        self.register_builtin_action(ActionDef {
            name: "service_manage".to_string(),
            description: "Create, update, inspect, restart, stop, or delete a managed local service. Use this for durable browser-runnable apps, dashboards, games, tools, repo deployments, local services, and other outputs that must remain available after the turn. Stage source files with file_write/file_patch first when useful, then call operation=create/update with source_dir, files, or repo_url; include source_paths only when deploying a deliberate subset of source_dir. Use update/patch for the active app in the conversation; use duplicate_policy=create_new or allow_duplicate=true only when a separate new app is intentionally requested. Deploy completion is registration/startup evidence, not proof that browser JavaScript, client-side fetches, or the full requested workflow worked. Use schedule_task for independent future/recurring work; put refresh timers that belong to the delivered UI inside the service itself. This is the generic service primitive; legacy app_* lifecycle actions are compatibility surfaces.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["create", "update", "patch", "start", "restart", "stop", "status", "logs", "list", "delete"],
                        "description": "Lifecycle operation for the managed service."
                    },
                    "service_id": {
                        "type": "string",
                        "description": "Stable service/app id. For updates and controls, use this when known."
                    },
                    "name": {
                        "type": "string",
                        "description": "Human-readable service name/title."
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional service title/id query when service_id is not known."
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["auto", "static", "process", "repo"],
                        "default": "auto",
                        "description": "Service shape. Static services are served as files; process/repo services have lifecycle commands."
                    },
                    "source_dir": {
                        "type": "string",
                        "description": "Workspace/data directory already populated with service files."
                    },
                    "source_paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional relative paths under source_dir to include when only a subset should be published."
                    },
                    "files": {
                        "type": "object",
                        "description": "Optional complete file map for small bundles."
                    },
                    "file_patches": {
                        "type": "array",
                        "items": { "type": "object" },
                        "description": "Unified diffs for patching an existing service."
                    },
                    "delete_paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "App-relative files to remove during patch/update."
                    },
                    "repo_url": {
                        "type": "string",
                        "description": "Public Git repository URL to clone and run locally."
                    },
                    "repo_ref": { "type": "string" },
                    "repo_subdir": { "type": "string" },
                    "start_command": {
                        "type": "string",
                        "description": "Command to start a process service. Omit for static browser-only services."
                    },
                    "install_command": {
                        "type": "string",
                        "description": "Optional dependency installation command."
                    },
                    "stop_command": {
                        "type": "string",
                        "description": "Optional graceful stop hook."
                    },
                    "port": { "type": "integer" },
                    "environment": {
                        "type": "object",
                        "additionalProperties": { "type": ["string", "number", "boolean"] },
                        "description": "Non-sensitive runtime configuration values. Secrets must use required_inputs/required_secrets."
                    },
                    "required_inputs": {
                        "type": "array",
                        "items": {
                            "oneOf": [
                                { "type": "string" },
                                {
                                    "type": "object",
                                    "properties": {
                                        "key": { "type": "string" },
                                        "sensitive": { "type": "boolean" }
                                    },
                                    "required": ["key"]
                                }
                            ]
                        }
                    },
                    "required_secrets": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "public": {
                        "type": "boolean",
                        "description": "Expose through the configured public app surface. Default false."
                    },
                    "access_guard": { "type": "boolean" },
                    "access_password": { "type": "string" },
                    "duplicate_policy": {
                        "type": "string",
                        "enum": ["reuse_existing", "create_new"],
                        "default": "reuse_existing",
                        "description": "Reuse/skip an identical existing service by default. Use create_new only when the user explicitly wants another duplicate deployment."
                    },
                    "allow_duplicate": {
                        "type": "boolean",
                        "description": "Compatibility boolean for duplicate_policy=create_new. Default false."
                    }
                },
                "required": ["operation"],
                "additionalProperties": true
            }),
            capabilities: vec![
                "app_hosting".to_string(),
                "service_management".to_string(),
            ],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["app_hosting".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        })
        .await;

        // App deployment - write files, start servers, return live URL
        self.register_builtin_action(ActionDef {
            name: "app_deploy".to_string(),
            description: format!(
                "Deploy a web app or server and return a live URL. Supports generated files, files staged in the data-owned workspace, line-level patches to an existing app, explicit file deletes, OR a repository source. Use when the intended outcome is a managed browser-usable or hosted artifact, such as building a dashboard, creating a tool, making a website, building an app, or deploying/running a repo locally for the user. External publishing is explicit: deploy_target defaults to local; set deploy_target=\"vercel_direct\" only when the selected app deployment layer is Vercel direct API publishing, or deploy_target=\"vercel_git\" only when the selected layer is Git-backed Vercel. If the requested timing/cadence describes how the generated artifact refreshes, polls, auto-updates, backfills, or presents live data, implement that behavior inside the artifact rather than creating an AgentArk schedule or watcher. Build the smallest working app that satisfies the requested workflow, with polished responsive UI, clear controls, and useful loading/empty/error states. Keep generated bundles lean: avoid unrelated routes, auth, databases, admin areas, test suites, generated boilerplate, package manifests, server files, or lifecycle commands unless the user's intent semantically requires them. Prefer a standalone static/browser bundle when the requested behavior can run with browser APIs, timers, client-side state, and public same-origin/app-scoped fetch. Use a dynamic backend/runtime only for server-only needs: secret credentials, authenticated server-side API access, durable jobs that must continue with no browser open, durable server-side state/databases, filesystem/process access, webhooks, private-network access, non-HTTP protocols, or APIs that the browser/app proxy cannot safely call. {inline_report_boundary} For generated multi-file apps, prefer staging each file with `file_write` under one data-owned workspace subdirectory, then call app_deploy with `source_dir`; include `source_paths` only when intentionally deploying a subset. This gives the user per-file progress and avoids one giant deploy payload. For follow-up defects, runtime errors, or requested changes to a known existing app, inspect the existing deployment status/logs/source first, run targeted diagnostic commands when status/logs/source are insufficient, then choose the smallest sufficient operation: app_restart for runtime-only recovery, mode=\"patch\" with app_id and file_patches for localized source changes, or full files/source replacement only when the required change is broad. Keep the existing app_id unless the user intentionally wants another deployment; do not regenerate a full app, re-emit unrelated files, or reinstall dependencies when a targeted patch/restart resolves the underlying issue. For small file-based apps, you may instead provide a `files` object containing every local file needed by the page: if HTML/CSS references a local stylesheet, script, image, font, manifest, or media asset, include that file too. The delivered app must implement the requested workflow and controls; do not substitute a placeholder, mock-only screen, weaker implementation, or decorative shell when the user asked for working behavior or stated implementation preferences. Include request_context and acceptance_criteria so deploy review can validate the delivered app against the requested semantic contract without relying on exact user phrasing. Static browser apps should omit package manifests, server files, `entry_command`, and `start_command` unless a real runtime is needed. Local asset paths must be app-relative, not root-relative. For generated static apps that read public APIs, prefer app-relative {} helpers over third-party CORS proxy services. The app-scoped `__agentark/http/fetch?url=...` helper performs same-origin public GET/HEAD requests for public hosts referenced by the deployed app source; it is not for private networks or secrets. Authenticated API apps are supported, but do not embed credentials in browser JavaScript or static files. Build a dynamic backend/proxy when an API needs secret headers/tokens, declare the needed keys in `required_inputs`/`required_secrets`, read them from process env at runtime, and use `config` only for non-sensitive values such as base URLs. AgentArk's own model/provider credentials are not inherited by generated apps; app credentials must be supplied intentionally through the secure credential store. When modifying a known deployed app, provide its stable `app_id`; otherwise a new deployment is created unless duplicate detection finds a matching app to reuse or replace. Use allow_duplicate=true or duplicate_policy=create_new only when a separate new deployment is intentionally requested. The returned deploy result confirms registration/startup scope only; do not claim browser JavaScript, client-side fetches, or end-to-end workflow validation unless a separate browser/runtime check was run and passed. For repo-based apps, provide `repo_url` (and optionally `repo_ref`, `repo_subdir`, `service_mode`) so {} can clone the repo, inspect the README/manifests, stand up the detected frontend/backend services, and return managed endpoints. For generated file bundles, provide `entry_command` or `start_command` only when the app needs a long-lived server/runtime; a start command makes the app dynamic unless `runtime_required=false` is explicitly supplied. Generated dynamic bundles may be Python, Node/TypeScript, Rust, or another direct-command stack when the files include complete project configuration plus appropriate lifecycle commands. Dynamic app runtimes persist their app directory and lifecycle commands, can install dependencies with network access before startup (`pip`, `npm`, `cargo`, etc.), can run optional multi-step direct setup commands separated by &&, can run an optional `stop_command` as a graceful stop hook, and restart from saved metadata. Repo-based deploys default to container runtime unless overridden. Dynamic app containers default to the installed {} image unless `runtime_image` or a runner-image env override is provided; use `runtime_image` for specialized toolchains not present in the default runner. Deployment is local by default. Content visibility or audience requirements inside the app are not the same as external network exposure; set expose_public only when the deployment target itself is external/public internet exposure. Local app deployments stay local and access guard defaults to off unless the user explicitly enables local App Guard or supplies a local access password. Public exposure does not change the local URL or local guard setting; the public app surface is protected by App Guard and AgentArk generates a public access password if one is not supplied. After deployment, direct the user to the Apps page for start, stop, restart, logs, App Guard, public exposure, and delete controls. Declare required inputs via required_inputs and mark each item sensitive=true/false.",
                crate::branding::PRODUCT_NAME,
                crate::branding::PRODUCT_NAME,
                crate::branding::PRODUCT_NAME,
                inline_report_boundary =
                    crate::core::platform::inline_artifacts::app_deploy_inline_report_boundary()
            ),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "app_id": {
                        "type": "string",
                        "description": "Optional stable deployed app id to update in place. Use when modifying a known existing generated app; omit when creating a new app."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["replace", "patch"],
                        "default": "replace",
                        "description": "replace creates or replaces the declared app bundle and removes stale managed files. patch requires app_id and applies only file_patches, complete changed files in files or resolved source_dir contents, and delete_paths while preserving all other managed files."
                    },
                    "files": {
                        "type": "object",
                        "description": "Object mapping filename to file content. Include every locally referenced asset; use relative paths such as \"style.css\", \"app.js\", or \"assets/logo.svg\", not \"/style.css\". For generated static pages, prefer {\"index.html\":\"<html>...<link rel=\\\"stylesheet\\\" href=\\\"style.css\\\">...\", \"style.css\":\"body{...}\", \"app.js\":\"...\"} over large inline style/script blocks. Each value must be the complete file body."
                    },
                    "file_patches": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": { "type": "string", "description": "App-relative file path to patch." },
                                "patch": { "type": "string", "description": "Unified diff hunks for this file. Include context lines so the patch can be verified against the current file." }
                            },
                            "required": ["path", "patch"]
                        },
                        "description": "Line-level unified diffs to apply when mode='patch'. This lets small edits avoid re-emitting whole files."
                    },
                    "delete_paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "App-relative files to remove from the existing app bundle. Use with mode='patch' for deletions; replace mode also removes stale managed files not declared in files or resolved source_dir contents."
                    },
                    "source_dir": {
                        "type": "string",
                        "description": "Optional data-owned workspace/data directory already populated with app files via file_write. If source_paths is omitted, app_deploy discovers deployable files under this directory instead of receiving a large files object."
                    },
                    "source_paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional app-relative file paths under source_dir to include when only a subset should be published, such as [\"index.html\", \"style.css\", \"src/App.tsx\"]."
                    },
                    "repo_url": {
                        "type": "string",
                        "description": format!(
                            "Public Git repository URL to clone and deploy, e.g. https://github.com/org/repo. Use this instead of `files` when the user wants {} to run an existing repo locally.",
                            crate::branding::PRODUCT_NAME
                        )
                    },
                    "repo_ref": {
                        "type": "string",
                        "description": "Optional branch, tag, or commit-ish to check out after cloning the repo."
                    },
                    "repo_subdir": {
                        "type": "string",
                        "description": "Optional subdirectory inside the cloned repo to treat as the deployment root."
                    },
                    "service_mode": {
                        "type": "string",
                        "enum": ["auto", "frontend", "backend", "fullstack"],
                        "description": "For repo deploys, choose which service(s) to stand up. auto deploys the detected default services."
                    },
                    "deploy_target": {
                        "type": "string",
                        "enum": ["local", "vercel_direct", "vercel_git"],
                        "default": "local",
                        "description": "Explicit app deployment layer. local creates the standard AgentArk /apps/{id} deployment. vercel_direct also publishes the resulting app bundle to Vercel through the REST API using the configured Vercel token. vercel_git records the Git-backed Vercel intent and returns a structured nudge when Git or Vercel project configuration is missing."
                    },
                    "production": {
                        "type": "boolean",
                        "description": "For external Vercel publishing, deploy to production when true; otherwise create a preview deployment. Production publishing should be explicit."
                    },
                    "vercel_project_mode": {
                        "type": "string",
                        "enum": ["auto", "existing", "create"],
                        "default": "auto",
                        "description": "Project handling for external Vercel publishing. auto uses the saved or generated project name with the deployment API. existing requires a saved or supplied project id/name. create calls the Vercel Projects API before deploying and then deploys into that project."
                    },
                    "vercel_project_id": {
                        "type": "string",
                        "description": "Optional Vercel project id/name for external Vercel publishing. If omitted, the saved Vercel project setting or a generated project name is used."
                    },
                    "vercel_team_id": {
                        "type": "string",
                        "description": "Optional Vercel team id used as the teamId API query parameter for external Vercel publishing."
                    },
                    "build_command": {
                        "type": "string",
                        "description": "Optional Vercel projectSettings.buildCommand for source-based deployments such as Next.js apps."
                    },
                    "output_dir": {
                        "type": "string",
                        "description": "Optional Vercel projectSettings.outputDirectory for source-based deployments."
                    },
                    "title": { "type": "string", "description": "App name/title (default: App)" },
                    "request_context": {
                        "type": "string",
                        "description": "Semantic summary of the user-requested app outcome, including explicit implementation preferences or constraints. Used for deploy acceptance review; do not use this as a trigger phrase."
                    },
                    "acceptance_criteria": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Concrete requested capabilities, workflows, data/persistence requirements, runtime/integration constraints, and user preferences that the deployed app must satisfy."
                    },
                    "entry_command": {
                        "type": "string",
                        "description": "Command to start the server process (omit for static HTML apps). Supplying this makes the app a persistent dynamic runtime unless runtime_required=false is explicitly set. Use {PORT} placeholder or PORT env var for the port. Python apps auto-activate their venv. Examples: 'python3 app.py', 'node server.js', 'npm run start', 'uvicorn app:app --host 0.0.0.0 --port {PORT}', 'cargo run'"
                    },
                    "start_command": {
                        "type": "string",
                        "description": "Alias for entry_command. Use when the generated app or repo naturally describes its lifecycle as start/stop commands. It is persisted and used by the Apps UI Start/Restart action."
                    },
                    "install_command": {
                        "type": "string",
                        "description": "One or more direct setup commands to run before starting (optional). Separate multiple direct setup steps with &&; each step is parsed and executed separately without a shell, so shell snippets, pipes, redirection, and shell interpreters are rejected. Omit for Python apps with requirements.txt - a venv is auto-created. Each app runs in its own persistent isolated environment (Python venv, local node_modules, or stack-specific build cache), and dynamic runtime dependency installs may use network access. Examples: 'pip install -r requirements.txt', 'npm install && npm run build', 'cargo fetch'"
                    },
                    "stop_command": {
                        "type": "string",
                        "description": "Optional direct command to run from the app directory before the managed runtime is stopped. Used as a best-effort graceful stop hook by the Apps UI Stop action. Keep it a single direct command such as 'npm run stop'; shell operators are rejected."
                    },
                    "commands": {
                        "type": "object",
                        "description": "Optional lifecycle command block. Supported keys: install/setup, start/entry, and stop. Values are persisted in app metadata and used by the Apps UI Start/Restart/Stop actions.",
                        "additionalProperties": { "type": "string" }
                    },
                    "required_inputs": {
                        "type": "array",
                        "items": {
                            "oneOf": [
                                { "type": "string" },
                                {
                                    "type": "object",
                                    "properties": {
                                        "key": { "type": "string" },
                                        "sensitive": { "type": "boolean" }
                                    },
                                    "required": ["key"]
                                }
                            ]
                        },
                        "description": "Required runtime inputs. String entries default to sensitive=true. Use object entries for per-key sensitivity, e.g. [{\"key\":\"API_TOKEN\",\"sensitive\":true},{\"key\":\"BASE_URL\",\"sensitive\":false}]. For authenticated APIs, declare secret headers/tokens here and read them from process env in a dynamic backend rather than embedding them in static browser files."
                    },
                    "required_secrets": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Compatibility alias for sensitive required inputs."
                    },
                    "required_config": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Compatibility alias for non-sensitive required inputs."
                    },
                    "required_env": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Legacy alias for required_secrets."
                    },
                    "config": {
                        "type": "object",
                        "additionalProperties": { "type": ["string", "number", "boolean"] },
                        "description": "Optional non-sensitive runtime config values (e.g. BASE_URL). Values are stored in app metadata for restart/restore."
                    },
                    "runtime_actions": {
                        "type": "array",
                        "items": {
                            "oneOf": [
                                { "type": "string" },
                                {
                                    "type": "object",
                                    "properties": {
                                        "action": { "type": "string" }
                                    },
                                    "required": ["action"]
                                }
                            ]
                        },
                        "description": "Registered read-only AgentArk action names this app may call through its app-scoped runtime action bridge. The bridge executes through the normal runtime registry and credential resolver, so saved integration credentials are never embedded in app files or exposed to the model."
                    },
                    "runtime_image": {
                        "type": "string",
                        "description": format!(
                            "Optional container image used to run the app. Defaults to the installed {} image when available; use this only to override with a dedicated runner image.",
                            crate::branding::PRODUCT_NAME
                        )
                    },
                    "runtime_preference": {
                        "type": "string",
                        "enum": ["local", "container"],
                        "description": format!(
                            "Preferred runtime for dynamic apps. Default: container when Docker is configured for {}, otherwise local.",
                            crate::branding::PRODUCT_NAME
                        )
                    },
                    "runtime_required": {
                        "type": "boolean",
                        "description": "Whether the generated bundle needs a long-lived server/runtime. Omit to infer from entry_command/start_command; set false only when a bundle with lifecycle metadata should still be served as static files."
                    },
                    "runtime_reason": {
                        "type": "string",
                        "description": "Optional short explanation of why a dynamic runtime is needed for this generated bundle."
                    },
                    "expose_public": {
                        "type": "boolean",
                        "description": "Whether to expose this deployment through the configured remote-access provider. Default: false; ordinary app deployment remains local even if the app content is intended to be shared or read-only."
                    },
                    "access_guard": {
                        "type": "boolean",
                        "description": "Enable access-password guard for the local app URL. Defaults to false for local app deployments. Public exposure has its own mandatory public-surface guard and does not change this local setting."
                    },
                    "access_password": {
                        "type": "string",
                        "description": "Optional operator-chosen access password. Providing it enables local App Guard unless public exposure is the only guarded surface. If public exposure is requested and this is omitted, AgentArk generates a public-surface password."
                    },
                    "replace_existing": {
                        "type": "boolean",
                        "description": "Update/recreate the targeted deployed app in place when an app_id or matching app is available. Default: false."
                    },
                    "allow_duplicate": {
                        "type": "boolean",
                        "description": "Create another matching app deployment instead of reusing/updating a matching existing app. Default false."
                    },
                    "duplicate_policy": {
                        "type": "string",
                        "enum": ["reuse_existing", "create_new"],
                        "default": "reuse_existing",
                        "description": "Reuse/skip an identical existing app by default. Use create_new only when the user explicitly wants another duplicate deployment."
                    }
                }
            }),
            capabilities: vec!["app_hosting".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: authorization_with_access(crate::actions::ActionAccessMetadata {
                permission_ids: vec!["app_hosting".to_string()],
                ..crate::actions::ActionAccessMetadata::default()
            }),
        }).await;

        // Provider-based text/image-to-video generation (Runway/Luma/Fal/Veo/etc.)
        self.register_builtin_action(ActionDef {
            name: "generate_video".to_string(),
            description: "Generate an AI video via configured video providers (Runway, Luma, Fal, Sora, Veo, etc.) for text-to-video or image-to-video requests.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Video prompt/description" },
                    "image_url": { "type": "string", "description": "Optional source image URL for image-to-video models" },
                    "duration_seconds": { "type": "integer", "minimum": 1, "maximum": 12, "description": "Desired duration in seconds (model-dependent; default 4)" },
                    "aspect_ratio": { "type": "string", "description": "Optional aspect ratio (e.g. 16:9, 9:16, 1:1)" },
                    "model": { "type": "string", "description": "Optional provider model override" },
                    "provider": { "type": "string", "description": "Optional provider override (replicate, runway, luma, fal, openai_sora, google_veo, etc.)" }
                },
                "required": ["prompt"]
            }),
            capabilities: vec!["video_generation".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: integration_authorization("media_gen"),
        }).await;

        // Self-evolve - policy-first self-improvement
        self.register_builtin_action(ActionDef {
            name: "self_evolve".to_string(),
            description: format!(
                "Evolve {} behavior with an auditable promotion loop. Supports policy/strategy, prompt, specialist prompt, and GEPA flows through benchmark, lineage archive, statistical gating, canary rollout, replay gate, and optional promotion. This tool does not mutate the local source tree.",
                crate::branding::PRODUCT_NAME
            ),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "request": {
                        "type": "string",
                        "description": "Natural language description of what should evolve"
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["policy", "strategy", "prompt", "specialist_prompt", "gepa_export", "gepa_run", "gepa_import", "gepa_status"],
                        "description": "Evolution mode. policy (default) evolves runtime strategy; prompt/specialist_prompt evolve prompt surfaces; GEPA modes run offline seed export/run/import/status."
                    },
                    "apply_promotion": {
                        "type": "boolean",
                        "description": "For policy mode: apply promoted policy by activating canary rollout and replay gate. Default true."
                    },
                    "canary_rollout_percent": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "description": "Traffic percentage for candidate policy during canary rollout. Default 20."
                    },
                    "canary_min_samples_per_version": {
                        "type": "integer",
                        "minimum": 5,
                        "description": "Minimum baseline/candidate samples required for replay promotion. Default 25."
                    },
                    "canary_min_success_gain": {
                        "type": "number",
                        "minimum": 0.0,
                        "description": "Minimum success-rate improvement required for promotion. Default 0.03."
                    },
                    "canary_max_sign_test_p_value": {
                        "type": "number",
                        "minimum": 0.0001,
                        "maximum": 1.0,
                        "description": "Maximum one-sided sign-test p-value for promotion. Default 0.10."
                    },
                    "replay_log_limit": {
                        "type": "integer",
                        "minimum": 100,
                        "description": "Operational log window size used for replay evaluation. Default 4000."
                    },
                    "gepa_run_id": {
                        "type": "string",
                        "description": "Optional GEPA run id used to locate .agentark/self_evolve/gepa/runs/<run_id> artifacts."
                    },
                    "export_path": {
                        "type": "string",
                        "description": "Optional path to a GEPA export.json file for gepa_run."
                    },
                    "candidates_path": {
                        "type": "string",
                        "description": "Optional path to GEPA candidates.jsonl for gepa_import or gepa_run output."
                    },
                    "gepa_quiet_window_seconds": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Quiet window required before GEPA work starts. Default 60."
                    },
                    "gepa_optimizer_timeout_seconds": {
                        "type": "integer",
                        "minimum": 30,
                        "description": "Maximum wall-clock seconds for the offline GEPA optimizer process. Default 900."
                    }
                }
            }),
            capabilities: vec!["self_evolve".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        authorization: Default::default(),
        }).await;

        // ==================== ArkOrbit (per-user canvas) ====================

        self.register_builtin_action(ActionDef {
            name: "arkorbit_create_orbit".to_string(),
            description: "Create a new ArkOrbit canvas backed by durable orbit files. Use when the user wants a fresh, separate space for a different topic, project, or purpose. The new canvas is owned by the active user and persisted to disk; it does not become the default unless the user has none yet.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Short human-readable label shown in the orbit switcher."
                    },
                    "icon": {
                        "type": "string",
                        "description": "Optional emoji or short glyph rendered alongside the name in the switcher."
                    },
                    "color": {
                        "type": "string",
                        "description": "Optional CSS color string (e.g. '#7c3aed') used to tint the orbit chip."
                    },
                    "agent_instructions": {
                        "type": "string",
                        "description": "Optional free-form instructions scoped to this orbit. The agent receives them as structural context whenever the user chats inside this canvas."
                    }
                },
                "required": ["name"]
            }),
            capabilities: vec!["arkorbit".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "arkorbit_file_write".to_string(),
            description: "Fallback write primitive for ArkOrbit files. The fast orbit chat path normally applies structured orbit file operations directly; this action exists for non-streaming providers and other structured tool paths. The path must be index.html, orbit.json, or under mod/, data/, or assets/.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "orbit_id": {
                        "type": "string",
                        "description": "Selected orbit identifier."
                    },
                    "path": {
                        "type": "string",
                        "description": "Relative orbit file path. Allowed roots: mod/, data/, assets/, index.html, orbit.json."
                    },
                    "content": {
                        "type": "string",
                        "description": "Full file contents to write atomically."
                    }
                },
                "required": ["orbit_id", "path", "content"]
            }),
            capabilities: vec!["arkorbit".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        })
        .await;

        Ok(())
    }
}
