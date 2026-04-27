use super::*;

/// Serve built frontend assets from `frontend/dist/assets/*`.
pub(super) async fn serve_frontend_asset(Path(path): Path<String>) -> Response {
    if !is_safe_asset_path(&path) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let rel = PathBuf::from("assets").join(&path);
    for base in frontend_dist_roots() {
        let file_path = base.join(&rel);
        let is_file = tokio::fs::metadata(&file_path)
            .await
            .map(|meta| meta.is_file())
            .unwrap_or(false);
        if is_file {
            if let Ok(bytes) = tokio::fs::read(&file_path).await {
                return (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, mime_for_asset(&path)),
                        (header::CACHE_CONTROL, CACHE_CONTROL_FRONTEND_ASSET),
                    ],
                    bytes,
                )
                    .into_response();
            }
        }
    }
    StatusCode::NOT_FOUND.into_response()
}

pub(super) fn frontend_dist_roots() -> Vec<PathBuf> {
    vec![
        PathBuf::from(FRONTEND_DIST_DIR),
        PathBuf::from("./frontend/dist"),
        PathBuf::from("/app/frontend/dist"),
    ]
}

pub(super) async fn read_frontend_index_html() -> Option<String> {
    for root in frontend_dist_roots() {
        let index = root.join("index.html");
        let is_file = tokio::fs::metadata(&index)
            .await
            .map(|meta| meta.is_file())
            .unwrap_or(false);
        if is_file {
            if let Ok(html) = tokio::fs::read_to_string(index).await {
                return Some(html);
            }
        }
    }
    None
}

pub(super) fn is_safe_asset_path(path: &str) -> bool {
    if path.is_empty() || path.contains('\\') {
        return false;
    }
    let clean = FsPath::new(path);
    clean
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
}

pub(super) fn mime_for_asset(path: &str) -> &'static str {
    if path.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else if path.ends_with(".woff") {
        "font/woff"
    } else {
        "application/octet-stream"
    }
}

/// Serve PNG logo
pub(super) async fn serve_logo_png() -> Response {
    // Try to include PNG at compile time, return 404 if not available
    {
        // Try to read from filesystem at runtime as fallback
        if let Ok(bytes) = tokio::fs::read("assets/logo.png").await {
            return ([(header::CONTENT_TYPE, "image/png")], bytes).into_response();
        }
        // Check common paths
        for path in &[
            "/app/assets/logo.png",
            "./assets/logo.png",
            "../assets/logo.png",
        ] {
            if let Ok(bytes) = tokio::fs::read(path).await {
                return ([(header::CONTENT_TYPE, "image/png")], bytes).into_response();
            }
        }
        StatusCode::NOT_FOUND.into_response()
    }
}

/// Serve JPG logo
pub(super) async fn serve_logo_jpg() -> Response {
    // Try to read from filesystem at runtime
    if let Ok(bytes) = tokio::fs::read("assets/logo.jpg").await {
        return ([(header::CONTENT_TYPE, "image/jpeg")], bytes).into_response();
    }
    // Check common paths
    for path in &[
        "/app/assets/logo.jpg",
        "./assets/logo.jpg",
        "../assets/logo.jpg",
    ] {
        if let Ok(bytes) = tokio::fs::read(path).await {
            return ([(header::CONTENT_TYPE, "image/jpeg")], bytes).into_response();
        }
    }
    StatusCode::NOT_FOUND.into_response()
}

/// Serve favicon PNG
pub(super) async fn serve_favicon_png() -> Response {
    for path in &[
        "assets/favicon.png",
        "/app/assets/favicon.png",
        "./assets/favicon.png",
        "../assets/favicon.png",
    ] {
        if let Ok(bytes) = tokio::fs::read(path).await {
            return ([(header::CONTENT_TYPE, "image/png")], bytes).into_response();
        }
    }
    serve_logo_png().await
}

/// Serve SVG logo (animated)
pub(super) async fn serve_logo_svg() -> Response {
    // Try to read from filesystem at runtime
    if let Ok(bytes) = tokio::fs::read("assets/logo.svg").await {
        return ([(header::CONTENT_TYPE, "image/svg+xml")], bytes).into_response();
    }
    // Check common paths
    for path in &[
        "/app/assets/logo.svg",
        "./assets/logo.svg",
        "../assets/logo.svg",
    ] {
        if let Ok(bytes) = tokio::fs::read(path).await {
            return ([(header::CONTENT_TYPE, "image/svg+xml")], bytes).into_response();
        }
    }
    StatusCode::NOT_FOUND.into_response()
}

/// Serve output files from code execution (images, CSVs, code files, etc.)
pub(super) async fn serve_output_file(
    State(state): State<AppState>,
    Path((exec_id, filename)): Path<(String, String)>,
) -> Response {
    // Validate exec_id is a UUID to prevent path traversal
    if uuid::Uuid::parse_str(&exec_id).is_err() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    // Validate filename has no path separators
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    // Resolve data directory from agent config
    let data_dir = {
        let agent = state.agent.read().await;
        agent.data_dir().to_path_buf()
    };
    let file_path = data_dir.join("outputs").join(&exec_id).join(&filename);
    let bytes = match tokio::fs::read(&file_path).await {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            if let Some(workspace) = state.workspace_client.as_ref() {
                workspace
                    .get_blob(&format!("outputs/{}/{}", exec_id, filename))
                    .await
                    .ok()
            } else {
                None
            }
        }
    };

    match bytes {
        Some(bytes) => {
            let content_type = guess_content_type(&filename);
            let safe_filename = sanitize_content_disposition_filename(&filename);
            (
                [
                    (header::CONTENT_TYPE, content_type.as_str()),
                    (
                        header::CONTENT_DISPOSITION,
                        &format!("inline; filename=\"{}\"", safe_filename),
                    ),
                    (header::CACHE_CONTROL, "public, max-age=86400"),
                ],
                bytes,
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Serve output file as a download (Content-Disposition: attachment)
pub(super) async fn download_output_file(
    State(state): State<AppState>,
    Path((exec_id, filename)): Path<(String, String)>,
) -> Response {
    if uuid::Uuid::parse_str(&exec_id).is_err() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let data_dir = {
        let agent = state.agent.read().await;
        agent.data_dir().to_path_buf()
    };
    let file_path = data_dir.join("outputs").join(&exec_id).join(&filename);
    let bytes = match tokio::fs::read(&file_path).await {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            if let Some(workspace) = state.workspace_client.as_ref() {
                workspace
                    .get_blob(&format!("outputs/{}/{}", exec_id, filename))
                    .await
                    .ok()
            } else {
                None
            }
        }
    };

    match bytes {
        Some(bytes) => {
            let content_type = guess_content_type(&filename);
            let safe_filename = sanitize_content_disposition_filename(&filename);
            (
                [
                    (header::CONTENT_TYPE, content_type.as_str()),
                    (
                        header::CONTENT_DISPOSITION,
                        &format!("attachment; filename=\"{}\"", safe_filename),
                    ),
                    (header::CACHE_CONTROL, "public, max-age=86400"),
                ],
                bytes,
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Guess MIME content type from filename extension.
/// Falls back to octet-stream for unknown types.
pub(super) fn guess_content_type(filename: &str) -> String {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        // Images
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        // Documents
        "pdf" => "application/pdf",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        // Data
        "json" | "ipynb" => "application/json",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "yaml" | "yml" => "text/yaml",
        "toml" => "text/toml",
        // Code (all served as plain text for viewing)
        "txt" | "log" | "md" | "rst" => "text/plain",
        "py" | "js" | "ts" | "java" | "c" | "cpp" | "h" | "hpp" | "rs" | "go" | "rb" | "php"
        | "pl" | "lua" | "r" | "sh" | "bash" | "zsh" | "fish" | "kt" | "swift" | "sql" | "css"
        | "scss" | "less" => "text/plain; charset=utf-8",
        // Archives
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        // Audio/Video
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        // Office
        "xlsx" | "xls" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "docx" | "doc" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "pptx" | "ppt" => {
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        }
        // Fallback
        _ => "application/octet-stream",
    }
    .to_string()
}

// ==================== Deployed Apps ====================

pub(super) fn merge_executor_status_fields(
    row: &mut serde_json::Map<String, serde_json::Value>,
    status: &crate::clients::AppStatusResponse,
) {
    row.insert("running".to_string(), serde_json::json!(status.running));
    row.insert(
        "runtime_mode".to_string(),
        serde_json::Value::String(
            status
                .runtime_mode
                .clone()
                .unwrap_or_else(|| "stopped".to_string()),
        ),
    );
    row.insert(
        "is_isolated_runtime".to_string(),
        serde_json::json!(status.is_isolated_runtime),
    );
    row.insert(
        "port".to_string(),
        status
            .port
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null),
    );
}

pub(super) fn executor_websocket_base_url(base_url: &str) -> String {
    if let Some(stripped) = base_url.strip_prefix("https://") {
        format!("wss://{}", stripped.trim_end_matches('/'))
    } else if let Some(stripped) = base_url.strip_prefix("http://") {
        format!("ws://{}", stripped.trim_end_matches('/'))
    } else {
        format!("ws://{}", base_url.trim_end_matches('/'))
    }
}

/// List all deployed apps
pub(super) async fn list_apps(State(state): State<AppState>) -> Json<serde_json::Value> {
    let mut apps = Vec::new();
    for mut row in state.app_registry.list().await {
        let Some(obj) = row.as_object_mut() else {
            apps.push(row);
            continue;
        };
        let Some(app_id) = obj
            .get("id")
            .and_then(|value| value.as_str())
            .map(|s| s.to_string())
        else {
            apps.push(row);
            continue;
        };
        let access_guard_enabled = obj
            .get("access_guard_enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        obj.insert(
            "url".to_string(),
            serde_json::Value::String(app_root_url_for_state(&state, &app_id)),
        );
        obj.insert(
            "access_url".to_string(),
            serde_json::Value::String(
                app_access_url_for_state(&state, &app_id, access_guard_enabled).await,
            ),
        );
        if !obj
            .get("is_static")
            .and_then(|value| value.as_bool())
            .unwrap_or(true)
        {
            if let Some(executor) = state.executor_client.as_ref() {
                if let Ok(status) = executor.app_status(&app_id).await {
                    merge_executor_status_fields(obj, &status);
                }
            }
        }
        apps.push(row);
    }
    let restore = state.app_registry.restore_snapshot().await;
    Json(serde_json::json!({ "apps": apps, "restore": restore }))
}

pub(super) fn is_valid_app_id(app_id: &str) -> bool {
    !app_id.is_empty()
        && app_id.len() <= 64
        && app_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub(super) fn is_secure_origin_request(headers: &axum::http::HeaderMap) -> bool {
    if headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("https"))
    {
        return true;
    }
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    host.ends_with(".trycloudflare.com") || host.ends_with(".cfargotunnel.com")
}

pub(super) fn should_upgrade_insecure_links(content_type: &str) -> bool {
    let ct = content_type.to_ascii_lowercase();
    ct.starts_with("text/html")
        || ct.starts_with("application/javascript")
        || ct.starts_with("text/javascript")
        || ct.starts_with("text/css")
}

pub(super) fn app_scoped_public_fetch_prefix(app_id: &str) -> String {
    format!(
        "/apps/{}/__agentark/http/fetch?url=",
        urlencoding::encode(app_id)
    )
}

pub(super) fn rewrite_external_proxy_urls_for_public_apps(content: &str, app_id: &str) -> String {
    let proxy_prefix = app_scoped_public_fetch_prefix(app_id);
    let nested_prefix = format!("{}{}", proxy_prefix, proxy_prefix);

    let mut rewritten = content
        .replace("/public/proxy/raw?url=", &proxy_prefix)
        .replace("https://api.allorigins.win/raw?url=", &proxy_prefix)
        .replace("http://api.allorigins.win/raw?url=", &proxy_prefix)
        .replace("https://corsproxy.io/?", &proxy_prefix)
        .replace("http://corsproxy.io/?", &proxy_prefix)
        .replace("https://api.codetabs.com/v1/proxy/?quest=", &proxy_prefix)
        .replace("http://api.codetabs.com/v1/proxy/?quest=", &proxy_prefix);

    while rewritten.contains(&nested_prefix) {
        rewritten = rewritten.replace(&nested_prefix, &proxy_prefix);
    }

    rewritten
}

pub(super) fn inject_app_runtime_fetch_shims(content: &str, app_id: &str) -> String {
    if content.contains("__agentarkLlmProxyShimApplied") {
        return content.to_string();
    }
    let shim = format!(
        r#"<script>
(function() {{
  if (window.__agentarkLlmProxyShimApplied) return;
  window.__agentarkLlmProxyShimApplied = true;
  const APP_ID = "{app_id}";
  const PROXY_PATH = "/apps/" + encodeURIComponent(APP_ID) + "/__agentark/llm/chat";
  const PUBLIC_FETCH_PATH = "/apps/" + encodeURIComponent(APP_ID) + "/__agentark/http/fetch";

  const nativeFetch = window.fetch ? window.fetch.bind(window) : null;
  if (nativeFetch) {{
    const extractUrl = (input) => {{
      try {{
        if (typeof input === "string") return input;
        if (input && typeof input.url === "string") return input.url;
        if (input instanceof URL) return input.toString();
      }} catch (_) {{}}
      return "";
    }};
    const toAbsoluteUrl = (input) => {{
      try {{
        const candidate = extractUrl(input);
        return candidate ? new URL(candidate, window.location.href).toString() : "";
      }} catch (_) {{
        return extractUrl(input);
      }}
    }};
    const shouldProxyLlm = (url) => {{
      const lower = String(url || "").toLowerCase();
      return (
        lower.includes("openrouter.ai/api/v1/chat/completions") ||
        lower.includes("openrouter.ai/api/v1/responses") ||
        lower.includes("openrouter.ai/api/v1/completions") ||
        lower.includes("api.openai.com/v1/chat/completions") ||
        lower.includes("api.openai.com/v1/responses") ||
        lower.includes("api.openai.com/v1/completions") ||
        lower.endsWith("/v1/chat/completions") ||
        lower.endsWith("/v1/responses") ||
        lower.endsWith("/v1/completions")
      );
    }};
    const shouldProxyPublicRead = (url, method) => {{
      const inferredMethod = String(method || "GET").toUpperCase();
      if (inferredMethod !== "GET" && inferredMethod !== "HEAD") {{
        return false;
      }}
      try {{
        const absolute = new URL(String(url || ""), window.location.href);
        if ((absolute.protocol !== "http:" && absolute.protocol !== "https:") || absolute.origin === window.location.origin) {{
          return false;
        }}
        const lowerPath = absolute.pathname.toLowerCase();
        return (
          !lowerPath.includes("/__agentark/http/fetch") &&
          !lowerPath.includes("/__agentark/llm/chat") &&
          !lowerPath.includes("/__agentark/arxiv/search")
        );
      }} catch (_) {{
        return false;
      }}
    }};
    const buildPublicFetchHeaders = (source) => {{
      const headers = new Headers();
      const sourceHeaders = new Headers(source || {{}});
      ["accept", "accept-language", "if-none-match", "if-modified-since", "range"].forEach((name) => {{
        const value = sourceHeaders.get(name);
        if (value) {{
          headers.set(name, value);
        }}
      }});
      headers.set("x-agentark-app-proxy", "raw");
      return headers;
    }};
    window.fetch = function(input, init) {{
      const targetUrl = extractUrl(input);
      const absoluteTargetUrl = toAbsoluteUrl(input);
      const baseInit = Object.assign({{}}, init || {{}});
      const inferredMethod = (
        baseInit.method ||
        (input && input.method) ||
        "GET"
      )
        .toString()
        .toUpperCase();
      if (shouldProxyPublicRead(absoluteTargetUrl || targetUrl, inferredMethod)) {{
        const proxyReadInit = {{
          method: inferredMethod,
          headers: buildPublicFetchHeaders(baseInit.headers || (input && input.headers)),
          credentials: "same-origin"
        }};
        if (baseInit.signal) {{
          proxyReadInit.signal = baseInit.signal;
        }}
        return nativeFetch(
          PUBLIC_FETCH_PATH + "?url=" + encodeURIComponent(absoluteTargetUrl || targetUrl),
          proxyReadInit
        );
      }}
      if (!shouldProxyLlm(targetUrl)) {{
        return nativeFetch(input, init);
      }}
      const proxyInit = baseInit;
      proxyInit.method = inferredMethod;
      if (inferredMethod !== "POST") {{
        return nativeFetch(input, init);
      }}
      const headers = new Headers(proxyInit.headers || (input && input.headers) || {{}});
      headers.delete("authorization");
      headers.delete("x-api-key");
      headers.set("content-type", "application/json");
      headers.set("x-agentark-app-proxy", "llm");
      proxyInit.headers = headers;
      return nativeFetch(PROXY_PATH, proxyInit);
    }};
  }}

  if (window.XMLHttpRequest && window.XMLHttpRequest.prototype) {{
    const xhrProto = window.XMLHttpRequest.prototype;
    const nativeXhrOpen = xhrProto.open;
    const nativeXhrSend = xhrProto.send;
    xhrProto.open = function(method, url) {{
      const args = Array.prototype.slice.call(arguments);
      const targetUrl = toAbsoluteUrl(url);
      if (shouldProxyPublicRead(targetUrl, method)) {{
        args[1] = PUBLIC_FETCH_PATH + "?url=" + encodeURIComponent(targetUrl);
        this.__agentarkPublicFetchProxy = true;
      }} else {{
        this.__agentarkPublicFetchProxy = false;
      }}
      return nativeXhrOpen.apply(this, args);
    }};
    xhrProto.send = function(body) {{
      if (this.__agentarkPublicFetchProxy) {{
        try {{
          this.setRequestHeader("x-agentark-app-proxy", "raw");
        }} catch (_) {{}}
      }}
      return nativeXhrSend.call(this, body);
    }};
  }}

  const nativePrompt = window.prompt ? window.prompt.bind(window) : null;
  if (nativePrompt) {{
    window.prompt = function(message, defaultValue) {{
      const text = String(message || "").toLowerCase();
      if (
        text.includes("api key") ||
        text.includes("openai") ||
        text.includes("openrouter") ||
        text.includes("anthropic")
      ) {{
        return "agentark-managed";
      }}
      return nativePrompt(message, defaultValue);
    }};
  }}
}})();
</script>"#
    );

    if content.contains("</head>") {
        return content.replacen("</head>", &format!("{}\n</head>", shim), 1);
    }
    if content.contains("</body>") {
        return content.replacen("</body>", &format!("{}\n</body>", shim), 1);
    }
    format!("{}\n{}", content, shim)
}

pub(super) fn extract_openai_message_text(value: &serde_json::Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Some(obj) = value.as_object() {
        if let Some(s) = obj.get("text").and_then(|v| v.as_str()) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        if let Some(s) = obj.get("content").and_then(|v| v.as_str()) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    if let Some(arr) = value.as_array() {
        let mut chunks = Vec::new();
        for item in arr {
            if let Some(obj) = item.as_object() {
                let item_type = obj
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("text")
                    .to_ascii_lowercase();
                if item_type != "text" && item_type != "input_text" && item_type != "output_text" {
                    continue;
                }
                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                    if !text.trim().is_empty() {
                        chunks.push(text.trim().to_string());
                    }
                }
            } else if let Some(s) = item.as_str() {
                if !s.trim().is_empty() {
                    chunks.push(s.trim().to_string());
                }
            }
        }
        if !chunks.is_empty() {
            return Some(chunks.join("\n"));
        }
    }
    None
}

#[derive(Debug, Clone, Serialize, Default)]
pub(super) struct AppArxivPaper {
    id: String,
    paper_url: String,
    pdf_url: String,
    title: String,
    summary: String,
    published: String,
    updated: String,
    authors: Vec<String>,
    primary_category: String,
    categories: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct AppArxivSearchRequest {
    pub(super) source_url: String,
    pub(super) search_query: String,
    pub(super) start: usize,
    pub(super) max_results: usize,
    pub(super) sort_by: String,
    pub(super) sort_order: String,
    pub(super) categories: Vec<String>,
    pub(super) keywords: Vec<String>,
}

pub(super) fn app_origin_request_allowed(
    headers: &axum::http::HeaderMap,
    app_id: &str,
    proxy_tag: &str,
) -> bool {
    let has_proxy_header = headers
        .get("x-agentark-app-proxy")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case(proxy_tag));
    let referer_ok = headers
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| reqwest::Url::parse(v).ok())
        .map(|url| {
            let path_prefix = format!("/apps/{}/", app_id);
            url.path().starts_with(&path_prefix) || url.path() == format!("/apps/{}", app_id)
        })
        .unwrap_or(false);
    has_proxy_header || referer_ok
}

pub(super) fn collapse_inline_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn split_app_query_csv(value: Option<&String>) -> Vec<String> {
    value
        .map(|raw| {
            raw.split([',', '|', '\n'])
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| item.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(super) fn push_unique_ci(items: &mut Vec<String>, seen: &mut HashSet<String>, value: &str) {
    let normalized = collapse_inline_whitespace(value.trim().trim_matches('"').trim_matches('\''));
    if normalized.is_empty() {
        return;
    }
    let key = normalized.to_ascii_lowercase();
    if seen.insert(key) {
        items.push(normalized);
    }
}

pub(super) fn normalize_arxiv_field_aliases(raw: &str) -> String {
    static ARXIV_FIELD_ALIAS_RE: OnceLock<Regex> = OnceLock::new();
    let re = ARXIV_FIELD_ALIAS_RE.get_or_init(|| {
        Regex::new(r"(?i)\b(title|abstract)\s*[:=]").expect("valid arxiv alias regex")
    });
    re.replace_all(raw, |caps: &regex::Captures| {
        let field = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        if field.eq_ignore_ascii_case("title") {
            "ti:"
        } else {
            "abs:"
        }
    })
    .into_owned()
}

pub(super) fn extract_arxiv_categories_and_keywords(
    search_query: &str,
) -> (Vec<String>, Vec<String>) {
    static BOOLEAN_SPLIT_RE: OnceLock<Regex> = OnceLock::new();
    static CATEGORY_VALUE_RE: OnceLock<Regex> = OnceLock::new();
    let split_re = BOOLEAN_SPLIT_RE.get_or_init(|| {
        Regex::new(r"(?i)\bANDNOT\b|\bAND\b|\bOR\b").expect("valid boolean split regex")
    });
    let category_re = CATEGORY_VALUE_RE
        .get_or_init(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9.\-]+$").expect("valid category regex"));

    let decoded = urlencoding::decode(search_query)
        .map(|value| value.into_owned())
        .unwrap_or_else(|_| search_query.to_string());
    let normalized = normalize_arxiv_field_aliases(&decoded);
    let mut categories = Vec::new();
    let mut keywords = Vec::new();
    let mut seen_categories = HashSet::new();
    let mut seen_keywords = HashSet::new();

    for token in split_re.split(&normalized) {
        let cleaned = collapse_inline_whitespace(
            token
                .trim()
                .trim_matches('(')
                .trim_matches(')')
                .trim_matches('"')
                .trim(),
        );
        if cleaned.is_empty() {
            continue;
        }
        let lower = cleaned.to_ascii_lowercase();
        if lower.starts_with("submitteddate:") {
            continue;
        }
        if lower.starts_with("cat:") {
            let category = cleaned[4..].trim().trim_matches('"');
            if category_re.is_match(category) {
                push_unique_ci(&mut categories, &mut seen_categories, category);
            }
            continue;
        }
        if lower.starts_with("ti:") || lower.starts_with("abs:") || lower.starts_with("all:") {
            let value = cleaned
                .split_once(':')
                .map(|(_, value)| value)
                .unwrap_or_default();
            push_unique_ci(&mut keywords, &mut seen_keywords, value);
            continue;
        }
        if !cleaned.contains(':') {
            push_unique_ci(&mut keywords, &mut seen_keywords, &cleaned);
        }
    }

    (categories, keywords)
}

pub(super) fn build_canonical_arxiv_search_query(
    categories: &[String],
    keywords: &[String],
    raw_fallback: Option<&str>,
) -> String {
    let mut groups = Vec::new();

    if !categories.is_empty() {
        let category_group = categories
            .iter()
            .map(|category| format!("cat:{}", category))
            .collect::<Vec<_>>()
            .join(" OR ");
        groups.push(if categories.len() > 1 {
            format!("({})", category_group)
        } else {
            category_group
        });
    }

    if !keywords.is_empty() {
        let keyword_group = keywords
            .iter()
            .flat_map(|keyword| {
                let value = if keyword.contains(' ') {
                    format!("\"{}\"", keyword)
                } else {
                    keyword.to_string()
                };
                [format!("ti:{}", value), format!("abs:{}", value)]
            })
            .collect::<Vec<_>>()
            .join(" OR ");
        groups.push(if keywords.len() > 1 {
            format!("({})", keyword_group)
        } else {
            keyword_group
        });
    }

    if !groups.is_empty() {
        return groups.join(" AND ");
    }

    raw_fallback
        .map(normalize_arxiv_field_aliases)
        .map(|value| collapse_inline_whitespace(&value))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "all:electron".to_string())
}

pub(super) fn arxiv_sort_by_param(raw: Option<&str>) -> String {
    match raw.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
        "relevance" => "relevance".to_string(),
        "lastupdateddate" | "last_updated_date" | "updated" => "lastUpdatedDate".to_string(),
        _ => "submittedDate".to_string(),
    }
}

pub(super) fn arxiv_sort_order_param(raw: Option<&str>) -> String {
    match raw.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
        "ascending" | "asc" => "ascending".to_string(),
        _ => "descending".to_string(),
    }
}

pub(super) fn extract_query_value_from_url(parsed: &reqwest::Url, key: &str) -> Option<String> {
    parsed
        .query()
        .and_then(|query| {
            url::form_urlencoded::parse(query.as_bytes())
                .find_map(|(query_key, value)| (query_key == key).then(|| value.into_owned()))
        })
        .filter(|value| !value.trim().is_empty())
}

pub(super) fn extract_query_value_from_raw_url(raw: &str, key: &str) -> Option<String> {
    let query = raw.split_once('?')?.1;
    let http_prefix = format!("{}=http://", key);
    let https_prefix = format!("{}=https://", key);
    let local_proxy_prefix = format!("{}=/", key);
    if query.starts_with(&http_prefix)
        || query.starts_with(&https_prefix)
        || query.starts_with(&local_proxy_prefix)
    {
        return query
            .split_once('=')
            .map(|(_, value)| value.trim().to_string())
            .filter(|value| !value.is_empty());
    }
    url::form_urlencoded::parse(query.as_bytes())
        .find_map(|(query_key, value)| (query_key == key).then(|| value.into_owned()))
        .filter(|value| !value.trim().is_empty())
}

pub(super) fn extract_wrapped_public_proxy_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.starts_with("/public/proxy/raw?url=") {
        return Some(
            trimmed
                .trim_start_matches("/public/proxy/raw?url=")
                .trim()
                .to_string(),
        );
    }
    if trimmed.starts_with("/apps/") && trimmed.contains("/__agentark/http/fetch") {
        return extract_query_value_from_raw_url(trimmed, "url");
    }
    if trimmed.starts_with("/apps/") && trimmed.contains("/__agentark/arxiv/search") {
        return extract_query_value_from_raw_url(trimmed, "source_url");
    }

    let parsed = reqwest::Url::parse(trimmed).ok()?;
    if parsed.path() == "/public/proxy/raw" {
        return extract_query_value_from_url(&parsed, "url");
    }

    let host = parsed
        .host_str()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if host == "api.allorigins.win" && parsed.path() == "/raw" {
        return extract_query_value_from_url(&parsed, "url");
    }
    if host == "api.codetabs.com" && parsed.path().starts_with("/v1/proxy") {
        return extract_query_value_from_url(&parsed, "quest");
    }
    if host == "corsproxy.io" {
        if let Some(query) = parsed.query() {
            let trimmed_query = query.trim();
            if trimmed_query.starts_with("http://") || trimmed_query.starts_with("https://") {
                return Some(trimmed_query.to_string());
            }
            if let Some(url_value) = extract_query_value_from_url(&parsed, "url") {
                return Some(url_value);
            }
        }
    }
    if parsed.path().starts_with("/apps/") && parsed.path().contains("/__agentark/http/fetch") {
        return extract_query_value_from_url(&parsed, "url");
    }
    if parsed.path().starts_with("/apps/") && parsed.path().contains("/__agentark/arxiv/search") {
        return extract_query_value_from_url(&parsed, "source_url");
    }
    None
}

pub(super) fn unwrap_nested_public_proxy_url(raw: &str) -> String {
    let mut current = raw.trim().to_string();
    for _ in 0..5 {
        let decoded = urlencoding::decode(&current)
            .map(|value| value.into_owned())
            .unwrap_or_else(|_| current.clone());
        let trimmed = decoded.trim().to_string();
        if let Some(unwrapped) = extract_wrapped_public_proxy_url(&trimmed) {
            current = unwrapped;
            continue;
        }
        return trimmed;
    }
    current
}

pub(super) fn trim_public_proxy_url_candidate(raw: &str) -> &str {
    raw.trim_end_matches(|ch: char| matches!(ch, ',' | ';' | '.'))
}

pub(super) fn is_public_proxy_target_url_candidate(url: &reqwest::Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let host = url.host_str().unwrap_or_default().trim();
    !host.is_empty() && !is_local_or_private_host_for_upgrade(host)
}

pub(super) fn extract_public_proxy_target_hosts_from_text(content: &str) -> HashSet<String> {
    static ABSOLUTE_URL_RE: OnceLock<Regex> = OnceLock::new();
    static WRAPPED_PROXY_URL_RE: OnceLock<Regex> = OnceLock::new();
    let absolute_url_re = ABSOLUTE_URL_RE.get_or_init(|| {
        Regex::new(r#"https?://[^\s"'<>`)]+"#).expect("valid absolute public URL regex")
    });
    let wrapped_proxy_url_re = WRAPPED_PROXY_URL_RE.get_or_init(|| {
        Regex::new(
            r#"/(?:public/proxy/raw\?url=|apps/[A-Za-z0-9_-]+/__agentark/(?:http/fetch\?url=|arxiv/search\?source_url=))[^\s"'<>`)]+"#,
        )
        .expect("valid wrapped public URL regex")
    });

    let mut hosts = HashSet::new();
    for raw_match in absolute_url_re
        .find_iter(content)
        .chain(wrapped_proxy_url_re.find_iter(content))
    {
        let raw_candidate = trim_public_proxy_url_candidate(raw_match.as_str());
        let unwrapped = unwrap_nested_public_proxy_url(raw_candidate);
        let Ok(parsed) = reqwest::Url::parse(&unwrapped) else {
            continue;
        };
        if !is_public_proxy_target_url_candidate(&parsed) {
            continue;
        }
        let host = parsed
            .host_str()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if !host.is_empty() {
            hosts.insert(host);
        }
    }

    hosts
}

pub(super) fn should_scan_app_public_proxy_file(path: &FsPath) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()).map(|ext| ext.to_ascii_lowercase()),
        Some(ext)
            if matches!(
                ext.as_str(),
                "html"
                    | "htm"
                    | "js"
                    | "mjs"
                    | "cjs"
                    | "jsx"
                    | "ts"
                    | "tsx"
                    | "json"
                    | "css"
                    | "txt"
                    | "md"
            )
    )
}

pub(super) fn should_descend_public_proxy_scan_entry(entry: &walkdir::DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
    !matches!(
        name.as_str(),
        ".git" | "node_modules" | "target" | ".next" | ".venv" | "venv"
    )
}

pub(super) fn scan_public_proxy_hosts_from_app_dir_sync(app_dir: &FsPath) -> HashSet<String> {
    const MAX_FILES: usize = 256;
    const MAX_FILE_BYTES: u64 = 512 * 1024;

    let mut hosts = HashSet::new();
    let mut scanned_files = 0usize;

    for entry in walkdir::WalkDir::new(app_dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(should_descend_public_proxy_scan_entry)
        .filter_map(Result::ok)
    {
        if scanned_files >= MAX_FILES {
            break;
        }
        if !entry.file_type().is_file() || !should_scan_app_public_proxy_file(entry.path()) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.len() > MAX_FILE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        let content = String::from_utf8_lossy(&bytes);
        hosts.extend(extract_public_proxy_target_hosts_from_text(&content));
        scanned_files += 1;
    }

    hosts
}

pub(super) async fn load_allowed_public_proxy_hosts_for_app(app_dir: PathBuf) -> HashSet<String> {
    tokio::task::spawn_blocking(move || scan_public_proxy_hosts_from_app_dir_sync(&app_dir))
        .await
        .unwrap_or_default()
}

pub(super) fn build_arxiv_search_request_from_source_url(
    raw_source_url: &str,
) -> Option<AppArxivSearchRequest> {
    let source_url = unwrap_nested_public_proxy_url(raw_source_url);
    let without_fragment = source_url.split('#').next()?.trim();
    let (base, raw_query) = without_fragment.split_once('?')?;
    let mut base_url = reqwest::Url::parse(base).ok()?;
    if !is_allowed_public_proxy_host(base_url.host_str().unwrap_or_default()) {
        return None;
    }
    if base_url.path() != "/api/query" {
        return None;
    }
    if base_url.scheme() == "http" {
        let _ = base_url.set_scheme("https");
    }

    let params: HashMap<String, String> = url::form_urlencoded::parse(raw_query.as_bytes())
        .into_owned()
        .collect();
    let raw_search_query = params.get("search_query").cloned().unwrap_or_default();
    let (categories, keywords) = extract_arxiv_categories_and_keywords(&raw_search_query);
    Some(AppArxivSearchRequest {
        source_url,
        search_query: build_canonical_arxiv_search_query(
            &categories,
            &keywords,
            Some(&raw_search_query),
        ),
        start: params
            .get("start")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0),
        max_results: params
            .get("max_results")
            .and_then(|value| value.parse::<usize>().ok())
            .map(|value| value.clamp(1, 50))
            .unwrap_or(25),
        sort_by: arxiv_sort_by_param(params.get("sortBy").map(String::as_str)),
        sort_order: arxiv_sort_order_param(params.get("sortOrder").map(String::as_str)),
        categories,
        keywords,
    })
}

pub(super) fn build_arxiv_search_request_from_query(
    raw_query: Option<&str>,
) -> Option<AppArxivSearchRequest> {
    let params: HashMap<String, String> = raw_query
        .map(|query| {
            url::form_urlencoded::parse(query.as_bytes())
                .into_owned()
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();

    if let Some(source_url) = params
        .get("source_url")
        .filter(|value| !value.trim().is_empty())
    {
        return build_arxiv_search_request_from_source_url(source_url);
    }

    let mut categories = split_app_query_csv(
        params
            .get("categories")
            .or_else(|| params.get("subjects"))
            .or_else(|| params.get("category")),
    );
    let mut keywords = split_app_query_csv(
        params
            .get("keywords")
            .or_else(|| params.get("terms"))
            .or_else(|| params.get("q")),
    );
    let raw_search_query = params.get("search_query").cloned().unwrap_or_default();
    if (categories.is_empty() && keywords.is_empty()) && !raw_search_query.trim().is_empty() {
        let extracted = extract_arxiv_categories_and_keywords(&raw_search_query);
        categories = extracted.0;
        keywords = extracted.1;
    }

    if categories.is_empty() && keywords.is_empty() && raw_search_query.trim().is_empty() {
        return None;
    }

    Some(AppArxivSearchRequest {
        source_url: String::new(),
        search_query: build_canonical_arxiv_search_query(
            &categories,
            &keywords,
            (!raw_search_query.trim().is_empty()).then_some(raw_search_query.as_str()),
        ),
        start: params
            .get("start")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0),
        max_results: params
            .get("max_results")
            .and_then(|value| value.parse::<usize>().ok())
            .map(|value| value.clamp(1, 50))
            .unwrap_or(25),
        sort_by: arxiv_sort_by_param(
            params
                .get("sortBy")
                .or_else(|| params.get("sort_by"))
                .map(String::as_str),
        ),
        sort_order: arxiv_sort_order_param(
            params
                .get("sortOrder")
                .or_else(|| params.get("sort_order"))
                .map(String::as_str),
        ),
        categories,
        keywords,
    })
}

pub(super) fn arxiv_upstream_url(request: &AppArxivSearchRequest) -> reqwest::Url {
    let mut url =
        reqwest::Url::parse("https://export.arxiv.org/api/query").expect("valid arxiv url");
    url.query_pairs_mut()
        .append_pair("search_query", &request.search_query)
        .append_pair("start", &request.start.to_string())
        .append_pair("max_results", &request.max_results.to_string())
        .append_pair("sortBy", &request.sort_by)
        .append_pair("sortOrder", &request.sort_order);
    url
}

pub(super) fn canonicalize_public_arxiv_api_url(parsed: &reqwest::Url) -> reqwest::Url {
    let host = parsed.host_str().unwrap_or_default();
    if !is_allowed_public_proxy_host(host) || parsed.path() != "/api/query" {
        return parsed.clone();
    }

    build_arxiv_search_request_from_source_url(parsed.as_str())
        .map(|request| arxiv_upstream_url(&request))
        .unwrap_or_else(|| parsed.clone())
}

pub(super) fn canonical_arxiv_id(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Ok(parsed) = reqwest::Url::parse(trimmed) {
        let path = parsed.path().trim_start_matches('/');
        return path
            .strip_prefix("abs/")
            .or_else(|| path.strip_prefix("pdf/"))
            .unwrap_or(path)
            .trim_end_matches(".pdf")
            .to_string();
    }
    trimmed.to_string()
}

pub(super) fn decode_arxiv_xml_entities(raw: &str) -> String {
    raw.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

pub(super) fn first_xml_tag_text(block: &str, tag: &str) -> String {
    let pattern = format!(
        r"(?is)<(?:[A-Za-z0-9_]+:)?{tag}\b[^>]*>(.*?)</(?:[A-Za-z0-9_]+:)?{tag}>",
        tag = regex::escape(tag)
    );
    Regex::new(&pattern)
        .ok()
        .and_then(|re| re.captures(block))
        .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
        .map(|value| collapse_inline_whitespace(&decode_arxiv_xml_entities(&value)))
        .unwrap_or_default()
}

pub(super) fn all_xml_tag_text(block: &str, tag: &str) -> Vec<String> {
    let pattern = format!(
        r"(?is)<(?:[A-Za-z0-9_]+:)?{tag}\b[^>]*>(.*?)</(?:[A-Za-z0-9_]+:)?{tag}>",
        tag = regex::escape(tag)
    );
    Regex::new(&pattern)
        .ok()
        .map(|re| {
            re.captures_iter(block)
                .filter_map(|caps| caps.get(1).map(|m| m.as_str()))
                .map(decode_arxiv_xml_entities)
                .map(|value| collapse_inline_whitespace(&value))
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(super) fn xml_attr_value(attrs: &str, key: &str) -> String {
    let pattern = format!(r#"(?is)\b{}\s*=\s*"([^"]*)""#, regex::escape(key));
    Regex::new(&pattern)
        .ok()
        .and_then(|re| re.captures(attrs))
        .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
        .map(|value| decode_arxiv_xml_entities(&value))
        .unwrap_or_default()
}

pub(super) fn parse_arxiv_atom_feed(xml: &str) -> anyhow::Result<serde_json::Value> {
    static ENTRY_RE: OnceLock<Regex> = OnceLock::new();
    static LINK_RE: OnceLock<Regex> = OnceLock::new();
    static CATEGORY_RE: OnceLock<Regex> = OnceLock::new();
    static PRIMARY_CATEGORY_RE: OnceLock<Regex> = OnceLock::new();
    let entry_re = ENTRY_RE.get_or_init(|| {
        Regex::new(r"(?is)<entry\b[^>]*>(.*?)</entry>").expect("valid arxiv entry regex")
    });
    let link_re = LINK_RE.get_or_init(|| {
        Regex::new(r"(?is)<(?:[A-Za-z0-9_]+:)?link\b([^>]*)/?>").expect("valid arxiv link regex")
    });
    let category_re = CATEGORY_RE.get_or_init(|| {
        Regex::new(r"(?is)<(?:[A-Za-z0-9_]+:)?category\b([^>]*)/?>")
            .expect("valid arxiv category regex")
    });
    let primary_category_re = PRIMARY_CATEGORY_RE.get_or_init(|| {
        Regex::new(r"(?is)<(?:[A-Za-z0-9_]+:)?primary_category\b([^>]*)/?>")
            .expect("valid arxiv primary category regex")
    });

    let total_results = first_xml_tag_text(xml, "totalResults")
        .parse::<usize>()
        .unwrap_or(0);
    let start_index = first_xml_tag_text(xml, "startIndex")
        .parse::<usize>()
        .unwrap_or(0);
    let items_per_page = first_xml_tag_text(xml, "itemsPerPage")
        .parse::<usize>()
        .unwrap_or(0);
    let mut papers = Vec::new();

    for entry_caps in entry_re.captures_iter(xml) {
        let Some(entry_block) = entry_caps.get(1).map(|m| m.as_str()) else {
            continue;
        };

        let mut paper_url = String::new();
        let mut pdf_url = String::new();
        for link_caps in link_re.captures_iter(entry_block) {
            let attrs = link_caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let href = xml_attr_value(attrs, "href");
            let rel = xml_attr_value(attrs, "rel");
            let title = xml_attr_value(attrs, "title");
            let content_type = xml_attr_value(attrs, "type");
            if rel == "alternate" && paper_url.is_empty() {
                paper_url = href.clone();
            }
            if title.eq_ignore_ascii_case("pdf")
                || (rel == "related" && content_type.contains("pdf"))
            {
                pdf_url = href;
            }
        }

        let mut categories = Vec::new();
        for category_caps in category_re.captures_iter(entry_block) {
            let attrs = category_caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let value = xml_attr_value(attrs, "term");
            if !value.is_empty() && !categories.iter().any(|item| item == &value) {
                categories.push(value);
            }
        }
        let primary_category = primary_category_re
            .captures(entry_block)
            .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
            .map(|attrs| xml_attr_value(&attrs, "term"))
            .filter(|value| !value.is_empty())
            .or_else(|| categories.first().cloned())
            .unwrap_or_default();
        let id = canonical_arxiv_id(&first_xml_tag_text(entry_block, "id"));

        papers.push(AppArxivPaper {
            id,
            paper_url,
            pdf_url,
            title: first_xml_tag_text(entry_block, "title"),
            summary: first_xml_tag_text(entry_block, "summary"),
            published: first_xml_tag_text(entry_block, "published"),
            updated: first_xml_tag_text(entry_block, "updated"),
            authors: all_xml_tag_text(entry_block, "name"),
            primary_category,
            categories,
        });
    }

    Ok(serde_json::json!({
        "total_results": total_results,
        "start_index": start_index,
        "items_per_page": items_per_page,
        "papers": papers,
    }))
}

pub(super) async fn app_scoped_arxiv_search_proxy(
    _state: &AppState,
    app_id: &str,
    headers: &axum::http::HeaderMap,
    raw_query: Option<&str>,
) -> Response {
    if !app_origin_request_allowed(headers, app_id, "arxiv") {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "forbidden",
                "message": "app-scoped arXiv helper requires app-origin request context"
            })),
        )
            .into_response();
    }

    let Some(request) = build_arxiv_search_request_from_query(raw_query) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "missing_query",
                "message": "Provide source_url, search_query, categories, keywords, or q."
            })),
        )
            .into_response();
    };

    let upstream_url = arxiv_upstream_url(&request);
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::limited(3))
        .user_agent(crate::branding::user_agent_with_suffix("arXiv app helper"))
        .build()
    {
        Ok(client) => client,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let response = match client.get(upstream_url.clone()).send().await {
        Ok(response) => response,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": "arxiv_fetch_failed",
                    "message": error.to_string(),
                    "query": request.search_query,
                })),
            )
                .into_response();
        }
    };

    if !response.status().is_success() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": "arxiv_upstream_error",
                "status": response.status().as_u16(),
                "query": request.search_query,
                "upstream_url": upstream_url.to_string(),
            })),
        )
            .into_response();
    }

    let xml = match response.text().await {
        Ok(xml) => xml,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": "arxiv_read_failed",
                    "message": error.to_string(),
                })),
            )
                .into_response();
        }
    };

    let parsed = match parse_arxiv_atom_feed(&xml) {
        Ok(parsed) => parsed,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": "arxiv_parse_failed",
                    "message": error.to_string(),
                    "query": request.search_query,
                })),
            )
                .into_response();
        }
    };

    let mut payload = parsed.as_object().cloned().unwrap_or_default();
    payload.insert(
        "query".to_string(),
        serde_json::json!({
            "search_query": request.search_query,
            "start": request.start,
            "max_results": request.max_results,
            "sortBy": request.sort_by,
            "sortOrder": request.sort_order,
            "categories": request.categories,
            "keywords": request.keywords,
            "source_url": request.source_url,
            "upstream_url": upstream_url.to_string(),
        }),
    );

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        serde_json::Value::Object(payload).to_string(),
    )
        .into_response()
}

pub(super) fn extract_public_proxy_target_from_query(
    raw_query: Option<&str>,
    key: &str,
) -> Option<String> {
    let mut raw_url = raw_query
        .and_then(|query| {
            url::form_urlencoded::parse(query.as_bytes())
                .find_map(|(query_key, value)| (query_key == key).then(|| value.into_owned()))
        })
        .unwrap_or_default();
    if let Some(query) = raw_query.map(str::trim) {
        let http_prefix = format!("{}=http://", key);
        let https_prefix = format!("{}=https://", key);
        if query.starts_with(&http_prefix) || query.starts_with(&https_prefix) {
            raw_url = query
                .split_once('=')
                .map(|(_, value)| value.trim().to_string())
                .unwrap_or_default();
        }
    }
    let trimmed = raw_url.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub(super) fn parse_public_proxy_target_url(
    raw_url: &str,
) -> Result<reqwest::Url, (&'static str, &'static str)> {
    let unwrapped = unwrap_nested_public_proxy_url(raw_url);
    let mut parsed = reqwest::Url::parse(&unwrapped).map_err(|_| ("invalid_url", "invalid url"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(("invalid_scheme", "only http and https urls are allowed"));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err((
            "embedded_credentials",
            "embedded credentials are not allowed in proxied urls",
        ));
    }
    let host = parsed.host_str().unwrap_or_default().trim();
    if host.is_empty() {
        return Err(("missing_host", "url must include a host"));
    }
    if is_local_or_private_host_for_upgrade(host) {
        return Err(("host_not_allowed", "host not allowed"));
    }
    if parsed.scheme() == "http" {
        let _ = parsed.set_scheme("https");
    }
    if parsed.scheme() != "https" {
        return Err(("invalid_scheme", "only https urls are allowed"));
    }
    Ok(parsed)
}

pub(super) async fn app_scoped_public_fetch_proxy(
    state: &AppState,
    app_id: &str,
    headers: &axum::http::HeaderMap,
    method: &Method,
    raw_query: Option<&str>,
) -> Response {
    if !app_origin_request_allowed(headers, app_id, "raw") {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "forbidden",
                "message": "app-scoped fetch proxy requires app-origin request context"
            })),
        )
            .into_response();
    }

    let Some(raw_url) = extract_public_proxy_target_from_query(raw_query, "url") else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "missing_url",
                "message": "Provide a url query parameter."
            })),
        )
            .into_response();
    };

    let parsed = match parse_public_proxy_target_url(&raw_url) {
        Ok(url) => url,
        Err((error, message)) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": error,
                    "message": message
                })),
            )
                .into_response();
        }
    };
    let parsed = canonicalize_public_arxiv_api_url(&parsed);
    let host = parsed
        .host_str()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    let Some(app_dir) = state.app_registry.get_dir(app_id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let allowed_hosts = load_allowed_public_proxy_hosts_for_app(app_dir).await;
    if !allowed_hosts.contains(&host) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "host_not_allowed_for_app",
                "host": host,
                "message": "Public app fetch proxy only allows public hosts referenced by the deployed app source."
            })),
        )
            .into_response();
    }

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::limited(3))
        .user_agent(crate::branding::user_agent_with_suffix(
            "public app fetch proxy",
        ))
        .build()
    {
        Ok(client) => client,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let mut request_builder = if *method == Method::HEAD {
        client.head(parsed.clone())
    } else {
        client.get(parsed.clone())
    };
    for header_name in [
        "accept",
        "accept-language",
        "if-none-match",
        "if-modified-since",
        "range",
    ] {
        if let Some(value) = headers
            .get(header_name)
            .and_then(|value| value.to_str().ok())
        {
            request_builder = request_builder.header(header_name, value);
        }
    }

    let response = match request_builder.send().await {
        Ok(response) => response,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": "fetch_failed",
                    "message": error.to_string(),
                    "url": parsed.to_string(),
                })),
            )
                .into_response();
        }
    };

    if !response.status().is_success() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": "upstream_error",
                "status": response.status().as_u16(),
                "url": parsed.to_string(),
            })),
        )
            .into_response();
    }

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    if *method == Method::HEAD {
        return (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, content_type),
                (header::CACHE_CONTROL, "no-store".to_string()),
                (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".to_string()),
            ],
            Vec::<u8>::new(),
        )
            .into_response();
    }

    match response.bytes().await {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, content_type),
                (header::CACHE_CONTROL, "no-store".to_string()),
                (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".to_string()),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::BAD_GATEWAY.into_response(),
    }
}

pub(super) async fn app_scoped_llm_chat_proxy(
    state: &AppState,
    app_id: &str,
    headers: &axum::http::HeaderMap,
    body: axum::body::Body,
) -> Response {
    let has_proxy_header = headers
        .get("x-agentark-app-proxy")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("llm"));
    let referer_ok = headers
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| reqwest::Url::parse(v).ok())
        .map(|url| {
            let path_prefix = format!("/apps/{}/", app_id);
            url.path().starts_with(&path_prefix) || url.path() == format!("/apps/{}", app_id)
        })
        .unwrap_or(false);
    if !has_proxy_header && !referer_ok {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "forbidden",
                "message": "app-scoped LLM proxy requires app-origin request context"
            })),
        )
            .into_response();
    }

    let body_bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({ "error": "request body too large" })),
            )
                .into_response();
        }
    };
    let payload: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid JSON payload" })),
            )
                .into_response();
        }
    };

    let stream_requested = payload
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if stream_requested {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "streaming_not_supported",
                "message": "Use non-streaming chat completion for app proxy requests."
            })),
        )
            .into_response();
    }

    let mut system_lines: Vec<String> = Vec::new();
    let mut convo: Vec<(String, String)> = Vec::new();
    let requested_model_hint = payload
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    if let Some(messages) = payload.get("messages").and_then(|v| v.as_array()) {
        for msg in messages {
            let Some(obj) = msg.as_object() else {
                continue;
            };
            let role = obj
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("user")
                .to_ascii_lowercase();
            let Some(content_val) = obj.get("content") else {
                continue;
            };
            let Some(text) = extract_openai_message_text(content_val) else {
                continue;
            };
            if role == "system" {
                system_lines.push(text);
            } else {
                convo.push((role, text));
            }
        }
    }

    if convo.is_empty() {
        if let Some(input) = payload.get("input") {
            if let Some(text) = extract_openai_message_text(input) {
                convo.push(("user".to_string(), text));
            } else if let Some(arr) = input.as_array() {
                for item in arr {
                    let Some(obj) = item.as_object() else {
                        continue;
                    };
                    let role = obj
                        .get("role")
                        .and_then(|v| v.as_str())
                        .unwrap_or("user")
                        .to_ascii_lowercase();
                    let content_val = obj.get("content").or_else(|| obj.get("text"));
                    let Some(content_val) = content_val else {
                        continue;
                    };
                    let Some(text) = extract_openai_message_text(content_val) else {
                        continue;
                    };
                    if role == "system" {
                        system_lines.push(text);
                    } else {
                        convo.push((role, text));
                    }
                }
            }
        }
    }

    if convo.is_empty() {
        if let Some(prompt) = payload.get("prompt").and_then(|v| v.as_str()) {
            let trimmed = prompt.trim();
            if !trimmed.is_empty() {
                convo.push(("user".to_string(), trimmed.to_string()));
            }
        }
    }

    if convo.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "missing_messages",
                "message": "Provide messages[], input, or prompt."
            })),
        )
            .into_response();
    }

    let system_prompt = if system_lines.is_empty() {
        "You are a concise assistant helping summarize and explain app content for the end user."
            .to_string()
    } else {
        system_lines.join("\n")
    };

    let (last_role, last_text) = convo.pop().unwrap_or(("user".to_string(), String::new()));
    let user_message = if last_text.trim().is_empty() {
        "Please help with this request.".to_string()
    } else if last_role == "assistant" {
        format!(
            "Continue from the previous assistant context:\n{}",
            last_text
        )
    } else {
        last_text
    };

    let history: Vec<crate::core::ConversationMessage> = convo
        .into_iter()
        .map(|(role, content)| crate::core::ConversationMessage {
            role: if role == "assistant" {
                "assistant".to_string()
            } else {
                "user".to_string()
            },
            content,
            _timestamp: chrono::Utc::now(),
        })
        .collect();

    let (selected_llm, model_name, selection_note) = {
        let agent = state.agent.read().await;
        let (llm, _slot_label, note) =
            agent.select_llm_for_app_proxy(requested_model_hint.as_deref());
        let name = llm.model_name().to_string();
        (llm, name, note)
    };

    let no_actions: Vec<crate::actions::ActionDef> = Vec::new();
    let response = match selected_llm
        .chat_with_history(&system_prompt, &user_message, &history, &[], &no_actions)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": "llm_proxy_failed",
                    "message": format!("LLM request failed: {}", e)
                })),
            )
                .into_response();
        }
    };
    let assistant_content = response.content;

    let openai_like = serde_json::json!({
        "id": format!("chatcmpl-{}", uuid::Uuid::new_v4()),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": model_name,
        "selection_note": selection_note,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": &assistant_content,
            },
            "text": &assistant_content,
            "finish_reason": "stop"
        }],
        "output_text": &assistant_content,
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": &assistant_content
            }]
        }],
        "status": "completed",
        "usage": response.usage.as_ref().map(|u| serde_json::json!({
            "prompt_tokens": u.prompt_tokens,
            "completion_tokens": u.completion_tokens,
            "total_tokens": u.total_tokens
        }))
    });

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        openai_like.to_string(),
    )
        .into_response()
}

pub(super) fn is_local_or_private_host_for_upgrade(host: &str) -> bool {
    let h = host
        .trim()
        .trim_matches('[')
        .trim_matches(']')
        .to_ascii_lowercase();
    if h.is_empty()
        || h == "localhost"
        || h.ends_with(".localhost")
        || h == "0.0.0.0"
        || h.ends_with(".local")
        || h.ends_with(".internal")
    {
        return true;
    }
    if let Ok(ip) = h.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
            }
            std::net::IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_unique_local()
                    || v6.is_unicast_link_local()
            }
        };
    }
    false
}

pub(super) fn upgrade_http_links_for_secure_origin(content: &str) -> String {
    static HTTP_URL_RE: OnceLock<Regex> = OnceLock::new();
    let re = HTTP_URL_RE.get_or_init(|| {
        Regex::new(r#"http://[A-Za-z0-9\.\-]+(?::\d+)?[^\s"'<>)]*"#)
            .expect("valid insecure URL regex")
    });
    re.replace_all(content, |caps: &regex::Captures| {
        let raw = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
        let Some(parsed) = reqwest::Url::parse(raw).ok() else {
            return raw.to_string();
        };
        let host = parsed.host_str().unwrap_or_default();
        if is_local_or_private_host_for_upgrade(host) {
            return raw.to_string();
        }
        raw.replacen("http://", "https://", 1)
    })
    .into_owned()
}

pub(super) fn is_allowed_public_proxy_host(host: &str) -> bool {
    let h = host.trim().to_ascii_lowercase();
    h == "export.arxiv.org" || h == "arxiv.org" || h.ends_with(".arxiv.org")
}

/// Public proxy for static tunneled apps.
/// Strict allowlist prevents open-proxy abuse.
pub(super) async fn public_proxy_raw(
    uri: Uri,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let raw_url = extract_public_proxy_target_from_query(uri.query(), "url")
        .or_else(|| params.get("url").map(|value| value.trim().to_string()));
    let Some(raw_url) = raw_url.filter(|value| !value.trim().is_empty()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "missing query param: url" })),
        )
            .into_response();
    };
    let parsed = match parse_public_proxy_target_url(&raw_url) {
        Ok(url) => url,
        Err((error, message)) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": error,
                    "message": message,
                })),
            )
                .into_response();
        }
    };

    let parsed = canonicalize_public_arxiv_api_url(&parsed);
    let host = parsed.host_str().unwrap_or("");
    if !is_allowed_public_proxy_host(host) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "host not allowed" })),
        )
            .into_response();
    }

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    match client.get(parsed).send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({
                        "error": format!("upstream returned {}", resp.status())
                    })),
                )
                    .into_response();
            }
            let content_type = resp
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_string();
            match resp.bytes().await {
                Ok(bytes) => (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, content_type),
                        (header::CACHE_CONTROL, "no-store".to_string()),
                        (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".to_string()),
                    ],
                    bytes,
                )
                    .into_response(),
                Err(_) => StatusCode::BAD_GATEWAY.into_response(),
            }
        }
        Err(_) => StatusCode::BAD_GATEWAY.into_response(),
    }
}

pub(super) fn extract_query_param(query: Option<&str>, key: &str) -> Option<String> {
    query.and_then(|q| {
        url::form_urlencoded::parse(q.as_bytes()).find_map(|(k, v)| {
            if k == key {
                Some(v.into_owned())
            } else {
                None
            }
        })
    })
}

pub(super) fn extract_query_param_any(query: Option<&str>, keys: &[&str]) -> Option<String> {
    query.and_then(|q| {
        url::form_urlencoded::parse(q.as_bytes()).find_map(|(k, v)| {
            if keys.iter().any(|candidate| k == *candidate) {
                Some(v.into_owned())
            } else {
                None
            }
        })
    })
}

pub(super) fn strip_query_params(query: Option<&str>, keys: &[&str]) -> Option<String> {
    let query = query?;
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    let mut has_pairs = false;
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        if keys.iter().any(|candidate| k == *candidate) {
            continue;
        }
        serializer.append_pair(&k, &v);
        has_pairs = true;
    }
    if has_pairs {
        Some(serializer.finish())
    } else {
        None
    }
}

pub(super) fn extract_cookie(headers: &axum::http::HeaderMap, cookie_name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').map(|c| c.trim()).find_map(|c| {
                c.strip_prefix(&format!("{}=", cookie_name))
                    .map(|v| v.to_string())
            })
        })
}

pub(super) fn filter_proxy_cookie(cookie_header: &str, app_id: &str) -> Option<String> {
    let app_cookie = format!("ark_app_{}=", app_id);
    let filtered: Vec<&str> = cookie_header
        .split(';')
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .filter(|c| !c.starts_with(&format!("{}=", crate::branding::SESSION_COOKIE_NAME)))
        .filter(|c| !c.starts_with(&app_cookie))
        .collect();
    if filtered.is_empty() {
        None
    } else {
        Some(filtered.join("; "))
    }
}

pub(super) fn build_app_url(app_id: &str, path: &str, query: Option<&str>) -> String {
    let mut url = if path.is_empty() {
        format!("/apps/{}/", app_id)
    } else {
        format!("/apps/{}/{}", app_id, path.trim_start_matches('/'))
    };
    if let Some(q) = query.filter(|q| !q.is_empty()) {
        url.push('?');
        url.push_str(q);
    }
    url
}

pub(super) fn build_absolute_app_url(
    base_url: Option<&str>,
    app_id: &str,
    path: &str,
    query: Option<&str>,
) -> String {
    let relative = build_app_url(app_id, path, query);
    match base_url {
        Some(base) if !base.trim().is_empty() => {
            format!("{}{}", base.trim_end_matches('/'), relative)
        }
        _ => relative,
    }
}

pub(super) fn app_root_url_for_state(state: &AppState, app_id: &str) -> String {
    build_absolute_app_url(state.public_app_base_url.as_deref(), app_id, "", None)
}

pub(super) async fn app_access_url_for_state(
    state: &AppState,
    app_id: &str,
    access_guard_enabled: bool,
) -> String {
    let relative = if access_guard_enabled {
        state
            .app_registry
            .issue_access_url(app_id)
            .await
            .unwrap_or_else(|| build_app_url(app_id, "", None))
    } else {
        build_app_url(app_id, "", None)
    };
    match state.public_app_base_url.as_deref() {
        Some(base) if !base.trim().is_empty() => {
            format!("{}{}", base.trim_end_matches('/'), relative)
        }
        _ => relative,
    }
}

pub(super) fn is_hop_by_hop_header(header_name: &str) -> bool {
    matches!(
        header_name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

pub(super) fn is_websocket_upgrade(headers: &axum::http::HeaderMap) -> bool {
    let has_upgrade_token = headers
        .get(header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false);
    let websocket_upgrade = headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    has_upgrade_token && websocket_upgrade
}

pub(super) fn axum_to_tungstenite_message(msg: AxumWsMessage) -> Option<TungsteniteMessage> {
    match msg {
        AxumWsMessage::Text(text) => Some(TungsteniteMessage::Text(text.to_string().into())),
        AxumWsMessage::Binary(data) => Some(TungsteniteMessage::Binary(data)),
        AxumWsMessage::Ping(data) => Some(TungsteniteMessage::Ping(data)),
        AxumWsMessage::Pong(data) => Some(TungsteniteMessage::Pong(data)),
        AxumWsMessage::Close(_) => Some(TungsteniteMessage::Close(None)),
    }
}

pub(super) fn tungstenite_to_axum_message(msg: TungsteniteMessage) -> Option<AxumWsMessage> {
    match msg {
        TungsteniteMessage::Text(text) => Some(AxumWsMessage::Text(text.to_string().into())),
        TungsteniteMessage::Binary(data) => Some(AxumWsMessage::Binary(data.into())),
        TungsteniteMessage::Ping(data) => Some(AxumWsMessage::Ping(data.into())),
        TungsteniteMessage::Pong(data) => Some(AxumWsMessage::Pong(data.into())),
        TungsteniteMessage::Close(_) => Some(AxumWsMessage::Close(None)),
        TungsteniteMessage::Frame(_) => None,
    }
}

pub(super) async fn proxy_websocket_connection(
    client_socket: WebSocket,
    upstream_url: String,
    requested_protocols: Vec<String>,
    forward_headers: Vec<(String, String)>,
) {
    let mut upstream_request = match upstream_url.into_client_request() {
        Ok(request) => request,
        Err(error) => {
            tracing::warn!("Failed to build upstream WS request: {}", error);
            return;
        }
    };
    if !requested_protocols.is_empty() {
        let protocols = requested_protocols.join(", ");
        if let Ok(value) = axum::http::HeaderValue::from_str(&protocols) {
            upstream_request
                .headers_mut()
                .insert("Sec-WebSocket-Protocol", value);
        }
    }
    for (name, value) in forward_headers {
        if let (Ok(header_name), Ok(header_value)) = (
            axum::http::HeaderName::from_bytes(name.as_bytes()),
            axum::http::HeaderValue::from_str(&value),
        ) {
            upstream_request
                .headers_mut()
                .insert(header_name, header_value);
        }
    }

    let (upstream_socket, _) = match tokio_tungstenite::connect_async(upstream_request).await {
        Ok(pair) => pair,
        Err(error) => {
            tracing::warn!("Failed to connect to upstream WS app: {}", error);
            return;
        }
    };

    let (mut client_sender, mut client_receiver) = client_socket.split();
    let (mut upstream_sender, mut upstream_receiver) = upstream_socket.split();

    let client_to_upstream = async {
        while let Some(result) = client_receiver.next().await {
            match result {
                Ok(message) => {
                    let Some(upstream_message) = axum_to_tungstenite_message(message) else {
                        continue;
                    };
                    if upstream_sender.send(upstream_message).await.is_err() {
                        break;
                    }
                }
                Err(error) => {
                    tracing::debug!("Client WS receive error: {}", error);
                    break;
                }
            }
        }
        let _ = upstream_sender.close().await;
    };

    let upstream_to_client = async {
        while let Some(result) = upstream_receiver.next().await {
            match result {
                Ok(message) => {
                    let Some(client_message) = tungstenite_to_axum_message(message) else {
                        continue;
                    };
                    if client_sender.send(client_message).await.is_err() {
                        break;
                    }
                }
                Err(error) => {
                    tracing::debug!("Upstream WS receive error: {}", error);
                    break;
                }
            }
        }
        let _ = client_sender.send(AxumWsMessage::Close(None)).await;
    };

    tokio::select! {
        _ = client_to_upstream => {}
        _ = upstream_to_client => {}
    }
}

/// Serve app root - static files or reverse proxy
pub(super) async fn serve_app_root(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    request: Request,
) -> Response {
    let (mut parts, body) = request.into_parts();
    let ws = if is_websocket_upgrade(&parts.headers) {
        WebSocketUpgrade::from_request_parts(&mut parts, &())
            .await
            .ok()
    } else {
        None
    };

    let method = parts.method.clone();
    let uri = parts.uri.clone();
    let headers = parts.headers.clone();

    serve_app_file_inner(
        &state,
        &app_id,
        "",
        AppServeRequestContext {
            method,
            uri,
            headers,
            ws,
            body,
        },
    )
    .await
}

/// Serve app file by path - static files or reverse proxy
pub(super) async fn serve_app_path(
    State(state): State<AppState>,
    Path((app_id, path)): Path<(String, String)>,
    request: Request,
) -> Response {
    let (mut parts, body) = request.into_parts();
    let ws = if is_websocket_upgrade(&parts.headers) {
        WebSocketUpgrade::from_request_parts(&mut parts, &())
            .await
            .ok()
    } else {
        None
    };

    let method = parts.method.clone();
    let uri = parts.uri.clone();
    let headers = parts.headers.clone();

    serve_app_file_inner(
        &state,
        &app_id,
        &path,
        AppServeRequestContext {
            method,
            uri,
            headers,
            ws,
            body,
        },
    )
    .await
}

pub(super) struct AppServeRequestContext {
    method: Method,
    uri: Uri,
    headers: axum::http::HeaderMap,
    ws: Option<WebSocketUpgrade>,
    body: axum::body::Body,
}

pub(super) fn should_proxy_app_request_to_executor(
    is_static: bool,
    executor_configured: bool,
) -> bool {
    executor_configured && !is_static
}

/// Inner handler: serve static file or reverse proxy to dynamic app
pub(super) async fn serve_app_file_inner(
    state: &AppState,
    app_id: &str,
    path: &str,
    request_ctx: AppServeRequestContext,
) -> Response {
    let method = request_ctx.method;
    let uri = request_ctx.uri;
    let headers = request_ctx.headers;
    let ws = request_ctx.ws;
    let body = request_ctx.body;

    if !is_valid_app_id(app_id) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    if state.server_role == HttpServerRole::ControlPlane
        && internet_facing_apps_should_be_isolated(
            state.deployment_mode,
            state.public_app_bind_addr.as_deref(),
        )
    {
        let target = build_absolute_app_url(
            state.public_app_base_url.as_deref(),
            app_id,
            path,
            uri.query(),
        );
        if target.starts_with("http://") || target.starts_with("https://") {
            return Response::builder()
                .status(StatusCode::TEMPORARY_REDIRECT)
                .header(header::LOCATION, target)
                .body(axum::body::Body::empty())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Public apps are isolated onto a dedicated app origin in internet-facing mode.",
        )
            .into_response();
    }

    // Check app existence first so unknown IDs return 404 instead of auth form.
    let Some(app_dir) = state.app_registry.get_dir(app_id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !state.app_registry.is_enabled(app_id).await {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "This app is disabled. Start it again from the Apps page.",
        )
            .into_response();
    }

    let access_guard_enabled = state.app_registry.access_guard_enabled(app_id).await;
    if state.server_role == HttpServerRole::PublicApps && !access_guard_enabled {
        return app_public_exposure_requires_guard_response(app_id, path, &headers);
    }
    let cookie_name = format!("ark_app_{}", app_id);
    let grant_from_query = if access_guard_enabled {
        extract_query_param(uri.query(), "grant")
    } else {
        None
    };
    let password_from_query = if access_guard_enabled {
        extract_query_param_any(uri.query(), &["password", "key"])
    } else {
        None
    };
    let session_from_cookie = if access_guard_enabled {
        extract_cookie(&headers, &cookie_name)
    } else {
        None
    };
    let key_from_header = if access_guard_enabled {
        ["x-agentark-app-password", "x-agentark-app-key"]
            .iter()
            .find_map(|header_name| {
                headers
                    .get(*header_name)
                    .and_then(|value| value.to_str().ok())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
            })
    } else {
        None
    };
    let is_ws_request = is_websocket_upgrade(&headers);

    if access_guard_enabled {
        let query_valid = match grant_from_query.as_deref() {
            Some(grant) => {
                state
                    .app_registry
                    .consume_access_bootstrap_grant(app_id, grant)
                    .await
            }
            None => false,
        };
        let cookie_valid = match session_from_cookie.as_deref() {
            Some(token) => {
                state
                    .app_registry
                    .validate_access_session(app_id, token)
                    .await
            }
            None => false,
        };
        let header_valid = match key_from_header.as_deref() {
            Some(key) => state.app_registry.verify_key(app_id, key).await,
            None => false,
        };
        let password_valid = match password_from_query.as_deref() {
            Some(password) => state.app_registry.verify_key(app_id, password).await,
            None => false,
        };

        if !query_valid && !cookie_valid && !header_valid && !password_valid {
            return app_access_denied_response(app_id, path, &headers);
        }

        // First successful password or bootstrap entry: set cookie and redirect to a clean URL.
        if (query_valid || password_valid)
            && !cookie_valid
            && !header_valid
            && method == Method::GET
            && !is_ws_request
        {
            if let Some(session_token) = state.app_registry.create_access_session(app_id).await {
                let request_proto = headers
                    .get("x-forwarded-proto")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("http");
                let secure_attr = if request_proto.eq_ignore_ascii_case("https") {
                    "; Secure"
                } else {
                    ""
                };
                let cookie = format!(
                    "{}={}; Path=/apps/{}; HttpOnly; SameSite=Lax; Max-Age=604800{}",
                    cookie_name, session_token, app_id, secure_attr
                );
                let clean_query = strip_query_params(uri.query(), &["grant", "password", "key"]);
                let clean_url = build_app_url(app_id, path, clean_query.as_deref());
                return Response::builder()
                    .status(StatusCode::FOUND)
                    .header(header::SET_COOKIE, cookie)
                    .header(header::LOCATION, clean_url)
                    .body(axum::body::Body::empty())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR.into_response());
            }
        }
    }

    let is_static = state.app_registry.is_static(app_id).await;

    state.app_registry.touch(app_id).await;
    let clean_query = if access_guard_enabled {
        strip_query_params(uri.query(), &["grant", "password", "key"])
    } else {
        uri.query().map(|q| q.to_string())
    };
    let normalized_path = path.trim_start_matches('/');
    if normalized_path.eq_ignore_ascii_case("__agentark/llm/chat") {
        if method != Method::POST {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        return app_scoped_llm_chat_proxy(state, app_id, &headers, body).await;
    }
    if normalized_path.eq_ignore_ascii_case("__agentark/http/fetch") {
        if method != Method::GET && method != Method::HEAD {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        return app_scoped_public_fetch_proxy(
            state,
            app_id,
            &headers,
            &method,
            clean_query.as_deref(),
        )
        .await;
    }
    if normalized_path.eq_ignore_ascii_case("__agentark/arxiv/search") {
        if method != Method::GET {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        return app_scoped_arxiv_search_proxy(state, app_id, &headers, clean_query.as_deref())
            .await;
    }

    let body_bytes = if is_ws_request {
        None
    } else {
        Some(match axum::body::to_bytes(body, 64 * 1024 * 1024).await {
            Ok(bytes) => bytes,
            Err(_) => {
                return (StatusCode::PAYLOAD_TOO_LARGE, "Request body too large").into_response();
            }
        })
    };

    if should_proxy_app_request_to_executor(is_static, state.executor_client.is_some()) {
        if let Some(executor) = state.executor_client.as_ref() {
            let mut proxy_path = if normalized_path.is_empty() {
                format!("/internal/v1/apps/{}/proxy", app_id)
            } else {
                format!("/internal/v1/apps/{}/proxy/{}", app_id, normalized_path)
            };
            if let Some(q) = clean_query.as_deref().filter(|q| !q.is_empty()) {
                proxy_path.push('?');
                proxy_path.push_str(q);
            }
            if is_ws_request {
                if method != Method::GET {
                    return StatusCode::METHOD_NOT_ALLOWED.into_response();
                }

                let Some(ws_upgrade) = ws else {
                    return (StatusCode::BAD_REQUEST, "Invalid websocket upgrade request")
                        .into_response();
                };

                let upstream_url = format!(
                    "{}{}",
                    executor_websocket_base_url(executor.base_url()),
                    proxy_path
                );
                let requested_protocols = headers
                    .get("Sec-WebSocket-Protocol")
                    .and_then(|v| v.to_str().ok())
                    .map(|raw| {
                        raw.split(',')
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                            .collect::<Vec<String>>()
                    })
                    .unwrap_or_default();
                let mut ws_forward_headers: Vec<(String, String)> = Vec::new();
                if let Some(v) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
                    ws_forward_headers.push(("origin".to_string(), v.to_string()));
                }
                if let Some(v) = headers
                    .get(header::USER_AGENT)
                    .and_then(|v| v.to_str().ok())
                {
                    ws_forward_headers.push(("user-agent".to_string(), v.to_string()));
                }
                if let Some(v) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) {
                    ws_forward_headers.push(("x-forwarded-host".to_string(), v.to_string()));
                }
                if let Some(token) = executor.bearer_token() {
                    ws_forward_headers
                        .push(("authorization".to_string(), format!("Bearer {}", token)));
                }
                let forwarded_proto = headers
                    .get("x-forwarded-proto")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("http");
                ws_forward_headers
                    .push(("x-forwarded-proto".to_string(), forwarded_proto.to_string()));
                ws_forward_headers.push((
                    "x-forwarded-prefix".to_string(),
                    format!("/apps/{}", app_id),
                ));
                if let Some(raw_cookie) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok())
                {
                    if let Some(filtered) = filter_proxy_cookie(raw_cookie, app_id) {
                        ws_forward_headers.push(("cookie".to_string(), filtered));
                    }
                }

                let ws_upgrade = if requested_protocols.is_empty() {
                    ws_upgrade
                } else {
                    ws_upgrade.protocols(requested_protocols.clone())
                };
                return ws_upgrade
                    .on_upgrade(move |socket| async move {
                        proxy_websocket_connection(
                            socket,
                            upstream_url,
                            requested_protocols,
                            ws_forward_headers,
                        )
                        .await;
                    })
                    .into_response();
            }

            let mut upstream = executor.request(
                reqwest::Method::from_bytes(method.as_str().as_bytes())
                    .unwrap_or(reqwest::Method::GET),
                &proxy_path,
            );
            for (name, value) in &headers {
                let lower = name.as_str().to_ascii_lowercase();
                if is_hop_by_hop_header(&lower)
                    || lower == "host"
                    || lower == "content-length"
                    || lower == "authorization"
                {
                    continue;
                }
                if lower == "cookie" {
                    if let Ok(raw_cookie) = value.to_str() {
                        if let Some(filtered) = filter_proxy_cookie(raw_cookie, app_id) {
                            upstream = upstream.header(header::COOKIE, filtered);
                        }
                    }
                    continue;
                }
                upstream = upstream.header(name, value);
            }
            if let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) {
                upstream = upstream.header("x-forwarded-host", host);
            }
            let forwarded_proto = headers
                .get("x-forwarded-proto")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("http");
            upstream = upstream
                .header("x-forwarded-proto", forwarded_proto)
                .header("x-forwarded-prefix", format!("/apps/{}", app_id));
            upstream = upstream.body(body_bytes.clone().unwrap_or_default());

            match upstream.send().await {
                Ok(resp) => {
                    let status = StatusCode::from_u16(resp.status().as_u16())
                        .unwrap_or(StatusCode::BAD_GATEWAY);
                    let response_headers = resp.headers().clone();
                    match resp.bytes().await {
                        Ok(response_body) => {
                            let mut builder = Response::builder().status(status);
                            for (name, value) in &response_headers {
                                if !is_hop_by_hop_header(name.as_str()) {
                                    builder = builder.header(name, value);
                                }
                            }
                            let response_body = if method == Method::HEAD {
                                axum::body::Body::empty()
                            } else {
                                axum::body::Body::from(response_body)
                            };
                            return builder
                                .body(response_body)
                                .unwrap_or(StatusCode::BAD_GATEWAY.into_response());
                        }
                        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        "Executor proxy failed for app {} on path '{}': {}",
                        app_id,
                        proxy_path,
                        error
                    );
                }
            }
        }
    }

    if let Some(port) = state.app_registry.get_port(app_id).await {
        if is_ws_request {
            if method != Method::GET {
                return StatusCode::METHOD_NOT_ALLOWED.into_response();
            }

            let Some(ws_upgrade) = ws else {
                return (StatusCode::BAD_REQUEST, "Invalid websocket upgrade request")
                    .into_response();
            };

            let upstream_path = path.trim_start_matches('/');
            let mut upstream_url = if upstream_path.is_empty() {
                format!("ws://127.0.0.1:{}/", port)
            } else {
                format!("ws://127.0.0.1:{}/{}", port, upstream_path)
            };
            if let Some(q) = clean_query.as_deref().filter(|q| !q.is_empty()) {
                upstream_url.push('?');
                upstream_url.push_str(q);
            }

            let requested_protocols = headers
                .get("Sec-WebSocket-Protocol")
                .and_then(|v| v.to_str().ok())
                .map(|raw| {
                    raw.split(',')
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .collect::<Vec<String>>()
                })
                .unwrap_or_default();
            let mut ws_forward_headers: Vec<(String, String)> = Vec::new();
            if let Some(v) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
                ws_forward_headers.push(("origin".to_string(), v.to_string()));
            }
            if let Some(v) = headers
                .get(header::USER_AGENT)
                .and_then(|v| v.to_str().ok())
            {
                ws_forward_headers.push(("user-agent".to_string(), v.to_string()));
            }
            if let Some(v) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) {
                ws_forward_headers.push(("x-forwarded-host".to_string(), v.to_string()));
            }
            let forwarded_proto = headers
                .get("x-forwarded-proto")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("http");
            ws_forward_headers.push(("x-forwarded-proto".to_string(), forwarded_proto.to_string()));
            ws_forward_headers.push((
                "x-forwarded-prefix".to_string(),
                format!("/apps/{}", app_id),
            ));
            if let Some(raw_cookie) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
                if let Some(filtered) = filter_proxy_cookie(raw_cookie, app_id) {
                    ws_forward_headers.push(("cookie".to_string(), filtered));
                }
            }

            let ws_upgrade = if requested_protocols.is_empty() {
                ws_upgrade
            } else {
                ws_upgrade.protocols(requested_protocols.clone())
            };
            return ws_upgrade
                .on_upgrade(move |socket| async move {
                    proxy_websocket_connection(
                        socket,
                        upstream_url,
                        requested_protocols,
                        ws_forward_headers,
                    )
                    .await;
                })
                .into_response();
        }

        let upstream_path = path.trim_start_matches('/');
        let mut target_url = if upstream_path.is_empty() {
            format!("http://127.0.0.1:{}/", port)
        } else {
            format!("http://127.0.0.1:{}/{}", port, upstream_path)
        };
        if let Some(q) = clean_query.as_deref().filter(|q| !q.is_empty()) {
            target_url.push('?');
            target_url.push_str(q);
        }

        let client = shared_http_client().clone();
        let mut upstream = client.request(method.clone(), &target_url);
        for (name, value) in &headers {
            let lower = name.as_str().to_ascii_lowercase();
            if is_hop_by_hop_header(&lower)
                || lower == "host"
                || lower == "content-length"
                || lower == "authorization"
            {
                continue;
            }
            if lower == "cookie" {
                if let Ok(raw_cookie) = value.to_str() {
                    if let Some(filtered) = filter_proxy_cookie(raw_cookie, app_id) {
                        upstream = upstream.header(header::COOKIE, filtered);
                    }
                }
                continue;
            }
            upstream = upstream.header(name, value);
        }
        if let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) {
            upstream = upstream.header("x-forwarded-host", host);
        }
        let forwarded_proto = headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("http");
        upstream = upstream
            .header("x-forwarded-proto", forwarded_proto)
            .header("x-forwarded-prefix", format!("/apps/{}", app_id))
            .body(body_bytes.unwrap_or_default());

        match upstream.send().await {
            Ok(resp) => {
                let status =
                    StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
                let response_headers = resp.headers().clone();
                match resp.bytes().await {
                    Ok(response_body) => {
                        let mut builder = Response::builder().status(status);
                        for (name, value) in &response_headers {
                            if !is_hop_by_hop_header(name.as_str()) {
                                builder = builder.header(name, value);
                            }
                        }
                        let response_body = if method == Method::HEAD {
                            axum::body::Body::empty()
                        } else {
                            axum::body::Body::from(response_body)
                        };
                        builder
                            .body(response_body)
                            .unwrap_or(StatusCode::BAD_GATEWAY.into_response())
                    }
                    Err(_) => StatusCode::BAD_GATEWAY.into_response(),
                }
            }
            Err(_) => {
                (StatusCode::SERVICE_UNAVAILABLE, "App server not responding").into_response()
            }
        }
    } else if is_static {
        if method != Method::GET && method != Method::HEAD {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }

        let relative_path = path.trim_start_matches('/');
        let relative_path = if relative_path.is_empty() {
            "index.html"
        } else {
            relative_path
        };
        if relative_path.contains('\0') {
            return StatusCode::BAD_REQUEST.into_response();
        }

        let app_root = match tokio::fs::canonicalize(&app_dir).await {
            Ok(path) => path,
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        };

        let mut canonical_file = match tokio::fs::canonicalize(app_dir.join(relative_path)).await {
            Ok(path) => path,
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        };
        if !canonical_file.starts_with(&app_root) {
            return StatusCode::FORBIDDEN.into_response();
        }

        if tokio::fs::metadata(&canonical_file)
            .await
            .map(|m| m.is_dir())
            .unwrap_or(false)
        {
            let index_path = canonical_file.join("index.html");
            let index_canonical = match tokio::fs::canonicalize(index_path).await {
                Ok(path) => path,
                Err(_) => return StatusCode::NOT_FOUND.into_response(),
            };
            if !index_canonical.starts_with(&app_root) {
                return StatusCode::FORBIDDEN.into_response();
            }
            canonical_file = index_canonical;
        }

        match tokio::fs::read(&canonical_file).await {
            Ok(bytes) => {
                let filename = canonical_file
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("index.html");
                let content_type = guess_content_type(filename);
                let mut response_bytes = bytes;
                if should_upgrade_insecure_links(&content_type) {
                    let mut rewritten = String::from_utf8_lossy(&response_bytes).into_owned();
                    rewritten = rewrite_external_proxy_urls_for_public_apps(&rewritten, app_id);
                    if content_type.to_ascii_lowercase().starts_with("text/html") {
                        rewritten = inject_app_runtime_fetch_shims(&rewritten, app_id);
                    }
                    if is_secure_origin_request(&headers) {
                        rewritten = upgrade_http_links_for_secure_origin(&rewritten);
                    }
                    response_bytes = rewritten.into_bytes();
                }
                if method == Method::HEAD {
                    (
                        StatusCode::OK,
                        [
                            (header::CONTENT_TYPE, content_type),
                            (header::CACHE_CONTROL, "no-store".to_string()),
                        ],
                        Vec::<u8>::new(),
                    )
                        .into_response()
                } else {
                    (
                        StatusCode::OK,
                        [
                            (header::CONTENT_TYPE, content_type),
                            (header::CACHE_CONTROL, "no-store".to_string()),
                        ],
                        response_bytes,
                    )
                        .into_response()
                }
            }
            Err(_) => StatusCode::NOT_FOUND.into_response(),
        }
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

pub(super) const APP_GUARD_PAGE_STYLE: &str = r#"
*{margin:0;padding:0;box-sizing:border-box}
:root{color-scheme:dark}
body{
  min-height:100dvh;
  display:grid;
  place-items:center;
  padding:24px;
  font-family:"IBM Plex Sans","Inter","Segoe UI",system-ui,sans-serif;
  color:#eff7ef;
  background:
    linear-gradient(rgb(255 255 255 / .018) 1px, transparent 1px),
    linear-gradient(90deg, rgb(255 255 255 / .014) 1px, transparent 1px),
    radial-gradient(circle at 50% 0%, rgb(0 255 170 / .07), transparent 34%),
    #030504;
  background-size:44px 44px,44px 44px,auto,auto;
}
.card{
  width:min(430px,100%);
  border:1px solid rgb(0 255 170 / .12);
  border-radius:8px;
  background:linear-gradient(180deg,#0d1310,#070b09);
  box-shadow:0 18px 44px rgb(0 0 0 / .42), inset 0 1px 0 rgb(255 255 255 / .03);
  padding:24px;
}
.eyebrow{
  margin-bottom:8px;
  color:#ffbe63;
  font:600 11px/1.4 "JetBrains Mono",ui-monospace,SFMono-Regular,Consolas,monospace;
  text-transform:uppercase;
}
h1{
  margin-bottom:8px;
  color:#eff7ef;
  font:600 20px/1.28 "JetBrains Mono",ui-monospace,SFMono-Regular,Consolas,monospace;
  letter-spacing:0;
}
p{
  color:rgb(213 216 223 / .72);
  font-size:14px;
  line-height:1.55;
}
form{margin-top:20px}
label{
  display:block;
  margin-bottom:8px;
  color:rgb(184 191 201 / .78);
  font:500 12px/1.4 "JetBrains Mono",ui-monospace,SFMono-Regular,Consolas,monospace;
}
input{
  width:100%;
  min-height:44px;
  padding:11px 12px;
  border-radius:8px;
  border:1px solid rgb(0 255 170 / .14);
  background:#030504;
  color:#eff7ef;
  font:400 16px/1.4 "IBM Plex Sans","Inter","Segoe UI",system-ui,sans-serif;
  outline:none;
  transition:border-color .18s ease,box-shadow .18s ease,background .18s ease;
}
input::placeholder{color:rgb(184 191 201 / .46)}
input:focus{
  border-color:rgb(0 255 170 / .38);
  box-shadow:0 0 0 2px rgb(0 255 170 / .12),0 0 18px rgb(0 255 170 / .08);
  background:#050806;
}
button{
  width:100%;
  min-height:44px;
  margin-top:12px;
  border:1px solid rgb(0 255 170 / .18);
  border-radius:8px;
  background:linear-gradient(180deg,#163d31,#0b241d);
  color:#f3f7fb;
  font:600 13px/1.2 "JetBrains Mono",ui-monospace,SFMono-Regular,Consolas,monospace;
  cursor:pointer;
  box-shadow:0 4px 10px rgb(0 0 0 / .30);
  transition:background .18s ease,border-color .18s ease,box-shadow .18s ease,transform .18s ease;
}
button:hover{
  background:linear-gradient(180deg,#1d5140,#0f3027);
  border-color:rgb(0 255 170 / .28);
  box-shadow:0 0 0 1px rgb(0 255 170 / .08),0 10px 24px rgb(0 0 0 / .40);
}
button:focus-visible{
  outline:none;
  box-shadow:0 0 0 2px rgb(0 255 170 / .18),0 0 18px rgb(0 255 170 / .10);
}
button:active{transform:scale(.98)}
strong{color:#eff7ef}
"#;

/// Access denied page for apps with invalid/missing access password.
pub(super) fn app_access_denied_response(
    app_id: &str,
    path: &str,
    headers: &axum::http::HeaderMap,
) -> Response {
    if should_render_app_guard_document(path, headers) {
        return app_access_denied_page(app_id);
    }
    (
        StatusCode::UNAUTHORIZED,
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        "Access password required",
    )
        .into_response()
}

pub(super) fn app_access_denied_page(app_id: &str) -> Response {
    let html = format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>AgentArk App Guard</title>
<style>{style}</style></head>
<body><div class="card">
<div class="eyebrow">AgentArk App Guard</div>
<h1>Access Password Required</h1>
<p>This app is protected. Enter the access password to continue.</p>
<form method="GET" action="/apps/{app_id}/">
<label for="password">Access password</label>
<input id="password" type="password" name="password" placeholder="Enter access password" autocomplete="current-password" autofocus required>
<button type="submit">Unlock</button>
</form>
</div></body></html>"#,
        app_id = app_id,
        style = APP_GUARD_PAGE_STYLE
    );
    Html(html).into_response()
}

pub(super) fn app_public_exposure_requires_guard_response(
    app_id: &str,
    path: &str,
    headers: &axum::http::HeaderMap,
) -> Response {
    if should_render_app_guard_document(path, headers) {
        return app_public_exposure_requires_guard_page(app_id);
    }
    (
        StatusCode::FORBIDDEN,
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        "Public app access requires App Guard",
    )
        .into_response()
}

pub(super) fn app_public_exposure_requires_guard_page(app_id: &str) -> Response {
    let html = format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Public Access Disabled</title>
<style>{style}</style></head>
<body><div class="card">
<div class="eyebrow">AgentArk App Guard</div>
<h1>Public Access Disabled</h1>
<p>App <strong>{app_id}</strong> cannot be served on a public origin until App Guard is enabled with an access password.</p>
</div></body></html>"#,
        app_id = app_id,
        style = APP_GUARD_PAGE_STYLE
    );
    (StatusCode::FORBIDDEN, Html(html)).into_response()
}

fn should_render_app_guard_document(path: &str, headers: &axum::http::HeaderMap) -> bool {
    let normalized = path.trim_start_matches('/');
    if normalized.is_empty() || normalized.ends_with('/') {
        return true;
    }
    let extension = normalized
        .rsplit_once('/')
        .map(|(_, name)| name)
        .unwrap_or(normalized)
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase());
    if matches!(extension.as_deref(), Some("html" | "htm")) {
        return true;
    }
    if extension.is_some() {
        return false;
    }
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .map(|accept| {
            accept
                .split(',')
                .any(|part| part.trim().starts_with("text/html"))
        })
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_guard_renders_document_for_app_root() {
        let headers = axum::http::HeaderMap::new();
        assert!(should_render_app_guard_document("", &headers));
    }

    #[test]
    fn app_guard_does_not_return_html_for_stylesheet_request() {
        let headers = axum::http::HeaderMap::new();
        assert!(!should_render_app_guard_document("styles.css", &headers));
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct AppAccessGuardUpdateRequest {
    enabled: bool,
    #[serde(default)]
    regenerate_key: bool,
    #[serde(default)]
    access_password: Option<String>,
}

/// Disable an app and stop its runtime if it has one.
pub(super) async fn stop_app(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
) -> Response {
    if !is_valid_app_id(&app_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid app_id" })),
        )
            .into_response();
    }
    if state.app_registry.get_dir(&app_id).await.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "App not found" })),
        )
            .into_response();
    }
    let is_static = state.app_registry.is_static(&app_id).await;
    let stop_result = if is_static {
        Ok(())
    } else if let Some(executor) = state.executor_client.as_ref() {
        executor
            .request(
                reqwest::Method::POST,
                &format!("/internal/v1/apps/{}/stop", app_id),
            )
            .json(&crate::clients::AppLifecycleRequest {
                title: None,
                query: None,
            })
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map(|_| ())
            .map_err(anyhow::Error::from)
    } else {
        state.app_registry.stop_runtime(&app_id).await
    };
    match stop_result {
        Ok(_) => match state.app_registry.set_enabled(&app_id, false).await {
            Ok(_) => {
                trigger_arkpulse_after_app_change(&state, "app_disable").await;
                (
                    StatusCode::OK,
                    Json(serde_json::json!({ "status": "disabled", "app_id": app_id })),
                )
                    .into_response()
            }
            Err(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": error.to_string() })),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub(super) async fn update_app_access_guard(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    Json(request): Json<AppAccessGuardUpdateRequest>,
) -> Response {
    if !is_valid_app_id(&app_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid app_id" })),
        )
            .into_response();
    }

    match state
        .app_registry
        .set_access_guard(
            &app_id,
            request.enabled,
            request.access_password.as_deref(),
            request.regenerate_key,
        )
        .await
    {
        Ok(access_key) => {
            trigger_arkpulse_after_app_change(&state, "app_access_guard_update").await;
            let access_url = app_access_url_for_state(&state, &app_id, request.enabled).await;
            let access_password = if request.enabled {
                access_key.clone()
            } else {
                String::new()
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "app_id": app_id,
                    "access_guard_enabled": request.enabled,
                    "access_key": if request.enabled { access_key } else { String::new() },
                    "access_password": access_password,
                    "access_url": access_url,
                })),
            )
                .into_response()
        }
        Err(e) => {
            let status = if e.to_string() == "App not found" {
                StatusCode::NOT_FOUND
            } else if e.to_string().contains("Access password")
                || e.to_string()
                    .contains("Public apps must keep App Guard enabled")
            {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(serde_json::json!({ "error": e.to_string() }))).into_response()
        }
    }
}

/// Restart an app from saved metadata
pub(super) async fn restart_app(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
) -> Response {
    if !is_valid_app_id(&app_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid app_id" })),
        )
            .into_response();
    }

    let app_dir = if let Some(path) = state.app_registry.get_dir(&app_id).await {
        path
    } else {
        let data_dir = {
            let agent = state.agent.read().await;
            agent.data_dir().to_path_buf()
        };
        let fallback = data_dir.join("apps").join(&app_id);
        if !fallback.exists() {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "App not found" })),
            )
                .into_response();
        }
        fallback
    };

    if let Some(executor) = state.executor_client.as_ref() {
        match executor
            .request(
                reqwest::Method::POST,
                &format!("/internal/v1/apps/{}/restart", app_id),
            )
            .json(&crate::clients::AppLifecycleRequest {
                title: None,
                query: None,
            })
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status();
                let payload = response
                    .json::<serde_json::Value>()
                    .await
                    .unwrap_or_else(|_| serde_json::json!({}));
                if !status.is_success() {
                    return (
                        StatusCode::from_u16(status.as_u16())
                            .unwrap_or(StatusCode::BAD_GATEWAY),
                        Json(serde_json::json!({
                            "error": payload.get("message").and_then(|value| value.as_str()).unwrap_or("Failed to restart app"),
                            "details": payload
                        })),
                    )
                        .into_response();
                }
                let _ = state.app_registry.set_enabled(&app_id, true).await;
                trigger_arkpulse_after_app_change(&state, "app_restart").await;
                let raw = payload
                    .get("raw")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                let title = raw
                    .get("title")
                    .and_then(|value| value.as_str())
                    .unwrap_or(&app_id);
                let access_guard_enabled = raw
                    .get("access_guard_enabled")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                let app_url =
                    build_absolute_app_url(state.public_app_base_url.as_deref(), &app_id, "", None);
                let app_access_url =
                    app_access_url_for_state(&state, &app_id, access_guard_enabled).await;
                return Json(serde_json::json!({
                    "status": "restarted",
                    "type": raw.get("mode").cloned().unwrap_or_else(|| serde_json::json!("dynamic")),
                    "app_id": app_id,
                    "title": title,
                    "url": app_url,
                    "access_url": app_access_url,
                    "access_guard_enabled": access_guard_enabled,
                    "port": raw.get("port").cloned().unwrap_or(serde_json::Value::Null),
                    "runtime_preference": raw.get("runtime_mode").cloned().unwrap_or_else(|| serde_json::json!("executor")),
                    "details": payload,
                }))
                .into_response();
            }
            Err(error) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({
                        "error": format!("Failed to restart app through executor: {}", error)
                    })),
                )
                    .into_response();
            }
        }
    }

    let meta_path = app_dir.join(".app_meta.json");
    let mut meta: serde_json::Value = match tokio::fs::read(&meta_path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({})),
        Err(_) => serde_json::json!({}),
    };
    if !meta.is_object() {
        meta = serde_json::json!({});
    }

    let title = meta
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or(&app_id)
        .to_string();
    let entry_command = meta
        .as_object()
        .and_then(|_| crate::actions::app::app_meta_lifecycle_command(&meta, "entry_command"));
    let install_command = meta
        .as_object()
        .and_then(|_| crate::actions::app::app_meta_lifecycle_command(&meta, "install_command"));
    let runtime_image = meta
        .get("runtime_image")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let runtime_preference = crate::actions::app::runtime_preference_from_opt(
        meta.get("runtime_preference").and_then(|v| v.as_str()),
    );
    let required_inputs = crate::actions::app::parse_required_inputs(&meta);
    let config_values: std::collections::HashMap<String, String> = meta
        .get("config_values")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| {
                    let value = match v {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        serde_json::Value::Number(n) => n.to_string(),
                        _ => return None,
                    };
                    Some((k.clone(), value))
                })
                .collect()
        })
        .unwrap_or_default();
    let access_guard_enabled = meta
        .get("access_guard_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let access_key = if access_guard_enabled {
        state
            .app_registry
            .access_key(&app_id)
            .await
            .unwrap_or_else(crate::actions::app::generate_access_key)
    } else {
        String::new()
    };

    let mut lifecycle_meta_dirty = false;
    let stop_command = crate::actions::app::app_meta_lifecycle_command(&meta, "stop_command");
    let mut commands = meta
        .get("commands")
        .cloned()
        .filter(|value| value.is_object())
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = meta.as_object_mut() {
        if let Some(command) = entry_command.as_ref() {
            obj.insert(
                "entry_command".to_string(),
                serde_json::Value::String(command.clone()),
            );
            obj.insert(
                "start_command".to_string(),
                serde_json::Value::String(command.clone()),
            );
        }
        if let Some(command) = install_command.as_ref() {
            obj.insert(
                "install_command".to_string(),
                serde_json::Value::String(command.clone()),
            );
        }
        if let Some(command) = entry_command.as_ref() {
            commands["start"] = serde_json::Value::String(command.clone());
        }
        if let Some(command) = install_command.as_ref() {
            commands["install"] = serde_json::Value::String(command.clone());
        }
        if let Some(command) = stop_command.as_ref() {
            commands["stop"] = serde_json::Value::String(command.clone());
            obj.insert(
                "stop_command".to_string(),
                serde_json::Value::String(command.clone()),
            );
        }
        obj.insert("commands".to_string(), commands);
        lifecycle_meta_dirty = true;
    }
    if lifecycle_meta_dirty {
        let _ = tokio::fs::write(
            &meta_path,
            serde_json::to_vec_pretty(&meta).unwrap_or_default(),
        )
        .await;
    }

    if meta.get("access_guard_enabled").is_none() || meta.get("access_key").is_some() {
        meta["access_guard_enabled"] = serde_json::Value::Bool(access_guard_enabled);
        if let Some(obj) = meta.as_object_mut() {
            obj.remove("access_key");
        }
        let _ = tokio::fs::write(
            &meta_path,
            serde_json::to_vec_pretty(&meta).unwrap_or_default(),
        )
        .await;
    }
    if meta.get("enabled").and_then(|v| v.as_bool()) != Some(true) {
        meta["enabled"] = serde_json::Value::Bool(true);
        let _ = tokio::fs::write(
            &meta_path,
            serde_json::to_vec_pretty(&meta).unwrap_or_default(),
        )
        .await;
    }
    let _ = state.app_registry.set_enabled(&app_id, true).await;

    if let Err(e) = state.app_registry.stop_runtime(&app_id).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to stop running app before restart: {}", e) })),
        )
            .into_response();
    }

    if let Some(entry_command) = entry_command {
        let Some(port) = state.app_registry.find_available_port().await else {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": "No available app port" })),
            )
                .into_response();
        };

        let (config_dir, data_dir, llm_env) = {
            let agent = state.agent.read().await;
            (
                agent.config_dir.clone(),
                agent.data_dir().to_path_buf(),
                agent.app_model_env_vars(),
            )
        };
        let (resolved_env, missing_sensitive, missing_config) =
            match crate::actions::app::resolve_required_env_values(
                &config_dir,
                &data_dir,
                &required_inputs,
                &llm_env,
                &config_values,
            )
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Failed to resolve app secrets: {}", e) })),
                )
                    .into_response();
                }
            };
        if !missing_sensitive.is_empty() || !missing_config.is_empty() {
            let mut missing_all = missing_sensitive.clone();
            for m in &missing_config {
                if !missing_all.iter().any(|x| x == m) {
                    missing_all.push(m.clone());
                }
            }
            let required_secret_keys: Vec<String> = required_inputs
                .iter()
                .filter(|r| r.sensitive)
                .map(|r| r.key.clone())
                .collect();
            let required_config_keys: Vec<String> = required_inputs
                .iter()
                .filter(|r| !r.sensitive)
                .map(|r| r.key.clone())
                .collect();
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "status": "needs_secrets",
                    "app_id": app_id,
                    "missing_env": missing_sensitive,
                    "missing_config": missing_config,
                    "missing_inputs": missing_all,
                    "required_inputs": required_inputs,
                    "required_secrets": required_secret_keys.clone(),
                    "required_env": required_secret_keys,
                    "required_config": required_config_keys,
                    "message": "Missing required inputs. Use the secure credential form in chat or Settings for sensitive values; provide config for non-sensitive values."
                })),
            )
                .into_response();
        }

        match crate::actions::app::launch_dynamic_runtime(
            crate::actions::app::DynamicRuntimeLaunch {
                app_id: &app_id,
                app_dir: &app_dir,
                entry_command: &entry_command,
                install_command: install_command.as_deref(),
                port,
                extra_env: &resolved_env,
                runtime_image: runtime_image.as_deref(),
                runtime_preference,
                stream_tx: None,
            },
        )
        .await
        {
            Ok(runtime_handle) => {
                let (child, container_id) = match runtime_handle {
                    crate::actions::app::DynamicRuntimeHandle::Container(container_id) => {
                        (None, Some(container_id))
                    }
                    crate::actions::app::DynamicRuntimeHandle::Process(child) => {
                        (Some(*child), None)
                    }
                };
                let app_dir_for_diagnostics = app_dir.clone();
                state
                    .app_registry
                    .register_dynamic(
                        app_id.clone(),
                        crate::actions::app::DynamicAppRegistration {
                            title: title.clone(),
                            app_dir,
                            child,
                            container_id,
                            port,
                            access_key: access_key.clone(),
                            access_guard_enabled,
                            expose_public: meta
                                .get("expose_public")
                                .and_then(|value| value.as_bool())
                                .unwrap_or(false),
                            enabled: true,
                            last_accessed: None,
                        },
                    )
                    .await;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                if !state.app_registry.runtime_is_alive(&app_id).await {
                    let logs = crate::actions::app::read_local_runtime_log_tail(
                        &app_dir_for_diagnostics,
                        4096,
                    )
                    .await;
                    let detail = if logs.is_empty() {
                        "App process stopped shortly after restart.".to_string()
                    } else {
                        format!(
                            "App process stopped shortly after restart. Recent runtime logs:\n{}",
                            logs
                        )
                    };
                    return (
                        StatusCode::BAD_GATEWAY,
                        Json(serde_json::json!({ "error": detail })),
                    )
                        .into_response();
                }
                trigger_arkpulse_after_app_change(&state, "app_restart").await;
                let app_url =
                    build_absolute_app_url(state.public_app_base_url.as_deref(), &app_id, "", None);
                let app_access_url =
                    app_access_url_for_state(&state, &app_id, access_guard_enabled).await;
                Json(serde_json::json!({
                    "status": "restarted",
                    "type": "dynamic",
                    "app_id": app_id,
                    "title": title,
                    "url": app_url,
                    "access_url": app_access_url,
                    "access_key": access_key,
                    "access_password": access_key,
                    "access_guard_enabled": access_guard_enabled,
                    "port": port,
                    "runtime_preference": runtime_preference.as_str(),
                }))
                .into_response()
            }
            Err(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to restart app: {}", error) })),
            )
                .into_response(),
        }
    } else {
        state
            .app_registry
            .register_stored(
                app_id.clone(),
                crate::actions::app::StoredAppRegistration {
                    title: title.clone(),
                    app_dir,
                    is_static: true,
                    access_key: access_key.clone(),
                    access_guard_enabled,
                    expose_public: meta
                        .get("expose_public")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false),
                    enabled: true,
                    last_accessed: None,
                },
            )
            .await;
        trigger_arkpulse_after_app_change(&state, "app_restart").await;
        let app_url =
            build_absolute_app_url(state.public_app_base_url.as_deref(), &app_id, "", None);
        let app_access_url = app_access_url_for_state(&state, &app_id, access_guard_enabled).await;
        Json(serde_json::json!({
            "status": "restarted",
            "type": "static",
            "app_id": app_id,
            "title": title,
            "url": app_url,
            "access_url": app_access_url,
            "access_key": access_key,
            "access_password": access_key,
            "access_guard_enabled": access_guard_enabled,
            "runtime_preference": runtime_preference.as_str(),
        }))
        .into_response()
    }
}

/// Stop and delete an app from disk
pub(super) async fn delete_app(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
) -> Response {
    if !is_valid_app_id(&app_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid app_id" })),
        )
            .into_response();
    }

    let app_title: Option<String> = {
        let apps = state.app_registry.list().await;
        apps.iter()
            .find(|row| row.get("id").and_then(|v| v.as_str()) == Some(app_id.as_str()))
            .and_then(|row| row.get("title").and_then(|v| v.as_str()))
            .map(|s| s.to_string())
    };

    let app_dir = if let Some(path) = state.app_registry.get_dir(&app_id).await {
        path
    } else {
        let data_dir = {
            let agent = state.agent.read().await;
            agent.data_dir().to_path_buf()
        };
        data_dir.join("apps").join(&app_id)
    };

    if let Some(executor) = state.executor_client.as_ref() {
        match executor
            .request(
                reqwest::Method::DELETE,
                &format!("/internal/v1/apps/{}", app_id),
            )
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {}
            Ok(response) if response.status() == reqwest::StatusCode::NOT_FOUND => {}
            Ok(response) => {
                let status = StatusCode::from_u16(response.status().as_u16())
                    .unwrap_or(StatusCode::BAD_GATEWAY);
                let payload = response
                    .json::<serde_json::Value>()
                    .await
                    .unwrap_or_else(|_| serde_json::json!({}));
                return (
                    status,
                    Json(serde_json::json!({
                        "error": payload.get("message").and_then(|value| value.as_str()).unwrap_or("Failed to stop app before delete"),
                        "details": payload
                    })),
                )
                    .into_response();
            }
            Err(error) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({
                        "error": format!("Failed to contact executor before delete: {}", error)
                    })),
                )
                    .into_response();
            }
        }
    } else if let Err(e) = state.app_registry.stop(&app_id).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({ "error": format!("Failed to stop app before delete: {}", e) }),
            ),
        )
            .into_response();
    }
    match tokio::fs::remove_dir_all(&app_dir).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to delete app files: {}", error) })),
            )
                .into_response();
        }
    }

    let cleanup = cleanup_deleted_app_references(&state, &app_id, app_title.as_deref()).await;
    trigger_arkpulse_after_app_change(&state, "app_delete").await;
    Json(serde_json::json!({
        "status": "deleted",
        "app_id": app_id,
        "deleted_notifications": cleanup.deleted_notifications,
        "deleted_pulse_events": cleanup.deleted_pulse_events
    }))
    .into_response()
}

/// Upload a file for use in chat (attachments for code execution, analysis, etc.)
pub(super) async fn upload_chat_file(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Response {
    let (data_dir, storage) = {
        let agent = state.agent.read().await;
        (agent.data_dir().to_path_buf(), agent.storage.clone())
    };
    let uploads_dir = data_dir.join("uploads");

    let mut uploaded_files = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let original_name: String = field.file_name().unwrap_or("unnamed").to_string();
        let content_type = field.content_type().map(|value| value.to_string());

        // Sanitize filename: keep only safe characters
        let safe_name: String = original_name
            .chars()
            .map(|c: char| {
                if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();

        // Prevent path traversal
        if safe_name.contains("..") || safe_name.starts_with('.') {
            return (StatusCode::BAD_REQUEST, "Invalid filename").into_response();
        }

        match field.bytes().await {
            Ok(data) => {
                // 50MB limit per file
                if data.len() > 50 * 1024 * 1024 {
                    return (StatusCode::PAYLOAD_TOO_LARGE, "File too large (50MB max)")
                        .into_response();
                }

                if let Err(e) = tokio::fs::create_dir_all(&uploads_dir).await {
                    tracing::error!("Failed to create uploads dir: {}", e);
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }

                let upload_id = uuid::Uuid::new_v4().to_string();
                let stored_name = format!("{}__{}", upload_id, safe_name);
                let file_path = uploads_dir.join(&stored_name);
                if let Err(e) = tokio::fs::write(&file_path, &data).await {
                    tracing::error!("Failed to write upload: {}", e);
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }

                let manifest = crate::storage::UploadManifest {
                    id: upload_id.clone(),
                    original_name: safe_name.clone(),
                    stored_name,
                    content_type,
                    size_bytes: data.len() as u64,
                    created_at: chrono::Utc::now().to_rfc3339(),
                };
                if let Err(error) = storage.save_upload_manifest(&manifest).await {
                    let _ = tokio::fs::remove_file(&file_path).await;
                    tracing::error!("Failed to persist upload manifest: {}", error);
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
                if let Some(workspace) = state.workspace_client.as_ref() {
                    if let Err(error) = workspace
                        .put_blob(&format!("uploads/{}", manifest.stored_name), &data)
                        .await
                    {
                        tracing::warn!(
                            "Failed to mirror upload {} to workspace service: {}",
                            manifest.id,
                            error
                        );
                    }
                }

                tracing::info!("File uploaded: {} ({} bytes)", safe_name, data.len());
                uploaded_files.push(serde_json::json!({
                    "id": upload_id,
                    "name": safe_name,
                    "size": data.len(),
                    "path": format!("/api/uploads/{}", manifest.id),
                }));
            }
            Err(e) => {
                tracing::error!("Failed to read upload field: {}", e);
                return StatusCode::BAD_REQUEST.into_response();
            }
        }
    }

    if uploaded_files.is_empty() {
        return (StatusCode::BAD_REQUEST, "No files uploaded").into_response();
    }

    Json(serde_json::json!({ "files": uploaded_files })).into_response()
}

/// Serve uploaded files (for preview/download in chat)
pub(super) async fn serve_upload_file(
    State(state): State<AppState>,
    Path(upload_id): Path<String>,
) -> Response {
    let normalized_id = match uuid::Uuid::parse_str(upload_id.trim()) {
        Ok(value) => value.to_string(),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    let (data_dir, storage) = {
        let agent = state.agent.read().await;
        (agent.data_dir().to_path_buf(), agent.storage.clone())
    };
    let manifest = match storage.load_upload_manifest(&normalized_id).await {
        Ok(Some(value)) => value,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::error!(
                "Failed to load upload manifest {}: {}",
                normalized_id,
                error
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let uploads_dir = data_dir.join("uploads");
    let mut bytes = if let Ok(uploads_root) = tokio::fs::canonicalize(&uploads_dir).await {
        match tokio::fs::canonicalize(uploads_root.join(&manifest.stored_name)).await {
            Ok(file_path) if file_path.starts_with(&uploads_root) => {
                tokio::fs::read(&file_path).await.ok()
            }
            Ok(_) => return StatusCode::BAD_REQUEST.into_response(),
            Err(_) => None,
        }
    } else {
        None
    };
    if bytes.is_none() {
        if let Some(workspace) = state.workspace_client.as_ref() {
            bytes = workspace
                .get_blob(&format!("uploads/{}", manifest.stored_name))
                .await
                .ok();
        }
    }

    match bytes {
        Some(bytes) => {
            let content_type = manifest
                .content_type
                .clone()
                .unwrap_or_else(|| guess_content_type(&manifest.original_name));
            let safe_filename = sanitize_content_disposition_filename(&manifest.original_name);
            (
                [
                    (header::CONTENT_TYPE, content_type.as_str()),
                    (
                        header::CONTENT_DISPOSITION,
                        &format!("inline; filename=\"{}\"", safe_filename),
                    ),
                ],
                bytes,
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
