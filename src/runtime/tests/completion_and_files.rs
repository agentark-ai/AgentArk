use super::*;

#[test]

fn parse_schedule_task_completion_accepts_structured_marker() {
    let structured = format!(
        "{}{}",
        TOOL_COMPLETION_MARKER,
        serde_json::json!({

            "tool": "schedule_task",

            "status": "completed",

            "detail": "Task scheduled"

        })
    );

    let structured = parse_schedule_task_completion(&structured)
        .expect("structured schedule marker should parse");

    assert_eq!(structured.tool, "schedule_task");

    assert_eq!(structured.status, "completed");
}

#[test]

fn parse_watch_completion_accepts_structured_marker() {
    let structured = format!(
        "{}{}",
        TOOL_COMPLETION_MARKER,
        serde_json::json!({

            "tool": "watch",

            "status": "completed",

            "detail": "Watch created"

        })
    );

    let structured =
        parse_watch_completion(&structured).expect("structured watch marker should parse");

    assert_eq!(structured.tool, "watch");

    assert_eq!(structured.status, "completed");
}

#[test]

fn permission_requirement_error_uses_redaction_safe_labels() {
    let message = ActionRuntime::build_permission_requirement_error(
        "workspace_executor",
        &[crate::security::action_guard::Permission::Custom(
            "google_workspace_command".to_string(),
        )],
    );

    let redacted = crate::security::redact_secret_input(&message).text;

    assert!(!redacted.contains("[REDACTED_SECRET]"));

    assert!(redacted.contains("google workspace command"));

    assert!(!redacted.contains("Auto-Approve Skills"));
}

#[tokio::test]

async fn watch_action_rejects_incomplete_single_watcher() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let error = runtime
        .execute_action(
            "watch",
            &serde_json::json!({

                "description": "Monitor connected files"

            }),
        )
        .await
        .expect_err("incomplete watcher must not produce a completion marker");

    assert!(!error.to_string().contains(TOOL_COMPLETION_MARKER));
}

#[tokio::test]

async fn file_search_finds_filename_and_content_matches() {
    let temp = tempfile::tempdir().unwrap();

    let root = temp.path().join("workspace");

    std::fs::create_dir_all(&root).unwrap();

    std::fs::write(
        root.join("needle-name.txt"),
        "alpha\nneedle content\nomega\n",
    )
    .unwrap();

    std::fs::write(root.join("other.txt"), "plain text\n").unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let output = runtime
        .execute_action(
            "file_search",
            &serde_json::json!({

                "root": root.display().to_string(),

                "query": "needle",

                "context_lines": 1,

                "limit": 10

            }),
        )
        .await
        .unwrap();

    let payload = parse_tool_completion_output(&output);

    let matches = payload["data"]["matches"].as_array().unwrap();

    assert!(matches
        .iter()
        .any(|item| item["match_type"].as_str() == Some("filename")));

    assert!(matches
        .iter()
        .any(|item| item["match_type"].as_str() == Some("content")
            && item["line_number"].as_u64() == Some(2)));
}

#[tokio::test]

async fn file_search_skips_heavy_directories_by_default() {
    let temp = tempfile::tempdir().unwrap();

    let root = temp.path().join("workspace");

    std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();

    std::fs::create_dir_all(root.join("src")).unwrap();

    std::fs::write(root.join("node_modules/pkg/needle.txt"), "needle\n").unwrap();

    std::fs::write(root.join("src/app.txt"), "plain\n").unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let output = runtime
        .execute_action(
            "file_search",
            &serde_json::json!({

                "root": root.display().to_string(),

                "query": "needle",

                "limit": 10

            }),
        )
        .await
        .unwrap();

    let payload = parse_tool_completion_output(&output);

    assert_eq!(payload["data"]["matches"].as_array().unwrap().len(), 0);

    assert_eq!(payload["data"]["skipped_directories"].as_u64(), Some(1));
}

#[tokio::test]

async fn file_write_copies_resource_ref_bytes_exactly() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let resource_dir = temp
        .path()
        .join(TOOL_PAYLOAD_RESOURCE_DIR)
        .join("resource-1");

    std::fs::create_dir_all(&resource_dir).unwrap();

    let source = resource_dir.join("image.png");

    let bytes = b"\x89PNG\r\n\x1A\nbinary payload".to_vec();

    std::fs::write(&source, &bytes).unwrap();

    let target = temp.path().join("saved").join("image.png");

    let output = runtime
        .execute_action(
            "file_write",
            &serde_json::json!({

                "path": target.display().to_string(),

                "source_resource": {

                    "id": "resource-1",

                    "path": source.display().to_string(),

                    "mime": "image/png",

                    "bytes": bytes.len(),

                    "created_at": "2026-05-19T00:00:00Z",

                    "source_action": "browse"

                }

            }),
        )
        .await
        .unwrap();

    let payload = parse_tool_completion_output(&output);

    assert_eq!(std::fs::read(&target).unwrap(), bytes);

    assert_eq!(
        payload["data"]["payload"]["kind"].as_str(),
        Some("resource")
    );

    assert_eq!(
        payload["data"]["write"]["source_resource"]["id"].as_str(),
        Some("resource-1")
    );
}

#[tokio::test]

async fn file_write_rejects_sensitive_targets() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let new_target = temp.path().join(".env");

    let error = runtime
        .execute_action(
            "file_write",
            &serde_json::json!({

                "path": new_target.display().to_string(),

                "content": "TOKEN=new\n"

            }),
        )
        .await
        .expect_err("new sensitive target should be rejected");

    assert!(error.to_string().contains("sensitive credential file"));

    assert!(!new_target.exists());

    let existing_target = temp.path().join(".env.local");

    std::fs::write(&existing_target, "TOKEN=old\n").unwrap();

    let error = runtime
        .execute_action(
            "file_write",
            &serde_json::json!({

                "path": existing_target.display().to_string(),

                "content": "TOKEN=new\n"

            }),
        )
        .await
        .expect_err("existing sensitive target should be rejected");

    assert!(error.to_string().contains("sensitive credential file"));

    assert_eq!(
        std::fs::read_to_string(&existing_target).unwrap(),
        "TOKEN=old\n"
    );
}

#[tokio::test]

async fn file_write_completion_uses_managed_labels_not_delivery_paths() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    let target = temp.path().join("reports").join("runbook.md");

    let payload = FileWritePayload {
        bytes: b"# Runbook".to_vec(),

        mime: Some("text/markdown".to_string()),

        source_resource: None,
    };

    let output = runtime.file_write_completion_output(&target, &payload, None);

    let parsed = parse_tool_completion_output(&output);

    assert_eq!(
        parsed["detail"].as_str(),
        Some("Saved managed file runbook.md.")
    );

    assert_eq!(
        parsed["data"]["artifact"]["label"].as_str(),
        Some("runbook.md")
    );

    assert!(parsed["data"]["write"].get("path").is_none());
}

#[test]

fn generated_file_metadata_chunk_covers_text_and_binary_documents() {
    let resource = RuntimeResourceRef {
        id: "resource-1".to_string(),

        path: "/hidden/internal/path/image.png".to_string(),

        mime: Some("image/png".to_string()),

        bytes: 42,

        created_at: "2026-05-21T00:00:00Z".to_string(),

        source_action: Some("page_fetch".to_string()),
    };

    let chunk = ActionRuntime::generated_file_metadata_chunk(
        "image.png",
        "image/png",
        42,
        "abc123",
        false,
        Some(&resource),
        None,
    );

    assert!(chunk.contains("artifact_kind: managed_file"));

    assert!(chunk.contains("filename: image.png"));

    assert!(chunk.contains("content_type: image/png"));

    assert!(chunk.contains("file_size_bytes: 42"));

    assert!(chunk.contains("sha256: abc123"));

    assert!(chunk.contains("text_content_indexed: false"));

    assert!(chunk.contains("source_resource_id: resource-1"));

    assert!(!chunk.contains("/hidden/internal/path"));
}

#[test]

fn pdf_text_literal_degrades_common_unicode_punctuation() {
    let text =
        ActionRuntime::pdf_text_literal("LocalAgent\u{2014}Hermes \u{201c}agent\u{201d}\u{2026}");

    assert_eq!(text, "LocalAgent-Hermes \"agent\"...");
}

#[tokio::test]

async fn file_write_completion_reports_metadata_only_documents() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    let target = temp.path().join("assets").join("image.png");

    let payload = FileWritePayload {
        bytes: b"png bytes".to_vec(),

        mime: Some("image/png".to_string()),

        source_resource: None,
    };

    let document = IndexedDocumentArtifact {
        id: "doc-1".to_string(),

        filename: "image.png".to_string(),

        content_type: "image/png".to_string(),

        chunk_count: 1,

        file_size: payload.bytes.len() as u64,

        url: "/ui/documents".to_string(),

        download_url: Some(
            "/api/outputs/00000000-0000-0000-0000-000000000000/image.png/download".to_string(),
        ),

        duplicate_skipped: false,

        content_fingerprint: "abc123".to_string(),

        metadata_only: true,

        index_mode: "metadata".to_string(),
    };

    let output = runtime.file_write_completion_output(&target, &payload, Some(&document));

    let parsed = parse_tool_completion_output(&output);

    assert_eq!(
        parsed["detail"].as_str(),
        Some("Saved managed file image.png and registered its metadata in Documents.")
    );

    assert_eq!(
        parsed["data"]["document"]["index_mode"].as_str(),
        Some("metadata")
    );

    assert_eq!(
        parsed["data"]["document"]["download_url"].as_str(),
        Some("/api/outputs/00000000-0000-0000-0000-000000000000/image.png/download")
    );
}

#[tokio::test]

async fn pdf_generate_returns_managed_pdf_artifact_without_internal_path_handoff() {
    let config_dir = tempfile::tempdir().unwrap();

    let data_dir = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(config_dir.path(), data_dir.path())
        .await
        .unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let output = runtime
        .execute_action(
            "pdf_generate",
            &serde_json::json!({

                "title": "Market Report",

                "filename": "market-report.pdf",

                "style": "report",

                "content": "Executive summary\n\nGenerated PDF content.",

            }),
        )
        .await
        .unwrap();

    let payload = parse_tool_completion_output(&output);

    assert_eq!(payload["tool"].as_str(), Some("pdf_generate"));

    assert_eq!(payload["status"].as_str(), Some("completed"));

    assert_eq!(
        payload["data"]["artifact"]["kind"].as_str(),
        Some("managed_file")
    );

    assert_eq!(
        payload["data"]["artifact"]["content_type"].as_str(),
        Some("application/pdf")
    );

    let download_url = payload["data"]["artifact"]["download_url"]
        .as_str()
        .expect("pdf_generate should expose a direct download URL");

    assert!(download_url.starts_with("/api/outputs/"));

    assert!(download_url.ends_with("/market-report.pdf/download"));

    assert_eq!(
        payload["data"]["payload"]["resource"]["mime"].as_str(),
        Some("application/pdf")
    );

    let resource_path = payload["data"]["payload"]["resource"]["path"]
        .as_str()
        .expect("pdf_generate should keep a private runtime resource path");

    let resource_path = PathBuf::from(resource_path);

    let resource_canonical = std::fs::canonicalize(&resource_path).unwrap();

    let outputs_canonical = std::fs::canonicalize(data_dir.path().join("outputs")).unwrap();

    let repo_canonical = std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap();

    assert!(
        resource_canonical.starts_with(&outputs_canonical),
        "generated PDF must stay under the temp runtime outputs directory"
    );

    assert!(
        !resource_canonical.starts_with(&repo_canonical),
        "generated PDF test artifact must never be written under the source checkout"
    );

    assert!(std::fs::read(&resource_canonical)
        .expect("generated PDF should be saved under the served outputs tree")
        .starts_with(b"%PDF"));

    assert!(
        payload["data"]["document"].is_null(),
        "bare runtimes without attached storage cannot report Documents registration metadata"
    );

    assert!(
        !payload["detail"]
            .as_str()
            .unwrap_or_default()
            .contains(data_dir.path().to_string_lossy().as_ref()),
        "completion detail must not ask the model to chase an internal path"
    );
}

#[tokio::test]

async fn file_write_copies_structured_resource_content_exactly() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let resource_dir = temp
        .path()
        .join(TOOL_PAYLOAD_RESOURCE_DIR)
        .join("resource-2");

    std::fs::create_dir_all(&resource_dir).unwrap();

    let source = resource_dir.join("artifact.pdf");

    let bytes = b"%PDF-1.7\nbinary payload".to_vec();

    std::fs::write(&source, &bytes).unwrap();

    let marker = format!(
        "{}{}",
        TOOL_COMPLETION_MARKER,
        serde_json::json!({

            "tool": "http_get",

            "status": "completed",

            "data": {

                "payload": {

                    "kind": "resource",

                    "resource": {

                        "id": "resource-2",

                        "path": source.display().to_string(),

                        "mime": "application/pdf",

                        "bytes": bytes.len(),

                        "created_at": "2026-05-19T00:00:00Z",

                        "source_action": "http_get"

                    }

                }

            }

        })
    );

    let target = temp.path().join("saved").join("artifact.pdf");

    runtime
        .execute_action(
            "file_write",
            &serde_json::json!({

                "path": target.display().to_string(),

                "content": marker

            }),
        )
        .await
        .unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), bytes);
}

#[tokio::test]

async fn file_read_returns_resource_for_binary_files() {
    let temp = tempfile::tempdir().unwrap();

    let target = temp.path().join("artifact.bin");

    std::fs::write(&target, [0_u8, 159, 146, 150]).unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let output = runtime
        .execute_action(
            "file_read",
            &serde_json::json!({

                "path": target.display().to_string()

            }),
        )
        .await
        .unwrap();

    let payload = parse_tool_completion_output(&output);

    assert_eq!(
        payload["data"]["payload"]["kind"].as_str(),
        Some("resource")
    );

    assert_eq!(
        payload["data"]["body_quality"]["binary"].as_bool(),
        Some(true)
    );

    let target_text = target.display().to_string();

    assert_eq!(
        payload["data"]["payload"]["resource"]["path"].as_str(),
        Some(target_text.as_str())
    );
}

#[tokio::test]

async fn code_execute_files_accept_resource_refs() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    let source = temp.path().join("input.dat");

    let bytes = b"resource input".to_vec();

    std::fs::write(&source, &bytes).unwrap();

    let files = runtime
        .collect_code_execute_files(&serde_json::json!({

            "files": [

                {

                    "id": "file-resource",

                    "path": source.display().to_string(),

                    "mime": "application/octet-stream",

                    "bytes": bytes.len(),

                    "created_at": "2026-05-19T00:00:00Z"

                }

            ]

        }))
        .await
        .unwrap();

    assert_eq!(files.len(), 1);

    assert_eq!(files[0].bytes, bytes);

    assert_eq!(files[0].filename, "input.dat");
}

#[tokio::test]

async fn file_patch_applies_unified_diff_and_reports_summary() {
    let temp = tempfile::tempdir().unwrap();

    let target = temp.path().join("demo.txt");

    std::fs::write(&target, "one\ntwo\nthree\n").unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let output = runtime
        .execute_action(
            "file_patch",
            &serde_json::json!({

                "path": target.display().to_string(),

                "patch": "@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n"

            }),
        )
        .await
        .unwrap();

    let payload = parse_tool_completion_output(&output);

    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "one\nTWO\nthree\n"
    );

    assert_eq!(payload["data"]["changed_count"].as_u64(), Some(1));

    assert_eq!(
        payload["data"]["changed_files"][0]["context_verified"].as_bool(),
        Some(true)
    );
}

#[tokio::test]

async fn file_patch_rejects_sensitive_targets() {
    let temp = tempfile::tempdir().unwrap();

    let target = temp.path().join(".env");

    std::fs::write(&target, "TOKEN=old\n").unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let error = runtime
        .execute_action(
            "file_patch",
            &serde_json::json!({

                "path": target.display().to_string(),

                "patch": "@@ -1 +1 @@\n-TOKEN=old\n+TOKEN=new\n"

            }),
        )
        .await
        .expect_err("sensitive target should be rejected");

    assert!(error.to_string().contains("sensitive credential file"));

    assert_eq!(std::fs::read_to_string(&target).unwrap(), "TOKEN=old\n");
}

#[tokio::test]

async fn file_delete_deletes_allowed_file_and_reports_terminal_not_found() {
    let temp = tempfile::tempdir().unwrap();

    let target = temp.path().join("reports").join("old.md");

    std::fs::create_dir_all(target.parent().unwrap()).unwrap();

    std::fs::write(&target, "# Old report\n").unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let output = runtime
        .execute_action(
            "file_delete",
            &serde_json::json!({

                "path": target.display().to_string()

            }),
        )
        .await
        .unwrap();

    let payload = parse_tool_completion_output(&output);

    assert!(!target.exists());

    assert_eq!(payload["status"].as_str(), Some("completed"));

    assert_eq!(payload["data"]["status"].as_str(), Some("deleted"));

    assert_eq!(payload["data"]["deleted"].as_bool(), Some(true));

    let output = runtime
        .execute_action(
            "file_delete",
            &serde_json::json!({

                "path": target.display().to_string()

            }),
        )
        .await
        .unwrap();

    let payload = parse_tool_completion_output(&output);

    assert_eq!(payload["status"].as_str(), Some("completed"));

    assert_eq!(payload["data"]["status"].as_str(), Some("not_found"));

    assert_eq!(payload["data"]["deleted"].as_bool(), Some(false));

    assert_eq!(
        payload["data"]["terminal_observation"].as_bool(),
        Some(true)
    );
}

#[tokio::test]

async fn browser_wrapper_actions_are_registered() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    for name in [
        "browser_navigate",
        "browser_click",
        "browser_type",
        "browser_scroll",
        "browser_snapshot",
        "browser_screenshot",
        "browser_back",
        "browser_press",
        "browser_console",
    ] {
        let action = action_def_by_name(&runtime, name).await;

        assert!(action.capabilities.contains(&"browser".to_string()));
    }
}

#[tokio::test]

async fn page_fetch_is_registered_as_direct_url_fetcher() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let action = action_def_by_name(&runtime, "page_fetch").await;

    assert!(action.capabilities.contains(&"page_fetch".to_string()));

    assert!(action.capabilities.contains(&"url_fetch".to_string()));

    assert_eq!(action.sandbox_mode, Some(SandboxMode::Native));
}

#[test]

fn generic_runtime_download_contract_recognizes_non_text_resource_urls() {
    let image_url =
        reqwest::Url::parse("https://cdn.example.test/assets/rendered-output.png").unwrap();

    let page_url = reqwest::Url::parse("https://example.test/docs/index.html").unwrap();

    let json_url = reqwest::Url::parse("https://example.test/api/result.json").unwrap();

    assert_eq!(runtime_url_expected_mime(&image_url), Some("image/png"));

    assert!(runtime_url_expects_non_text_resource(&image_url));

    assert!(!runtime_url_expects_non_text_resource(&page_url));

    assert!(!runtime_url_expects_non_text_resource(&json_url));
}

#[test]

fn generic_runtime_download_contract_rejects_text_for_image_resource_url() {
    let image_url =
        reqwest::Url::parse("https://cdn.example.test/assets/rendered-output.png").unwrap();

    let expected_mime = runtime_url_expected_mime(&image_url);

    assert!(!runtime_response_matches_expected_url_mime(
        expected_mime,
        "text/html; charset=utf-8",
        b"not image bytes"
    ));

    assert!(runtime_response_matches_expected_url_mime(
        expected_mime,
        "image/png",
        &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
    ));
}

#[tokio::test]

async fn generic_runtime_download_contract_code_execute_does_not_advertise_unwired_wasm_backend() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let action = action_def_by_name(&runtime, "code_execute").await;

    let backends = action
        .input_schema
        .pointer("/properties/backend/enum")
        .and_then(|value| value.as_array())
        .expect("code_execute backend enum should exist")
        .iter()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        backends,
        vec!["auto", "docker", "native", "executor_server"]
    );

    assert!(!backends.contains(&"wasm"));

    let preferred_backends = action
        .input_schema
        .pointer("/properties/backend_preference/properties/preferred/items/enum")
        .and_then(|value| value.as_array())
        .expect("backend_preference enum should exist")
        .iter()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        preferred_backends,
        vec!["docker", "native", "remote_executor"]
    );

    assert!(!preferred_backends.contains(&"wasm"));
}

#[tokio::test]

async fn generic_runtime_download_contract_code_execute_rejects_stale_wasm_backend_requests() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let error = runtime
        .execute_action(
            "code_execute",
            &serde_json::json!({

                "language": "python",

                "code": "print('ok')",

                "backend": "wasm"

            }),
        )
        .await
        .expect_err("unwired wasm backend should not be accepted");

    assert!(error
        .to_string()
        .contains("code_execute backend 'wasm' is not wired"));
}

#[tokio::test]

async fn imported_wasm_modules_fail_closed() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.actions.write().await.insert(
        "custom_wasm".to_string(),
        LoadedAction {
            info: ActionDef {
                name: "custom_wasm".to_string(),

                sandbox_mode: Some(SandboxMode::Wasm),

                ..ActionDef::default()
            },

            builtin_handler: None,

            supports_background: false,

            wasm_module: Some(vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]),

            workflow_content: None,

            cli_binding: None,

            mcp_binding: None,

            plugin_binding: None,

            custom_api_binding: None,

            extension_pack_binding: None,
        },
    );

    let error = runtime
        .execute_action("custom_wasm", &serde_json::json!({}))
        .await
        .expect_err("imported wasm modules should not execute");

    assert!(error
        .to_string()
        .contains("Imported WASM module execution is disabled"));
}

#[test]

fn generic_runtime_download_contract_binary_detection_is_byte_based() {
    assert!(!runtime_response_body_is_probably_binary(
        "application/octet-stream",
        br#"{"ok":true,"message":"plain utf-8"}"#
    ));

    assert!(!runtime_response_body_is_probably_binary(
        "application/octet-stream",
        b"# Heading\n\nPlain markdown bytes."
    ));

    assert!(runtime_response_body_is_probably_binary(
        "application/octet-stream",
        &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
    ));

    assert!(runtime_response_body_is_probably_binary(
        "text/plain",
        &[0x00, 0x01, 0x02, 0x03]
    ));
}

#[tokio::test]

async fn generic_runtime_download_contract_http_request_exposes_raw_byte_save_path() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let action = action_def_by_name(&runtime, "http_request").await;

    let save_to = action
        .input_schema
        .pointer("/properties/save_to/type")
        .and_then(|value| value.as_str());

    assert_eq!(save_to, Some("string"));
}

#[tokio::test]

async fn generic_runtime_download_contract_http_request_can_return_resource_without_user_path() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let action = action_def_by_name(&runtime, "http_request").await;

    assert_eq!(
        action
            .input_schema
            .pointer("/properties/as_resource/type")
            .and_then(|value| value.as_str()),
        Some("boolean")
    );

    assert_eq!(
        action
            .input_schema
            .pointer("/properties/suggested_name/type")
            .and_then(|value| value.as_str()),
        Some("string")
    );
}

#[tokio::test]

async fn generic_runtime_download_contract_page_fetch_can_return_resource_without_user_path() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let action = action_def_by_name(&runtime, "page_fetch").await;

    assert_eq!(
        action
            .input_schema
            .pointer("/properties/as_resource/type")
            .and_then(|value| value.as_str()),
        Some("boolean")
    );

    assert_eq!(
        action
            .input_schema
            .pointer("/properties/suggested_name/type")
            .and_then(|value| value.as_str()),
        Some("string")
    );
}

#[tokio::test]

async fn http_request_contract_supports_encrypted_response_field_persistence() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let action = action_def_by_name(&runtime, "http_request").await;

    assert_eq!(
        action
            .input_schema
            .pointer("/properties/persist_response/items/properties/secret_key/type")
            .and_then(|value| value.as_str()),
        Some("string")
    );

    assert_eq!(
        action
            .input_schema
            .pointer("/properties/persist_response/items/required/0")
            .and_then(|value| value.as_str()),
        Some("response_path")
    );
}

#[tokio::test]

async fn capability_acquire_contract_supports_default_operation_body() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let action = action_def_by_name(&runtime, "capability_acquire").await;

    assert!(action.input_schema.pointer("/properties/body").is_some());

    assert_eq!(
        action
            .input_schema
            .pointer("/properties/id/type")
            .and_then(|value| value.as_str()),
        Some("string")
    );
}

#[test]

fn generic_runtime_payload_contract_promotes_json_to_structured_payload() {
    let payload =
        ActionRuntime::tool_payload_from_legacy_output("example", "{\"ok\":true}".to_string());

    match payload {
        ToolPayload::Structured(value) => assert_eq!(value["ok"], serde_json::json!(true)),

        other => panic!("unexpected payload: {:?}", other),
    }
}

#[test]

fn generic_runtime_payload_contract_extracts_resource_markers() {
    let resource = RuntimeResourceRef {
        id: "resource-1".to_string(),

        path: "C:/tmp/resource.bin".to_string(),

        mime: Some("application/octet-stream".to_string()),

        bytes: 12,

        created_at: "2026-05-19T00:00:00Z".to_string(),

        source_action: Some("test".to_string()),
    };

    let marker = format!(
        "{}{}",
        TOOL_COMPLETION_MARKER,
        serde_json::json!({

            "tool": "test",

            "status": "completed",

            "data": {

                "payload": {

                    "kind": "resource",

                    "resource": resource,

                }

            }

        })
    );

    let payload = ActionRuntime::tool_payload_from_legacy_output("test", marker);

    match payload {
        ToolPayload::Resource { resource, .. } => {
            assert_eq!(resource.id, "resource-1");

            assert_eq!(resource.bytes, 12);
        }

        other => panic!("unexpected payload: {:?}", other),
    }
}

#[tokio::test]

async fn generic_runtime_payload_contract_persists_byte_payloads() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    let payload = runtime
        .persist_tool_payload_if_needed(
            ToolPayload::Bytes {
                mime: Some("application/octet-stream".to_string()),

                body: vec![0, 1, 2, 3],

                suggested_name: Some("payload.bin".to_string()),
            },
            PersistHints {
                source_action: Some("test".to_string()),

                ..PersistHints::default()
            },
        )
        .await
        .unwrap();

    match payload {
        ToolPayload::Resource { resource, .. } => {
            assert_eq!(resource.bytes, 4);

            assert!(std::path::Path::new(&resource.path).exists());
        }

        other => panic!("unexpected payload: {:?}", other),
    }
}

#[test]

fn generic_runtime_backend_contract_parses_preference_list() {
    let preference = ActionRuntime::code_execute_backend_preference(&serde_json::json!({

        "backend_preference": {

            "preferred": ["remote_executor", "docker", "native"],

            "fallback_policy": "auto_degrade"

        }

    }))
    .unwrap();

    assert_eq!(
        preference.preferred,
        vec![
            RuntimeBackend::RemoteExecutor,
            RuntimeBackend::Docker,
            RuntimeBackend::Native,
        ]
    );

    assert_eq!(
        preference.fallback_policy,
        BackendFallbackPolicy::AutoDegrade
    );
}

#[tokio::test]

async fn system_builtins_register_an_executable_handler() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let actions = runtime.actions.read().await;

    let missing = actions
        .values()
        .filter(|loaded| loaded.info.source == ActionSource::System)
        .filter(|loaded| loaded.builtin_handler.is_none())
        .map(|loaded| loaded.info.name.clone())
        .collect::<Vec<_>>();

    assert!(
        missing.is_empty(),
        "system built-ins missing handlers: {:?}",
        missing
    );
}

#[tokio::test]

async fn background_support_is_schema_declared() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    runtime.load_builtin_actions().await.unwrap();

    let actions = runtime.actions.read().await;

    for loaded in actions.values() {
        let schema_declares_background = loaded
            .info
            .input_schema
            .pointer("/properties/background")
            .is_some();

        assert_eq!(
            loaded.supports_background, schema_declares_background,
            "background support drifted for {}",
            loaded.info.name
        );
    }
}

#[test]

fn parse_delegate_completion_accepts_structured_marker() {
    let structured = format!(
        "{}{}",
        TOOL_COMPLETION_MARKER,
        serde_json::json!({

            "tool": "delegate",

            "status": "completed",

            "detail": "Delegation prepared"

        })
    );

    let structured =
        parse_delegate_completion(&structured).expect("structured delegate marker should parse");

    assert_eq!(structured.tool, "delegate");

    assert_eq!(structured.status, "completed");
}

#[test]

fn parse_tool_completion_accepts_marker_line_before_human_text() {
    let output = format!(
        "{}{}\nHuman readable status follows.",
        TOOL_COMPLETION_MARKER,
        serde_json::json!({

            "tool": "watch",

            "status": "completed",

            "detail": "Polling configured"

        })
    );

    let structured = parse_watch_completion(&output)
        .expect("structured marker line should parse before human text");

    assert_eq!(structured.tool, "watch");

    assert_eq!(structured.status, "completed");
}

#[test]

fn parse_tool_completion_accepts_raw_json_envelope() {
    let structured = parse_schedule_task_completion(
        &serde_json::json!({

            "tool": "schedule_task",

            "status": "completed",

            "detail": "Task scheduled"

        })
        .to_string(),
    )
    .expect("raw JSON completion should parse");

    assert_eq!(structured.tool, "schedule_task");

    assert_eq!(structured.status, "completed");
}

#[test]

fn parse_workflow_inputs_accepts_structured_json_without_marker() {
    let payload = serde_json::json!({

        "action": "lookup_customer",

        "missing": ["customer_id"],

        "required": ["customer_id"],

        "provided": [],

        "query": "lookup customer"

    })
    .to_string();

    let parsed = parse_workflow_missing_inputs_marker(&payload)
        .expect("raw JSON missing-input payload should parse");

    assert_eq!(parsed.action, "lookup_customer");

    assert_eq!(parsed.missing, vec!["customer_id".to_string()]);
}

#[test]

fn required_input_parsing_accepts_canonicalized_metadata_keys_and_headings() {
    let frontmatter = "Required Fields:\n  - customer_id\n  - account_id";

    assert_eq!(
        ActionRuntime::parse_required_fields_from_frontmatter(frontmatter),
        vec!["customer_id".to_string(), "account_id".to_string()]
    );

    let workflow = "## Inputs Required\n- `customer_id`: stable customer id\n- account_id";

    assert_eq!(
        ActionRuntime::parse_required_fields_from_workflow(workflow),
        vec!["customer_id".to_string(), "account_id".to_string()]
    );
}

#[cfg(feature = "docker")]
#[test]

fn docker_host_socket_transport_detection_handles_unix_and_tcp_hosts() {
    assert!(ActionRuntime::docker_host_uses_socket_transport(
        "unix:///var/run/docker.sock"
    ));

    assert!(ActionRuntime::docker_host_uses_socket_transport(
        "npipe:////./pipe/docker_engine"
    ));

    assert!(!ActionRuntime::docker_host_uses_socket_transport(
        "tcp://127.0.0.1:2375"
    ));

    assert!(!ActionRuntime::docker_host_uses_socket_transport(
        "http://docker.internal:2375"
    ));
}

#[test]

fn control_plane_without_local_docker_skips_local_docker_management() {
    assert!(!ActionRuntime::should_manage_local_sandbox_containers_for(
        Some("control"),
        false,
    ));

    assert!(!ActionRuntime::should_manage_local_sandbox_containers_for(
        Some("control-plane"),
        false,
    ));

    assert!(ActionRuntime::should_manage_local_sandbox_containers_for(
        Some("control"),
        true,
    ));

    assert!(ActionRuntime::should_manage_local_sandbox_containers_for(
        Some("executor"),
        false,
    ));

    assert!(ActionRuntime::should_manage_local_sandbox_containers_for(
        None, false,
    ));
}

#[test]

fn code_execute_execution_metadata_marks_bootstrap_setup_only() {
    let metadata = ActionRuntime::build_code_execute_execution_metadata(
        &serde_json::json!({

            "network_access": false,

            "execution_contract": {

                "phase": "bootstrap"

            }

        }),
        true,
        0,
    );

    assert_eq!(
        metadata.get("phase").and_then(|value| value.as_str()),
        Some("bootstrap")
    );

    assert_eq!(
        metadata.get("setup_only").and_then(|value| value.as_bool()),
        Some(true)
    );

    assert_eq!(
        metadata
            .get("ready_for_watch")
            .and_then(|value| value.as_bool()),
        Some(false)
    );
}

#[test]

fn upload_signature_detects_opus_by_bytes_without_extension() {
    let mut bytes = b"OggS".to_vec();

    bytes.extend_from_slice(&[0; 24]);

    bytes.extend_from_slice(b"OpusHead");

    let detected = ActionRuntime::upload_signature("voice", None, &bytes);

    assert_eq!(
        detected.get("input_type").and_then(|value| value.as_str()),
        Some("audio")
    );

    assert_eq!(
        detected.get("extension").and_then(|value| value.as_str()),
        Some("opus")
    );
}

#[test]

fn upload_signature_keeps_unknown_unresolved_instead_of_guessing_text() {
    let detected = ActionRuntime::upload_signature(
        "payload",
        Some("application/octet-stream"),
        b"plain utf8 but no durable type evidence",
    );

    assert_eq!(
        detected.get("input_type").and_then(|value| value.as_str()),
        Some("unknown")
    );

    assert_eq!(
        detected
            .get("needs_deeper_inspection")
            .and_then(|value| value.as_bool()),
        Some(true)
    );

    assert!(detected.get("mime").is_some_and(|value| value.is_null()));
}

#[test]

fn missing_binary_detector_reads_structured_marker() {
    assert_eq!(
        ActionRuntime::detect_missing_binary_from_output("AGENTARK_MISSING_BINARY: ffmpeg\n"),
        Some("ffmpeg".to_string())
    );
}

#[test]

fn missing_binary_detector_extracts_generic_shell_errors() {
    assert_eq!(
        ActionRuntime::detect_missing_binary_from_output("bash: custom-tool: command not found"),
        Some("custom-tool".to_string())
    );

    assert_eq!(
        ActionRuntime::detect_missing_binary_from_output(
            "FileNotFoundError: [Errno 2] No such file or directory: 'media-helper'"
        ),
        Some("media-helper".to_string())
    );
}

#[test]

fn code_execute_execution_metadata_marks_validated_poller_ready_for_watch() {
    let metadata = ActionRuntime::build_code_execute_execution_metadata(
        &serde_json::json!({

            "network_access": true,

            "execution_contract": {

                "phase": "validate",

                "target_validated_when_successful": true,

                "ready_for_watch_when_successful": true

            }

        }),
        true,
        1,
    );

    assert_eq!(
        metadata.get("phase").and_then(|value| value.as_str()),
        Some("validate")
    );

    assert_eq!(
        metadata
            .get("target_validated")
            .and_then(|value| value.as_bool()),
        Some(true)
    );

    assert_eq!(
        metadata
            .get("ready_for_watch")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
}

#[test]

fn target_connectivity_contract_enables_effective_network_access() {
    assert!(ActionRuntime::code_execute_effective_network_access(
        &serde_json::json!({

            "execution_contract": {

                "phase": "validate",

                "target_connectivity_required": true

            }

        })
    ));

    assert!(!ActionRuntime::code_execute_effective_network_access(
        &serde_json::json!({

            "execution_contract": {

                "phase": "bootstrap"

            }

        })
    ));
}

#[test]

fn code_execute_infers_network_access_from_endpoint_values() {
    assert!(ActionRuntime::code_execute_effective_network_access(
        &serde_json::json!({

            "language": "python",

            "code": "print('polling a device')",

            "env": {

                "TARGET_URL": "customproto://192.168.29.61:554/stream"

            }

        })
    ));

    assert!(ActionRuntime::code_execute_effective_network_access(
        &serde_json::json!({

            "language": "python",

            "code": "open('output.txt', 'w').write('http://example.com')"

        })
    ));

    assert!(!ActionRuntime::code_execute_effective_network_access(
        &serde_json::json!({

            "language": "python",

            "code": "print('local-only calculation')"

        })
    ));
}
