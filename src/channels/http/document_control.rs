use super::*;

// ==================== Document Endpoints ====================

pub(super) async fn list_documents_endpoint(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let project_id = params.get("project_id").map(|s| s.as_str());
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20u64);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let agent = state.agent.read().await;
    let total = agent.storage.count_documents(project_id).await.unwrap_or(0);
    match agent
        .storage
        .list_documents(limit, offset, project_id)
        .await
    {
        Ok(docs) => {
            let list: Vec<serde_json::Value> = docs
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "id": d.id, "filename": d.filename, "content_type": d.content_type,
                        "project_id": d.project_id, "chunk_count": d.chunk_count,
                        "file_size": d.file_size, "created_at": d.created_at,
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"documents": list, "total": total, "limit": limit, "offset": offset}))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn delete_document_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.delete_document(&id).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) fn sanitize_document_filename(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('_').trim_matches('.').to_string();
    if trimmed.is_empty() {
        "document.txt".to_string()
    } else {
        trimmed
    }
}

pub(super) fn decode_xml_entities(input: &str) -> String {
    input
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

pub(super) fn extract_docx_text(bytes: &[u8]) -> Result<String, String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("Invalid DOCX archive: {}", e))?;
    let mut doc_xml = archive
        .by_name("word/document.xml")
        .map_err(|_| "DOCX is missing word/document.xml".to_string())?;
    let mut xml = String::new();
    doc_xml
        .read_to_string(&mut xml)
        .map_err(|e| format!("Failed to read DOCX XML: {}", e))?;

    let normalized = xml
        .replace("<w:tab/>", "\t")
        .replace("<w:br/>", "\n")
        .replace("<w:cr/>", "\n")
        .replace("</w:p>", "\n")
        .replace("</w:tr>", "\n")
        .replace("</w:tc>", "\t");
    let without_tags = regex::Regex::new(r"<[^>]+>")
        .map_err(|e| format!("Regex error while parsing DOCX: {}", e))?
        .replace_all(&normalized, "");
    Ok(decode_xml_entities(&without_tags).trim().to_string())
}

pub(super) fn extract_document_text(
    filename: &str,
    content_type: &str,
    bytes: &[u8],
) -> Result<String, String> {
    let lower_name = filename.to_ascii_lowercase();
    let ext = lower_name.rsplit('.').next().unwrap_or("");
    let lower_ct = content_type.to_ascii_lowercase();

    let looks_pdf = ext == "pdf" || lower_ct == "application/pdf";
    if looks_pdf {
        return pdf_extract::extract_text_from_mem(bytes)
            .map(|s| s.trim().to_string())
            .map_err(|e| format!("Failed to parse PDF: {}", e));
    }

    let looks_docx = ext == "docx"
        || lower_ct
            .contains("application/vnd.openxmlformats-officedocument.wordprocessingml.document");
    if looks_docx {
        return extract_docx_text(bytes);
    }

    if ext == "doc" {
        return Err(
            "Legacy .doc files are not supported yet. Please save as .docx or .txt.".to_string(),
        );
    }

    let text_exts = [
        "txt", "md", "markdown", "json", "csv", "tsv", "xml", "html", "htm", "yaml", "yml", "log",
        "ini", "toml", "sql", "js", "ts", "tsx", "jsx", "py", "rs", "go", "java", "c", "cpp", "h",
        "hpp", "sh", "bat", "ps1",
    ];
    let likely_text = lower_ct.starts_with("text/")
        || lower_ct.contains("json")
        || lower_ct.contains("xml")
        || lower_ct.contains("yaml")
        || text_exts.contains(&ext);
    if likely_text {
        return String::from_utf8(bytes.to_vec())
            .or_else(|_| Ok(String::from_utf8_lossy(bytes).to_string()))
            .map(|s| s.trim().to_string());
    }

    Err(format!(
        "Unsupported file type '{}'. Supported: txt/md/json/csv/xml/yaml, PDF, DOCX.",
        content_type
    ))
}

pub(super) async fn insert_document_from_text(
    agent: &Agent,
    filename: String,
    content_type: String,
    project_id: Option<String>,
    content: String,
) -> Result<(String, usize), String> {
    // Chunk the content (simple fixed-size chunking)
    let chunk_size = 1000; // chars per chunk
    let chunks: Vec<String> = content
        .chars()
        .collect::<Vec<_>>()
        .chunks(chunk_size)
        .map(|c| c.iter().collect())
        .collect();

    let doc_id = uuid::Uuid::new_v4().to_string();
    let doc = crate::storage::entities::document::Model {
        id: doc_id.clone(),
        filename: filename.clone(),
        content_type,
        project_id,
        chunk_count: chunks.len() as i32,
        file_size: content.len().min(i64::MAX as usize) as i64,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    let mut chunk_rows: Vec<crate::storage::entities::document_chunk::Model> = chunks
        .iter()
        .enumerate()
        .map(
            |(i, chunk_content)| crate::storage::entities::document_chunk::Model {
                id: uuid::Uuid::new_v4().to_string(),
                document_id: doc_id.clone(),
                chunk_index: i as i32,
                content: chunk_content.clone(),
                embedding: None,
            },
        )
        .collect();
    if let Err(e) = crate::core::document_search::embed_document_chunks(
        agent.embedding_client.as_deref(),
        &filename,
        &doc.content_type,
        doc.project_id.as_deref(),
        &mut chunk_rows,
    )
    .await
    {
        tracing::warn!("Document embedding failed for {}: {}", filename, e);
    }

    agent
        .storage
        .insert_document_with_chunks(&doc, &chunk_rows)
        .await
        .map_err(|e| e.to_string())?;

    // Emit notification
    agent
        .emit_notification(
            &format!("Document uploaded: {}", filename),
            &format!("{} chunks indexed", chunks.len()),
            "info",
            "documents",
        )
        .await;

    Ok((doc_id, chunks.len()))
}

/// Upload a document (JSON body with already-extracted text content)
pub(super) async fn upload_document_endpoint(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let filename = match request.get("filename").and_then(|v| v.as_str()) {
        Some(f) => sanitize_document_filename(f),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "filename required".to_string(),
                }),
            )
                .into_response();
        }
    };
    let content = match request.get("content").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "content required".to_string(),
                }),
            )
                .into_response();
        }
    };
    if content.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "content is empty after parsing".to_string(),
            }),
        )
            .into_response();
    }
    let project_id = request
        .get("project_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let content_type = request
        .get("content_type")
        .and_then(|v| v.as_str())
        .unwrap_or("text/plain")
        .to_string();

    let agent = state.agent.read().await;
    match insert_document_from_text(&agent, filename.clone(), content_type, project_id, content)
        .await
    {
        Ok((doc_id, chunks)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": doc_id,
                "filename": filename,
                "chunks": chunks,
                "status": "ok"
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: e }),
        )
            .into_response(),
    }
}

/// Upload a binary/text document using multipart form-data and extract text server-side.
/// Expected fields:
/// - file (required)
/// - project_id (optional)
/// - filename (optional override)
/// - content_type (optional override)
pub(super) async fn upload_document_file_endpoint(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Response {
    let mut filename_override: Option<String> = None;
    let mut content_type_override: Option<String> = None;
    let mut project_id: Option<String> = None;
    let mut uploaded_filename: Option<String> = None;
    let mut uploaded_content_type = "application/octet-stream".to_string();
    let mut uploaded_bytes: Option<Vec<u8>> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "project_id" => match field.text().await {
                Ok(v) => {
                    let trimmed = v.trim();
                    if !trimmed.is_empty() {
                        project_id = Some(trimmed.to_string());
                    }
                }
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!("Invalid project_id field: {}", e),
                        }),
                    )
                        .into_response();
                }
            },
            "filename" => match field.text().await {
                Ok(v) => {
                    let trimmed = v.trim();
                    if !trimmed.is_empty() {
                        filename_override = Some(trimmed.to_string());
                    }
                }
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!("Invalid filename override field: {}", e),
                        }),
                    )
                        .into_response();
                }
            },
            "content_type" => match field.text().await {
                Ok(v) => {
                    let trimmed = v.trim();
                    if !trimmed.is_empty() {
                        content_type_override = Some(trimmed.to_string());
                    }
                }
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!("Invalid content_type override field: {}", e),
                        }),
                    )
                        .into_response();
                }
            },
            _ => {
                // Treat first non-metadata field as uploaded file payload.
                if uploaded_bytes.is_some() {
                    continue;
                }
                uploaded_filename = field.file_name().map(|s| s.to_string());
                if let Some(ct) = field.content_type() {
                    uploaded_content_type = ct.to_string();
                }
                match field.bytes().await {
                    Ok(bytes) => {
                        if bytes.len() > 50 * 1024 * 1024 {
                            return (
                                StatusCode::PAYLOAD_TOO_LARGE,
                                Json(ErrorResponse {
                                    error: "File too large (50MB max)".to_string(),
                                }),
                            )
                                .into_response();
                        }
                        uploaded_bytes = Some(bytes.to_vec());
                    }
                    Err(e) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: format!("Failed to read uploaded file: {}", e),
                            }),
                        )
                            .into_response();
                    }
                }
            }
        }
    }

    let bytes = match uploaded_bytes {
        Some(b) => b,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "No file uploaded. Expected multipart field 'file'.".to_string(),
                }),
            )
                .into_response();
        }
    };

    let raw_filename = filename_override
        .or(uploaded_filename)
        .unwrap_or("document.txt".to_string());
    let filename = sanitize_document_filename(&raw_filename);
    let content_type = content_type_override.unwrap_or(uploaded_content_type);
    let extracted = match extract_document_text(&filename, &content_type, &bytes) {
        Ok(text) if !text.trim().is_empty() => text,
        Ok(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Parsed document content is empty".to_string(),
                }),
            )
                .into_response();
        }
        Err(e) => {
            return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response();
        }
    };

    let agent = state.agent.read().await;
    match insert_document_from_text(
        &agent,
        filename.clone(),
        content_type.clone(),
        project_id,
        extracted,
    )
    .await
    {
        Ok((doc_id, chunks)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": doc_id,
                "filename": filename,
                "content_type": content_type,
                "chunks": chunks,
                "status": "ok"
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: e }),
        )
            .into_response(),
    }
}

/// Search within a specific document
pub(super) async fn search_document_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let query = match params.get("q") {
        Some(q) => q.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "query parameter 'q' required".to_string(),
                }),
            )
                .into_response();
        }
    };
    let agent = state.agent.read().await;
    match agent.storage.get_document_chunks(&id).await {
        Ok(chunks) => {
            let query_lower = query.to_lowercase();
            let mut results: Vec<serde_json::Value> = chunks
                .into_iter()
                .filter(|c| c.content.to_lowercase().contains(&query_lower))
                .map(|c| {
                    serde_json::json!({
                        "chunk_index": c.chunk_index,
                        "content": c.content,
                    })
                })
                .collect();
            results.truncate(10);
            (
                StatusCode::OK,
                Json(serde_json::json!({"results": results})),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}
