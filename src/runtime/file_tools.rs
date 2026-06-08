use super::*;

impl ActionRuntime {
    pub(super) fn sanitize_upload_filename(raw: &str) -> String {
        let filename: String = raw
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if filename.is_empty() {
            "file".to_string()
        } else {
            filename
        }
    }

    pub(super) fn inline_code_execute_payloads(
        arguments: &serde_json::Value,
    ) -> Result<Vec<SandboxUploadFile>> {
        let Some(payloads) = arguments
            .get("file_payloads")
            .and_then(|value| value.as_array())
        else {
            return Ok(Vec::new());
        };
        let mut files = Vec::with_capacity(payloads.len());
        for payload in payloads {
            let filename = payload
                .get("filename")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("Each file_payload must include a filename"))?;
            let bytes_b64 = payload
                .get("bytes_b64")
                .and_then(|value| value.as_str())
                .ok_or_else(|| anyhow::anyhow!("Each file_payload must include bytes_b64"))?;
            let bytes =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, bytes_b64)
                    .map_err(|e| {
                        anyhow::anyhow!("Invalid base64 file payload for '{}': {}", filename, e)
                    })?;
            files.push(SandboxUploadFile {
                filename: Self::sanitize_upload_filename(filename),
                content_type: payload
                    .get("content_type")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                bytes,
            });
        }
        Ok(files)
    }

    pub(super) async fn collect_code_execute_files(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<Vec<SandboxUploadFile>> {
        let inline = Self::inline_code_execute_payloads(arguments)?;
        if !inline.is_empty() {
            return Ok(inline);
        }
        let mut files = Vec::new();
        if let Some(files_arr) = arguments.get("files").and_then(|v| v.as_array()) {
            for file_val in files_arr {
                if let Some(upload_id) = file_val
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    match self.resolve_upload_for_sandbox(upload_id).await {
                        Ok(file) => {
                            files.push(file);
                            continue;
                        }
                        Err(upload_error) => {
                            let resource = Self::runtime_resource_from_argument(file_val)
                                .ok_or(upload_error)?;
                            files.push(self.sandbox_upload_from_resource(resource).await?);
                            continue;
                        }
                    }
                }
                let resource = Self::runtime_resource_from_argument(file_val).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Each code_execute file reference must be an upload ID, ResourceRef, resource path, or structured resource payload"
                    )
                })?;
                files.push(self.sandbox_upload_from_resource(resource).await?);
            }
        }
        Ok(files)
    }

    pub(super) fn upload_signature(
        filename: &str,
        content_type: Option<&str>,
        bytes: &[u8],
    ) -> serde_json::Value {
        let lower_name = filename.to_ascii_lowercase();
        let lower_ct = content_type.unwrap_or("").to_ascii_lowercase();
        let ext = lower_name
            .rsplit_once('.')
            .map(|(_, ext)| ext)
            .unwrap_or("");

        let mut detected = if bytes.starts_with(b"OggS") {
            if bytes
                .windows(b"OpusHead".len())
                .any(|win| win == b"OpusHead")
            {
                serde_json::json!({
                    "input_type": "audio",
                    "media_kind": "audio",
                    "mime": "audio/ogg; codecs=opus",
                    "extension": "opus",
                    "confidence": "high",
                    "source": "magic_bytes",
                })
            } else {
                serde_json::json!({
                    "input_type": "audio",
                    "media_kind": "audio",
                    "mime": "audio/ogg",
                    "extension": "ogg",
                    "confidence": "high",
                    "source": "magic_bytes",
                })
            }
        } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
            serde_json::json!({
                "input_type": "audio",
                "media_kind": "audio",
                "mime": "audio/wav",
                "extension": "wav",
                "confidence": "high",
                "source": "magic_bytes",
            })
        } else if bytes.starts_with(b"ID3") || bytes.starts_with(&[0xFF, 0xFB]) {
            serde_json::json!({
                "input_type": "audio",
                "media_kind": "audio",
                "mime": "audio/mpeg",
                "extension": "mp3",
                "confidence": "high",
                "source": "magic_bytes",
            })
        } else if bytes.starts_with(b"fLaC") {
            serde_json::json!({
                "input_type": "audio",
                "media_kind": "audio",
                "mime": "audio/flac",
                "extension": "flac",
                "confidence": "high",
                "source": "magic_bytes",
            })
        } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
            let brand =
                String::from_utf8_lossy(&bytes[8..bytes.len().min(24)]).to_ascii_lowercase();
            let audio_brand =
                brand.contains("m4a") || brand.contains("m4b") || brand.contains("mp42");
            serde_json::json!({
                "input_type": if audio_brand { "audio" } else { "audio_video" },
                "media_kind": if audio_brand { "audio" } else { "audio_or_video" },
                "mime": if audio_brand { "audio/mp4" } else { "video/mp4" },
                "extension": if audio_brand { "m4a" } else { "mp4" },
                "confidence": "medium",
                "source": "magic_bytes",
            })
        } else if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
            serde_json::json!({
                "input_type": "audio_video",
                "media_kind": "audio_or_video",
                "mime": "video/webm",
                "extension": "webm",
                "confidence": "medium",
                "source": "magic_bytes",
            })
        } else if bytes.starts_with(b"\x89PNG\r\n\x1A\n") {
            serde_json::json!({
                "input_type": "image",
                "media_kind": "image",
                "mime": "image/png",
                "extension": "png",
                "confidence": "high",
                "source": "magic_bytes",
            })
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            serde_json::json!({
                "input_type": "image",
                "media_kind": "image",
                "mime": "image/jpeg",
                "extension": "jpg",
                "confidence": "high",
                "source": "magic_bytes",
            })
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            serde_json::json!({
                "input_type": "image",
                "media_kind": "image",
                "mime": "image/gif",
                "extension": "gif",
                "confidence": "high",
                "source": "magic_bytes",
            })
        } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            serde_json::json!({
                "input_type": "image",
                "media_kind": "image",
                "mime": "image/webp",
                "extension": "webp",
                "confidence": "high",
                "source": "magic_bytes",
            })
        } else if bytes.starts_with(b"%PDF-") {
            serde_json::json!({
                "input_type": "document",
                "media_kind": "document",
                "mime": "application/pdf",
                "extension": "pdf",
                "confidence": "high",
                "source": "magic_bytes",
            })
        } else if bytes.starts_with(b"PK\x03\x04") {
            serde_json::json!({
                "input_type": "archive",
                "media_kind": "archive",
                "mime": "application/zip",
                "extension": "zip",
                "confidence": "medium",
                "source": "magic_bytes",
            })
        } else {
            serde_json::json!({
                "input_type": "unknown",
                "media_kind": "unknown",
                "mime": serde_json::Value::Null,
                "extension": ext,
                "confidence": "low",
                "source": "unresolved",
                "needs_deeper_inspection": true,
            })
        };

        if let Some(obj) = detected.as_object_mut() {
            obj.insert("filename".to_string(), serde_json::json!(filename));
            obj.insert("size_bytes".to_string(), serde_json::json!(bytes.len()));
            if let Some(content_type) = content_type {
                obj.insert(
                    "provided_content_type".to_string(),
                    serde_json::json!(content_type),
                );
            }
            if !lower_ct.is_empty() {
                obj.insert(
                    "provided_content_type_hint".to_string(),
                    serde_json::json!(lower_ct),
                );
            }
        }
        detected
    }

    pub(super) fn sanitize_missing_binary_candidate(raw: &str) -> Option<String> {
        let candidate = raw
            .trim()
            .trim_matches(|ch: char| {
                !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_' && ch != '.' && ch != '+'
            })
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("")
            .trim();
        if candidate.is_empty()
            || candidate.len() > 80
            || !candidate
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '+'))
        {
            return None;
        }
        Some(candidate.to_string())
    }

    pub(super) fn quoted_missing_binary_candidate(line: &str) -> Option<String> {
        for quote in ['\'', '"'] {
            let mut parts = line.split(quote);
            while let Some(_) = parts.next() {
                let Some(candidate) = parts.next() else {
                    break;
                };
                if let Some(cleaned) = Self::sanitize_missing_binary_candidate(candidate) {
                    return Some(cleaned);
                }
            }
        }
        None
    }

    pub(super) fn detect_missing_binary_from_output(output: &str) -> Option<String> {
        let lower = output.to_ascii_lowercase();
        if let Some(idx) = lower.find("agentark_missing_binary:") {
            let raw = output[idx + "AGENTARK_MISSING_BINARY:".len()..]
                .lines()
                .next()
                .unwrap_or("")
                .trim();
            if let Some(candidate) = Self::sanitize_missing_binary_candidate(
                raw.split_whitespace().next().unwrap_or(raw),
            ) {
                return Some(candidate);
            }
        }

        for line in output.lines() {
            let lower_line = line.to_ascii_lowercase();
            for pattern in [": command not found", ": not found"] {
                if let Some(idx) = lower_line.find(pattern) {
                    let prefix = line[..idx].trim();
                    let after_shell_prefix = prefix.rsplit(':').next().unwrap_or(prefix);
                    let candidate = after_shell_prefix
                        .split_whitespace()
                        .last()
                        .unwrap_or(after_shell_prefix);
                    if let Some(cleaned) = Self::sanitize_missing_binary_candidate(candidate) {
                        return Some(cleaned);
                    }
                }
            }

            if lower_line.contains("no such file or directory")
                || lower_line.contains("is not recognized")
            {
                if let Some(candidate) = Self::quoted_missing_binary_candidate(line) {
                    return Some(candidate);
                }
            }
        }
        None
    }

    pub(super) fn build_sandbox_transcription_code() -> &'static str {
        r#"import json
import pathlib
import shutil
import sys

data_dir = pathlib.Path("/data")
files = [p for p in data_dir.iterdir() if p.is_file()]
if not files:
    raise SystemExit("No uploaded audio file was injected into /data.")

input_path = files[0]
if shutil.which("ffmpeg") is None:
    print("AGENTARK_MISSING_BINARY: ffmpeg")
    raise SystemExit(127)

import whisper

model = whisper.load_model("base")
result = model.transcribe(str(input_path))
print(json.dumps({
    "input_file": input_path.name,
    "text": (result.get("text") or "").strip()
}, ensure_ascii=False))
"#
    }

    pub(super) fn control_plane_executor_client() -> Option<ExecutorClient> {
        let role = std::env::var("AGENTARK_STACK_ROLE")
            .ok()
            .map(|value| value.trim().to_ascii_lowercase());
        if !matches!(role.as_deref(), Some("control-plane" | "control")) {
            return None;
        }
        let client = ExecutorClient::new(ExecutorClientConfig::from_env()).ok()?;
        client.bearer_token()?;
        Some(client)
    }

    pub(super) async fn execute_code_remote(
        &self,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Result<String> {
        let language = arguments["language"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'language' argument"))?
            .to_string();
        let code = arguments["code"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'code' argument"))?
            .to_string();
        let env = arguments
            .get("env")
            .and_then(|value| value.as_object())
            .map(|map| {
                map.iter()
                    .filter_map(|(key, value)| {
                        value.as_str().map(|value| (key.clone(), value.to_string()))
                    })
                    .collect::<BTreeMap<String, String>>()
            })
            .unwrap_or_default();
        let network_access = arguments
            .get("network_access")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let execution_contract = arguments.get("execution_contract").cloned();
        let file_payloads = self
            .collect_code_execute_files(arguments)
            .await?
            .into_iter()
            .map(|file| CodeExecuteFilePayload {
                filename: file.filename,
                bytes_b64: base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    file.bytes,
                ),
            })
            .collect::<Vec<_>>();
        let executor = Self::control_plane_executor_client()
            .ok_or_else(|| anyhow::anyhow!("Executor service is not configured"))?;
        let response = executor
            .execute_code(&crate::clients::CodeExecuteRequest {
                language,
                code,
                files: Vec::new(),
                file_payloads,
                env,
                network_access,
                execution_contract,
                auth_context: Some(auth_context.clone()),
            })
            .await?;
        if response.status.eq_ignore_ascii_case("ok") {
            if response.raw.is_object() {
                return Ok(serde_json::to_string(&response.raw)?);
            }
            return Ok(serde_json::to_string(&serde_json::json!({
                "output": response.output_text.unwrap_or_default(),
                "error": serde_json::Value::Null,
                "exit_code": 0,
                "files": response.output_files,
            }))?);
        }
        if response.raw.is_object() {
            let error = response
                .raw
                .get("error")
                .and_then(|value| value.as_str())
                .unwrap_or(response.message.as_str());
            anyhow::bail!("{}", error);
        }
        anyhow::bail!("{}", response.message);
    }

    pub(super) fn remap_workspace_alias_path(&self, raw: &str) -> Option<PathBuf> {
        let trimmed = raw.trim();
        const PREFIXES: &[&str] = &["/workspace", "/repo", "/project"];
        let matched = PREFIXES.iter().find(|prefix| {
            trimmed == **prefix
                || trimmed
                    .strip_prefix(**prefix)
                    .is_some_and(|rest| rest.starts_with('/'))
        })?;
        let workspace_root = self.workspace_root();
        let suffix = trimmed.strip_prefix(matched).unwrap_or("");
        let relative = suffix.trim_start_matches('/');
        if relative.is_empty() {
            Some(workspace_root)
        } else {
            Some(workspace_root.join(relative))
        }
    }

    fn dedupe_allowed_roots(roots: Vec<PathBuf>) -> Vec<PathBuf> {
        let mut deduped = Vec::new();
        for root in roots {
            let candidate = root.canonicalize().unwrap_or(root);
            if !deduped
                .iter()
                .any(|existing: &PathBuf| existing == &candidate)
            {
                deduped.push(candidate);
            }
        }
        deduped
    }

    pub(super) fn allowed_write_file_roots(&self) -> Vec<PathBuf> {
        Self::dedupe_allowed_roots(vec![
            self.data_dir().to_path_buf(),
            self.actions_dir.clone(),
            self.config_dir.clone(),
            self.workspace_root(),
        ])
    }

    pub(super) fn allowed_file_roots(&self) -> Vec<PathBuf> {
        self.allowed_write_file_roots()
    }

    pub(super) fn absolutize_tool_path(&self, raw: &str) -> Result<PathBuf> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            anyhow::bail!("Path cannot be empty");
        }

        if let Some(remapped) = self.remap_workspace_alias_path(trimmed) {
            return Ok(remapped);
        }

        let path = PathBuf::from(trimmed);
        if path.is_absolute() {
            Ok(path)
        } else {
            Ok(self.workspace_root().join(path))
        }
    }

    pub(super) fn ensure_tool_path_allowed(&self, candidate: &Path) -> Result<()> {
        let allowed_roots = self.allowed_file_roots();
        if allowed_roots.iter().any(|root| candidate.starts_with(root)) {
            return Ok(());
        }
        Err(ToolPathAccessError::OutsideAllowedRoots {
            attempted_path: candidate.to_path_buf(),
            allowed_roots,
        }
        .into())
    }

    pub(super) fn ensure_tool_write_path_allowed(&self, candidate: &Path) -> Result<()> {
        let allowed_roots = self.allowed_write_file_roots();
        if allowed_roots.iter().any(|root| candidate.starts_with(root)) {
            return Ok(());
        }
        Err(ToolPathAccessError::OutsideAllowedRoots {
            attempted_path: candidate.to_path_buf(),
            allowed_roots,
        }
        .into())
    }

    pub(super) fn tool_path_looks_sensitive_file(path: &Path) -> bool {
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            return false;
        };
        let lower = name.trim().to_ascii_lowercase();
        lower == ".agentark_runtime_env"
            || lower == ".env"
            || lower.starts_with(".env.")
            || lower.ends_with(".pem")
            || lower.ends_with(".key")
            || lower.ends_with(".p12")
            || lower.ends_with(".pfx")
            || lower == "secrets.json"
            || lower == "credentials.json"
    }

    pub(super) fn ensure_tool_target_not_sensitive_file(path: &Path, verb: &str) -> Result<()> {
        if Self::tool_path_looks_sensitive_file(path) {
            anyhow::bail!(
                "Refusing to {} sensitive credential file '{}'. Use the secure credential store or app required_inputs flow instead.",
                verb,
                path.display()
            );
        }
        Ok(())
    }

    pub(super) fn resolve_tool_read_path(&self, raw: &str) -> Result<PathBuf> {
        let candidate = self.absolutize_tool_path(raw)?;
        let resolved = candidate.canonicalize()?;
        self.ensure_tool_path_allowed(&resolved)?;
        Self::ensure_tool_target_not_sensitive_file(&resolved, "read")?;
        Ok(resolved)
    }

    pub(super) fn display_tool_path(path: &Path) -> String {
        let text = path.display().to_string();
        #[cfg(windows)]
        {
            if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
                return format!(r"\\{}", rest);
            }
            if let Some(rest) = text.strip_prefix(r"\\?\") {
                return rest.to_string();
            }
        }
        text
    }

    pub(super) fn resolve_tool_write_path(&self, raw: &str) -> Result<PathBuf> {
        self.resolve_tool_mutation_path(raw, "write")
    }

    pub(super) fn resolve_tool_mutation_path(&self, raw: &str, verb: &str) -> Result<PathBuf> {
        let candidate = self.absolutize_tool_path(raw)?;
        if candidate.exists() {
            let resolved = candidate.canonicalize()?;
            self.ensure_tool_write_path_allowed(&resolved)?;
            if resolved.is_dir() {
                anyhow::bail!("Refusing to overwrite directory '{}'", resolved.display());
            }
            Self::ensure_tool_target_not_sensitive_file(&resolved, verb)?;
            return Ok(resolved);
        }

        let mut missing_components = Vec::new();
        let mut cursor = candidate.as_path();
        while !cursor.exists() {
            let name = cursor
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Path '{}' has no existing parent", raw))?;
            missing_components.push(name.to_os_string());
            cursor = cursor
                .parent()
                .ok_or_else(|| anyhow::anyhow!("Path '{}' has no existing parent", raw))?;
        }

        let mut rebuilt = cursor.canonicalize()?;
        self.ensure_tool_write_path_allowed(&rebuilt)?;
        for component in missing_components.into_iter().rev() {
            let component_text = component.to_string_lossy();
            if component_text.is_empty() || component_text == "." || component_text == ".." {
                anyhow::bail!("Invalid path component '{}'", component_text);
            }
            rebuilt.push(component);
        }
        Self::ensure_tool_target_not_sensitive_file(&rebuilt, verb)?;
        Ok(rebuilt)
    }

    pub(super) fn tool_payload_resource_dir(&self) -> PathBuf {
        self.data_dir().join(TOOL_PAYLOAD_RESOURCE_DIR)
    }

    pub(super) fn parse_runtime_resource_ref(
        value: &serde_json::Value,
    ) -> Option<RuntimeResourceRef> {
        serde_json::from_value::<RuntimeResourceRef>(value.clone()).ok()
    }

    pub(super) fn payload_from_structured_completion(
        value: &serde_json::Value,
    ) -> Option<ToolPayload> {
        let data = value.get("data").unwrap_or(value);
        if let Some(resource) = data
            .get("payload")
            .and_then(|payload| payload.get("resource"))
            .and_then(Self::parse_runtime_resource_ref)
        {
            return Some(ToolPayload::Resource {
                resource,
                metadata: Some(value.clone()),
            });
        }
        if let Some(resource) = data
            .get("resource")
            .and_then(Self::parse_runtime_resource_ref)
        {
            return Some(ToolPayload::Resource {
                resource,
                metadata: Some(value.clone()),
            });
        }
        if let Some(resource) = data
            .get("saved_body")
            .and_then(Self::runtime_resource_from_saved_body)
        {
            return Some(ToolPayload::Resource {
                resource,
                metadata: Some(value.clone()),
            });
        }
        None
    }

    pub(super) fn runtime_resource_from_saved_body(
        value: &serde_json::Value,
    ) -> Option<RuntimeResourceRef> {
        let path = value.get("path")?.as_str()?.trim();
        if path.is_empty() {
            return None;
        }
        let bytes = value
            .get("bytes")
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        let mime = value
            .get("content_type")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        Some(RuntimeResourceRef {
            id: path.to_string(),
            path: path.to_string(),
            mime,
            bytes,
            created_at: chrono::Utc::now().to_rfc3339(),
            source_action: None,
        })
    }

    pub(super) fn runtime_resource_from_argument(
        value: &serde_json::Value,
    ) -> Option<RuntimeResourceRef> {
        if let Some(resource) = Self::parse_runtime_resource_ref(value) {
            return Some(resource);
        }
        let data = value.get("data").unwrap_or(value);
        if let Some(resource) = data
            .get("payload")
            .and_then(|payload| payload.get("resource"))
            .and_then(Self::parse_runtime_resource_ref)
        {
            return Some(resource);
        }
        if let Some(resource) = data
            .get("resource")
            .and_then(Self::parse_runtime_resource_ref)
        {
            return Some(resource);
        }
        if let Some(resource) = data
            .get("saved_body")
            .and_then(Self::runtime_resource_from_saved_body)
        {
            return Some(resource);
        }
        let text = value.as_str()?.trim();
        if text.is_empty() {
            return None;
        }
        if let Some(parsed) = text
            .strip_prefix(TOOL_COMPLETION_MARKER)
            .and_then(|payload| {
                serde_json::from_str::<serde_json::Value>(
                    payload.lines().next().unwrap_or(payload).trim(),
                )
                .ok()
            })
            .and_then(|parsed| Self::runtime_resource_from_argument(&parsed))
        {
            return Some(parsed);
        }
        if let Some(parsed) = serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|parsed| Self::runtime_resource_from_argument(&parsed))
        {
            return Some(parsed);
        }
        Some(RuntimeResourceRef {
            id: text.to_string(),
            path: text.to_string(),
            mime: None,
            bytes: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            source_action: None,
        })
    }

    pub(super) async fn resolve_runtime_resource_path(
        &self,
        resource: &RuntimeResourceRef,
    ) -> Result<PathBuf> {
        let raw_path = resource.path.trim();
        if !raw_path.is_empty() {
            if let Ok(path) = self.resolve_tool_read_path(raw_path) {
                return Ok(path);
            }
        }

        let id = resource.id.trim();
        if !id.is_empty() {
            let base = self.tool_payload_resource_dir().join(id);
            let resolved = base.canonicalize().map_err(|_| {
                anyhow::anyhow!(
                    "Resource '{}' is not available on disk. Re-fetch it before saving.",
                    id
                )
            })?;
            self.ensure_tool_path_allowed(&resolved)?;
            if resolved.is_file() {
                return Ok(resolved);
            }
            if resolved.is_dir() {
                let mut files = Vec::new();
                let mut entries = tokio::fs::read_dir(&resolved).await?;
                while let Some(entry) = entries.next_entry().await? {
                    if entry.file_type().await?.is_file() {
                        files.push(entry.path().canonicalize()?);
                    }
                }
                match files.len() {
                    1 => return Ok(files.remove(0)),
                    0 => anyhow::bail!("Resource '{}' contains no file payload.", id),
                    _ => anyhow::bail!(
                        "Resource '{}' contains multiple files; pass the exact resource path.",
                        id
                    ),
                }
            }
        }

        anyhow::bail!(
            "Resource reference could not be resolved. Pass a ResourceRef object, resource id, or allowed resource path."
        )
    }

    pub(super) async fn file_write_payload_from_arguments(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<FileWritePayload> {
        if let Some(value) = arguments
            .get("source_resource")
            .or_else(|| arguments.get("resource"))
            .or_else(|| arguments.get("input_resource"))
        {
            let resource = Self::runtime_resource_from_argument(value).ok_or_else(|| {
                anyhow::anyhow!(
                    "source_resource must be a ResourceRef object, structured resource payload, resource id, or resource path"
                )
            })?;
            let path = self.resolve_runtime_resource_path(&resource).await?;
            let bytes = tokio::fs::read(&path).await?;
            return Ok(FileWritePayload {
                bytes,
                mime: resource.mime.clone(),
                source_resource: Some(resource),
            });
        }

        if let Some(source_path) = arguments
            .get("source_path")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let path = self.resolve_tool_read_path(source_path)?;
            return Ok(FileWritePayload {
                bytes: tokio::fs::read(&path).await?,
                mime: mime_guess::from_path(&path).first_raw().map(str::to_string),
                source_resource: None,
            });
        }

        if let Some(encoded) = arguments
            .get("content_base64")
            .or_else(|| arguments.get("bytes_b64"))
            .and_then(|value| value.as_str())
        {
            let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
                .map_err(|error| anyhow::anyhow!("Invalid base64 content: {}", error))?;
            return Ok(FileWritePayload {
                bytes,
                mime: arguments
                    .get("content_type")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                source_resource: None,
            });
        }

        if let Some(content) = arguments.get("content").and_then(|value| value.as_str()) {
            let trimmed = content.trim();
            let structured_resource = trimmed
                .strip_prefix(TOOL_COMPLETION_MARKER)
                .and_then(|payload| {
                    serde_json::from_str::<serde_json::Value>(
                        payload.lines().next().unwrap_or(payload).trim(),
                    )
                    .ok()
                })
                .and_then(|parsed| Self::runtime_resource_from_argument(&parsed))
                .or_else(|| {
                    serde_json::from_str::<serde_json::Value>(trimmed)
                        .ok()
                        .and_then(|parsed| Self::runtime_resource_from_argument(&parsed))
                });
            if let Some(resource) = structured_resource {
                let path = self.resolve_runtime_resource_path(&resource).await?;
                let bytes = tokio::fs::read(&path).await?;
                return Ok(FileWritePayload {
                    bytes,
                    mime: resource.mime.clone(),
                    source_resource: Some(resource),
                });
            }
            return Ok(FileWritePayload {
                bytes: content.as_bytes().to_vec(),
                mime: arguments
                    .get("content_type")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                source_resource: None,
            });
        }

        anyhow::bail!(
            "Missing file body. Provide content, content_base64, source_resource, or source_path."
        )
    }

    pub(super) fn file_write_label(path: &Path) -> String {
        path.file_name()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| "managed-file".to_string())
    }

    pub(super) fn pdf_generate_filename(raw: &str) -> String {
        let trimmed = raw.trim();
        let stem = Path::new(trimmed)
            .file_stem()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(trimmed);
        let stem = Self::normalize_generated_action_name(stem);
        if stem.is_empty() {
            "output.pdf".to_string()
        } else {
            format!("{stem}.pdf")
        }
    }

    pub(super) fn file_write_document_index_requested(arguments: &serde_json::Value) -> bool {
        for key in ["document_visible", "index_document"] {
            if let Some(value) = arguments.get(key).and_then(|value| value.as_bool()) {
                return value;
            }
        }
        false
    }

    pub(super) fn duplicate_policy_allows_create(arguments: &serde_json::Value) -> bool {
        arguments
            .get("allow_duplicate")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
            || arguments
                .get("duplicate_policy")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .is_some_and(|value| value == "create_new")
    }

    pub(super) fn fingerprint_bytes(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }

    pub(super) fn safe_output_route_segment(value: &str) -> bool {
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    }

    pub(super) fn output_file_links(&self, path: &Path) -> Option<(String, String)> {
        let outputs_root = self.data_dir().join("outputs");
        let relative = path.strip_prefix(&outputs_root).ok()?;
        let mut components = relative.components();
        let exec_id = match components.next()? {
            std::path::Component::Normal(value) => value.to_str()?,
            _ => return None,
        };
        let filename = match components.next()? {
            std::path::Component::Normal(value) => value.to_str()?,
            _ => return None,
        };
        if components.next().is_some()
            || uuid::Uuid::parse_str(exec_id).is_err()
            || !Self::safe_output_route_segment(filename)
        {
            return None;
        }
        let url = format!("/api/outputs/{}/{}", exec_id, filename);
        let download_url = format!("{}/download", url);
        Some((url, download_url))
    }

    pub(super) fn document_chunks_from_text(content: &str, chunk_size: usize) -> Vec<String> {
        let chars = content.chars().collect::<Vec<_>>();
        chars
            .chunks(chunk_size.max(1))
            .map(|chunk| chunk.iter().collect::<String>())
            .filter(|chunk| !chunk.trim().is_empty())
            .collect()
    }

    pub(super) fn generated_file_metadata_chunk(
        filename: &str,
        content_type: &str,
        file_size: usize,
        content_fingerprint: &str,
        text_content_indexed: bool,
        source_resource: Option<&RuntimeResourceRef>,
        download_url: Option<&str>,
    ) -> String {
        let mut lines = vec![
            "artifact_kind: managed_file".to_string(),
            format!("filename: {}", filename),
            format!("content_type: {}", content_type),
            format!("file_size_bytes: {}", file_size),
            format!("sha256: {}", content_fingerprint),
            format!("text_content_indexed: {}", text_content_indexed),
        ];
        if let Some(download_url) = download_url.map(str::trim).filter(|value| {
            value.starts_with("/api/outputs/") && !value.contains("..") && !value.contains('\\')
        }) {
            lines.push(format!("download_url: {}", download_url));
        }
        if let Some(resource) = source_resource {
            lines.push(format!("source_resource_id: {}", resource.id));
            if let Some(mime) = resource
                .mime
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                lines.push(format!("source_resource_content_type: {}", mime));
            }
            if let Some(source_action) = resource
                .source_action
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                lines.push(format!("source_action: {}", source_action));
            }
        }
        lines.join("\n")
    }

    pub(super) async fn find_duplicate_generated_document(
        &self,
        filename: &str,
        content_type: &str,
        file_size: u64,
        content_fingerprint: &str,
    ) -> Option<IndexedDocumentArtifact> {
        let storage = self.storage.as_ref()?;
        let docs = storage.list_documents_for_search(None).await.ok()?;
        let candidates = docs
            .into_iter()
            .filter(|doc| {
                doc.file_size == file_size.min(i64::MAX as u64) as i64
                    && doc.content_type == content_type
            })
            .collect::<Vec<_>>();
        for doc in candidates {
            let Ok(chunks) = storage.get_document_chunks(&doc.id).await else {
                continue;
            };
            let fingerprint_line = format!("sha256: {}", content_fingerprint);
            let has_matching_fingerprint = chunks.iter().any(|chunk| {
                chunk
                    .content
                    .lines()
                    .any(|line| line.trim() == fingerprint_line.as_str())
            });
            if has_matching_fingerprint {
                let metadata_only = chunks
                    .iter()
                    .any(|chunk| chunk.content.contains("text_content_indexed: false"));
                return Some(IndexedDocumentArtifact {
                    id: doc.id,
                    filename: if doc.filename.trim().is_empty() {
                        filename.to_string()
                    } else {
                        doc.filename
                    },
                    content_type: doc.content_type,
                    chunk_count: chunks.len(),
                    file_size,
                    url: "/ui/documents".to_string(),
                    download_url: None,
                    duplicate_skipped: true,
                    content_fingerprint: content_fingerprint.to_string(),
                    metadata_only,
                    index_mode: if metadata_only {
                        "metadata".to_string()
                    } else {
                        "metadata_and_content".to_string()
                    },
                });
            }
        }
        None
    }

    pub(super) async fn index_file_write_document_if_requested(
        &self,
        path: &Path,
        arguments: &serde_json::Value,
        payload: &FileWritePayload,
        mime: Option<&str>,
    ) -> Option<IndexedDocumentArtifact> {
        if !Self::file_write_document_index_requested(arguments) {
            return None;
        }
        let storage = self.storage.clone()?;
        let filename = Self::file_write_label(path);
        let content_type = mime
            .map(str::to_string)
            .unwrap_or_else(|| "text/plain".to_string());
        let content_fingerprint = Self::fingerprint_bytes(&payload.bytes);
        let text_content = std::str::from_utf8(&payload.bytes)
            .ok()
            .map(str::trim)
            .filter(|content| !content.is_empty() && payload.bytes.len() <= 2_000_000)
            .map(str::to_string);
        let text_content_indexed = text_content.is_some();
        let output_download_url = self
            .output_file_links(path)
            .map(|(_url, download_url)| download_url);
        let mut chunks = vec![Self::generated_file_metadata_chunk(
            &filename,
            &content_type,
            payload.bytes.len(),
            &content_fingerprint,
            text_content_indexed,
            payload.source_resource.as_ref(),
            output_download_url.as_deref(),
        )];
        if let Some(content) = text_content.as_deref() {
            chunks.extend(Self::document_chunks_from_text(content, 1000));
        }
        if !Self::duplicate_policy_allows_create(arguments) {
            if let Some(duplicate) = self
                .find_duplicate_generated_document(
                    &filename,
                    &content_type,
                    payload.bytes.len() as u64,
                    &content_fingerprint,
                )
                .await
            {
                tracing::info!(
                    path = %path.display(),
                    document_id = %duplicate.id,
                    "Skipped generated document ingestion because identical content is already indexed"
                );
                return Some(duplicate);
            }
        }
        let path_text = path.display().to_string();
        let id_prefix = format!(
            "generated-file:{}:",
            Self::fingerprint_text(&[path_text.as_str()])
        );
        let doc_id = format!("{}{}", id_prefix, uuid::Uuid::new_v4());
        let doc = crate::storage::entities::document::Model {
            id: doc_id.clone(),
            filename: filename.clone(),
            content_type: content_type.clone(),
            project_id: None,
            chunk_count: chunks.len() as i32,
            file_size: payload.bytes.len().min(i64::MAX as usize) as i64,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let mut chunk_rows = chunks
            .iter()
            .enumerate()
            .map(
                |(index, chunk)| crate::storage::entities::document_chunk::Model {
                    id: uuid::Uuid::new_v4().to_string(),
                    document_id: doc_id.clone(),
                    chunk_index: index as i32,
                    content: chunk.clone(),
                    embedding: None,
                },
            )
            .collect::<Vec<_>>();
        if let Err(error) = crate::core::knowledge::document_search::embed_document_chunks(
            self.embedding_client.as_deref(),
            &filename,
            &content_type,
            None,
            &mut chunk_rows,
        )
        .await
        {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "Generated managed file document embedding failed"
            );
        }
        if let Err(error) = storage
            .replace_documents_by_id_prefix(&id_prefix, &[(doc, chunk_rows)])
            .await
        {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "Failed to index generated managed file in Documents"
            );
            return None;
        }
        Some(IndexedDocumentArtifact {
            id: doc_id,
            filename,
            content_type,
            chunk_count: chunks.len(),
            file_size: payload.bytes.len() as u64,
            url: "/ui/documents".to_string(),
            download_url: output_download_url,
            duplicate_skipped: false,
            content_fingerprint,
            metadata_only: !text_content_indexed,
            index_mode: if text_content_indexed {
                "metadata_and_content".to_string()
            } else {
                "metadata".to_string()
            },
        })
    }

    pub(super) fn managed_file_completion_output(
        &self,
        action_name: &str,
        path: &Path,
        payload: &FileWritePayload,
        document: Option<&IndexedDocumentArtifact>,
    ) -> String {
        let path_text = path.display().to_string();
        let label = Self::file_write_label(path);
        let mime = payload
            .mime
            .clone()
            .or_else(|| mime_guess::from_path(path).first_raw().map(str::to_string));
        let resource = RuntimeResourceRef {
            id: format!("file:{}", Self::fingerprint_text(&[path_text.as_str()])),
            path: path_text.clone(),
            mime: mime.clone(),
            bytes: payload.bytes.len() as u64,
            created_at: chrono::Utc::now().to_rfc3339(),
            source_action: Some(action_name.to_string()),
        };
        let detail = if document
            .as_ref()
            .is_some_and(|document| document.duplicate_skipped)
        {
            format!(
                "Saved managed file {}. Identical document already exists; skipped Documents ingestion.",
                label
            )
        } else if document
            .as_ref()
            .is_some_and(|document| document.metadata_only)
        {
            format!(
                "Saved managed file {} and registered its metadata in Documents.",
                label
            )
        } else if document.is_some() {
            format!(
                "Saved managed file {} and indexed its metadata and text content in Documents.",
                label
            )
        } else {
            format!("Saved managed file {}.", label)
        };
        let artifact_label = label.clone();
        let write_label = label;
        let artifact_mime = mime.clone();
        let write_mime = mime.clone();
        let mut artifact = serde_json::Map::new();
        artifact.insert(
            "kind".to_string(),
            serde_json::Value::String("managed_file".to_string()),
        );
        artifact.insert(
            "label".to_string(),
            serde_json::Value::String(artifact_label),
        );
        artifact.insert("bytes".to_string(), serde_json::json!(payload.bytes.len()));
        if let Some(artifact_mime) = artifact_mime {
            artifact.insert(
                "content_type".to_string(),
                serde_json::Value::String(artifact_mime),
            );
        }
        if let Some((url, download_url)) = self.output_file_links(path) {
            artifact.insert("url".to_string(), serde_json::Value::String(url));
            artifact.insert(
                "download_url".to_string(),
                serde_json::Value::String(download_url),
            );
        }
        structured_tool_completion_output(
            action_name,
            "completed",
            detail,
            serde_json::json!({
                "payload": {
                    "kind": "resource",
                    "resource": resource,
                },
                "artifact": artifact,
                "document": document,
                "write": {
                    "label": write_label,
                    "bytes": payload.bytes.len(),
                    "content_type": write_mime,
                    "source_resource": payload.source_resource,
                }
            }),
        )
    }

    pub(super) fn file_write_completion_output(
        &self,
        path: &Path,
        payload: &FileWritePayload,
        document: Option<&IndexedDocumentArtifact>,
    ) -> String {
        self.managed_file_completion_output("file_write", path, payload, document)
    }

    pub fn tool_payload_from_legacy_output(_action_name: &str, output: String) -> ToolPayload {
        let trimmed = output.trim();
        if trimmed.is_empty() {
            return ToolPayload::Empty;
        }
        if let Some(payload) = trimmed
            .strip_prefix(TOOL_COMPLETION_MARKER)
            .and_then(|payload| {
                serde_json::from_str::<serde_json::Value>(
                    payload.lines().next().unwrap_or(payload).trim(),
                )
                .ok()
            })
        {
            if let Some(typed) = Self::payload_from_structured_completion(&payload) {
                return typed;
            }
            return ToolPayload::Structured(payload);
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(typed) = Self::payload_from_structured_completion(&value) {
                return typed;
            }
            return ToolPayload::Structured(value);
        }
        ToolPayload::Text(output)
    }

    pub(super) async fn persist_tool_payload_if_needed(
        &self,
        mut payload: ToolPayload,
        hints: PersistHints,
    ) -> Result<ToolPayload> {
        let ToolPayload::Bytes {
            mime,
            body,
            suggested_name,
        } = payload
        else {
            return Ok(payload);
        };
        if !hints.force_resource
            && body.len() <= TOOL_PAYLOAD_INLINE_BYTES
            && !runtime_response_body_is_probably_binary(mime.as_deref().unwrap_or(""), &body)
        {
            payload = ToolPayload::Text(String::from_utf8_lossy(&body).to_string());
            return Ok(payload);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let name = suggested_name
            .or(hints.suggested_name)
            .map(|value| Self::normalize_generated_action_name(&value))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| id.clone());
        let target = self.tool_payload_resource_dir().join(&id).join(name);
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&target, &body).await?;
        Self::set_private_file_permissions(&target).await?;
        let resource = RuntimeResourceRef {
            id,
            path: target.display().to_string(),
            mime: mime.or(hints.mime),
            bytes: body.len() as u64,
            created_at: chrono::Utc::now().to_rfc3339(),
            source_action: hints.source_action,
        };
        Ok(ToolPayload::Resource {
            resource,
            metadata: None,
        })
    }

    pub fn render_tool_payload_for_legacy(action_name: &str, payload: ToolPayload) -> String {
        match payload {
            ToolPayload::Text(text) => text,
            ToolPayload::Structured(value) => {
                if value.get("tool").and_then(|value| value.as_str()).is_some()
                    && value
                        .get("status")
                        .and_then(|value| value.as_str())
                        .is_some()
                {
                    format!("{}{}", TOOL_COMPLETION_MARKER, value)
                } else {
                    structured_tool_completion_output(
                        action_name,
                        "completed",
                        "Structured tool payload.",
                        serde_json::json!({
                            "payload": {
                                "kind": "structured",
                                "value": value,
                            }
                        }),
                    )
                }
            }
            ToolPayload::Bytes {
                mime,
                body,
                suggested_name,
            } => structured_tool_completion_output(
                action_name,
                "partial",
                "Tool produced raw bytes that were not persisted.",
                serde_json::json!({
                    "payload": {
                        "kind": "bytes",
                        "mime": mime,
                        "bytes": body.len(),
                        "suggested_name": suggested_name,
                    },
                    "body_quality": {
                        "body_bytes": body.len(),
                        "binary": true,
                        "degenerate": true,
                        "reason": "bytes_not_persisted"
                    }
                }),
            ),
            ToolPayload::Resource { resource, metadata } => structured_tool_completion_output(
                action_name,
                "completed",
                format!("Tool produced resource {}.", resource.path),
                serde_json::json!({
                    "payload": {
                        "kind": "resource",
                        "resource": resource,
                    },
                    "metadata": metadata,
                }),
            ),
            ToolPayload::Empty => structured_tool_completion_output(
                action_name,
                "completed",
                "Tool completed with no payload.",
                serde_json::json!({
                    "payload": {
                        "kind": "empty"
                    }
                }),
            ),
        }
    }

    pub(super) fn parse_tool_string_list(
        arguments: &serde_json::Value,
        key: &str,
    ) -> Result<Vec<String>> {
        let Some(value) = arguments.get(key) else {
            return Ok(Vec::new());
        };
        match value {
            serde_json::Value::String(item) => Ok(item
                .trim()
                .is_empty()
                .then(Vec::new)
                .unwrap_or_else(|| vec![item.trim().to_string()])),
            serde_json::Value::Array(items) => items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .map(str::to_string)
                        .ok_or_else(|| anyhow::anyhow!("{} entries must be non-empty strings", key))
                })
                .collect(),
            _ => anyhow::bail!("{} must be a string or an array of strings", key),
        }
    }

    pub(super) fn tool_path_relative_to(root: &Path, path: &Path) -> String {
        path.strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string()
            .replace('\\', "/")
    }

    pub(super) fn compile_tool_globs(patterns: &[String]) -> Result<Vec<Regex>> {
        patterns
            .iter()
            .map(|pattern| Self::compile_tool_glob(pattern))
            .collect()
    }

    pub(super) fn compile_tool_glob(pattern: &str) -> Result<Regex> {
        let normalized = pattern.trim().replace('\\', "/");
        if normalized.is_empty() {
            anyhow::bail!("Glob patterns cannot be empty");
        }
        let mut regex = String::from("^");
        let mut chars = normalized.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '*' => {
                    if chars.peek() == Some(&'*') {
                        chars.next();
                        if chars.peek() == Some(&'/') {
                            chars.next();
                            regex.push_str("(?:.*/)?");
                        } else {
                            regex.push_str(".*");
                        }
                    } else {
                        regex.push_str("[^/]*");
                    }
                }
                '?' => regex.push_str("[^/]"),
                '/' => regex.push('/'),
                _ => regex.push_str(&regex::escape(&ch.to_string())),
            }
        }
        regex.push('$');
        Regex::new(&regex).with_context(|| format!("Invalid glob pattern '{}'", pattern.trim()))
    }

    pub(super) fn tool_globs_match(
        patterns: &[Regex],
        relative_path: &str,
        file_name: &str,
    ) -> bool {
        !patterns.is_empty()
            && patterns
                .iter()
                .any(|pattern| pattern.is_match(relative_path) || pattern.is_match(file_name))
    }

    pub(super) fn tool_text_contains(haystack: &str, needle: &str, case_sensitive: bool) -> bool {
        if needle.is_empty() {
            return false;
        }
        if case_sensitive {
            haystack.contains(needle)
        } else {
            haystack.to_lowercase().contains(&needle.to_lowercase())
        }
    }

    pub(super) fn file_search_should_skip_default_dir(name: &std::ffi::OsStr) -> bool {
        let Some(name) = name.to_str() else {
            return false;
        };
        FILE_SEARCH_DEFAULT_SKIPPED_DIRS
            .iter()
            .any(|candidate| name.eq_ignore_ascii_case(candidate))
    }

    pub(super) async fn execute_file_search(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let root = if let Some(raw) = arguments
            .get("root")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            self.resolve_tool_read_path(raw)?
        } else {
            let root = self.workspace_root();
            let resolved = root.canonicalize().unwrap_or(root);
            self.ensure_tool_path_allowed(&resolved)?;
            resolved
        };
        if !root.is_dir() {
            anyhow::bail!("file_search root must be an existing directory");
        }

        let mode = arguments
            .get("mode")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("auto")
            .to_ascii_lowercase();
        if !matches!(mode.as_str(), "auto" | "filename" | "content" | "both") {
            anyhow::bail!("file_search mode must be auto, filename, content, or both");
        }

        let query = arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let filename_query = arguments
            .get("filename_query")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| if mode != "content" { query } else { None })
            .map(str::to_string);
        let content_query = arguments
            .get("content_query")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| if mode != "filename" { query } else { None })
            .map(str::to_string);
        let include_globs = Self::parse_tool_string_list(arguments, "globs")?;
        let exclude_globs = Self::parse_tool_string_list(arguments, "exclude_globs")?;
        if filename_query.is_none() && content_query.is_none() && include_globs.is_empty() {
            anyhow::bail!("file_search requires query, filename_query, content_query, or globs");
        }

        let include_patterns = Self::compile_tool_globs(&include_globs)?;
        let exclude_patterns = Self::compile_tool_globs(&exclude_globs)?;
        let case_sensitive = arguments
            .get("case_sensitive")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let context_lines = arguments
            .get("context_lines")
            .and_then(|value| value.as_u64())
            .unwrap_or(2)
            .min(8) as usize;
        let limit = arguments
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(50)
            .clamp(1, 200) as usize;
        let max_file_bytes = arguments
            .get("max_file_bytes")
            .and_then(|value| value.as_u64())
            .unwrap_or(1_000_000)
            .clamp(4_096, 2_000_000);
        let include_heavy_dirs = arguments
            .get("include_heavy_dirs")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let max_files_scanned = arguments
            .get("max_files_scanned")
            .and_then(|value| value.as_u64())
            .unwrap_or(FILE_SEARCH_DEFAULT_MAX_FILES_SCANNED as u64)
            .clamp(50, 50_000) as usize;
        let max_entries_visited = arguments
            .get("max_entries_visited")
            .and_then(|value| value.as_u64())
            .unwrap_or(FILE_SEARCH_DEFAULT_MAX_ENTRIES_VISITED as u64)
            .clamp(100, 250_000) as usize;

        let mut matches = Vec::<serde_json::Value>::new();
        let mut searched_files = 0usize;
        let mut visited_entries = 0usize;
        let mut content_scanned_files = 0usize;
        let mut skipped_sensitive = 0usize;
        let mut skipped_directories = 0usize;
        let mut skipped_large = 0usize;
        let mut skipped_binary = 0usize;
        let mut truncated = false;
        let mut truncation_reason: Option<&'static str> = None;

        for entry in walkdir::WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                if include_heavy_dirs || entry.depth() == 0 || !entry.file_type().is_dir() {
                    return true;
                }
                if Self::file_search_should_skip_default_dir(entry.file_name()) {
                    skipped_directories += 1;
                    return false;
                }
                true
            })
            .filter_map(|entry| entry.ok())
        {
            visited_entries += 1;
            if visited_entries > max_entries_visited {
                truncated = true;
                truncation_reason = Some("max_entries_visited");
                break;
            }
            if matches.len() >= limit {
                truncated = true;
                truncation_reason = Some("limit");
                break;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            if searched_files >= max_files_scanned {
                truncated = true;
                truncation_reason = Some("max_files_scanned");
                break;
            }
            let path = entry.path();
            let resolved = match path.canonicalize() {
                Ok(path) => path,
                Err(_) => continue,
            };
            if self.ensure_tool_path_allowed(&resolved).is_err() {
                continue;
            }
            if Self::tool_path_looks_sensitive_file(&resolved) {
                skipped_sensitive += 1;
                continue;
            }

            let relative_path = Self::tool_path_relative_to(&root, &resolved);
            let file_name = resolved
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string();
            if (!include_patterns.is_empty()
                && !Self::tool_globs_match(&include_patterns, &relative_path, &file_name))
                || Self::tool_globs_match(&exclude_patterns, &relative_path, &file_name)
            {
                continue;
            }

            searched_files += 1;
            if let Some(query) = filename_query.as_deref() {
                if Self::tool_text_contains(&relative_path, query, case_sensitive)
                    || Self::tool_text_contains(&file_name, query, case_sensitive)
                {
                    matches.push(serde_json::json!({
                        "match_type": "filename",
                        "path": resolved.display().to_string(),
                        "relative_path": relative_path.clone(),
                        "file_name": file_name.clone(),
                    }));
                    if matches.len() >= limit {
                        truncated = true;
                        truncation_reason = Some("limit");
                        break;
                    }
                }
            } else if content_query.is_none() && !include_globs.is_empty() {
                matches.push(serde_json::json!({
                    "match_type": "path",
                    "path": resolved.display().to_string(),
                    "relative_path": relative_path.clone(),
                    "file_name": file_name.clone(),
                }));
                if matches.len() >= limit {
                    truncated = true;
                    truncation_reason = Some("limit");
                    break;
                }
            }

            let Some(query) = content_query.as_deref() else {
                continue;
            };
            let metadata = match std::fs::metadata(&resolved) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if metadata.len() > max_file_bytes {
                skipped_large += 1;
                continue;
            }
            let bytes = match std::fs::read(&resolved) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            if bytes.contains(&0) {
                skipped_binary += 1;
                continue;
            }
            let text = match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(_) => {
                    skipped_binary += 1;
                    continue;
                }
            };
            content_scanned_files += 1;
            let lines = text.lines().collect::<Vec<_>>();
            for (index, line) in lines.iter().enumerate() {
                if !Self::tool_text_contains(line, query, case_sensitive) {
                    continue;
                }
                let before_start = index.saturating_sub(context_lines);
                let after_end = (index + 1 + context_lines).min(lines.len());
                matches.push(serde_json::json!({
                    "match_type": "content",
                    "path": resolved.display().to_string(),
                    "relative_path": relative_path.clone(),
                    "line_number": index + 1,
                    "line": runtime_truncate_chars(line.trim_end(), 800),
                    "context_before": lines[before_start..index]
                        .iter()
                        .map(|line| runtime_truncate_chars(line.trim_end(), 800))
                        .collect::<Vec<_>>(),
                    "context_after": lines[(index + 1)..after_end]
                        .iter()
                        .map(|line| runtime_truncate_chars(line.trim_end(), 800))
                        .collect::<Vec<_>>(),
                }));
                if matches.len() >= limit {
                    truncated = true;
                    truncation_reason = Some("limit");
                    break;
                }
            }
            if truncated {
                break;
            }
        }

        let detail = format!(
            "Found {} match(es) under {}. Searched {} file(s).",
            matches.len(),
            root.display(),
            searched_files
        );
        Ok(structured_tool_completion_output(
            "file_search",
            "completed",
            detail,
            serde_json::json!({
                "root": root.display().to_string(),
                "query": query,
                "filename_query": filename_query,
                "content_query": content_query,
                "globs": include_globs,
                "exclude_globs": exclude_globs,
                "case_sensitive": case_sensitive,
                "context_lines": context_lines,
                "limit": limit,
                "truncated": truncated,
                "truncation_reason": truncation_reason,
                "max_files_scanned": max_files_scanned,
                "max_entries_visited": max_entries_visited,
                "visited_entries": visited_entries,
                "searched_files": searched_files,
                "content_scanned_files": content_scanned_files,
                "skipped_sensitive": skipped_sensitive,
                "skipped_directories": skipped_directories,
                "skipped_large": skipped_large,
                "skipped_binary": skipped_binary,
                "include_heavy_dirs": include_heavy_dirs,
                "matches": matches,
            }),
        ))
    }

    pub(super) fn parse_file_patch_requests(
        arguments: &serde_json::Value,
    ) -> Result<Vec<(String, String)>> {
        if let Some(items) = arguments.get("patches") {
            let items = items
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("patches must be an array"))?;
            if items.is_empty() {
                anyhow::bail!("patches cannot be empty");
            }
            return items
                .iter()
                .map(|item| {
                    let object = item
                        .as_object()
                        .ok_or_else(|| anyhow::anyhow!("patches entries must be objects"))?;
                    let path = object
                        .get("path")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| anyhow::anyhow!("patches entries require path"))?;
                    let patch = object
                        .get("patch")
                        .and_then(|value| value.as_str())
                        .filter(|value| !value.trim().is_empty())
                        .ok_or_else(|| anyhow::anyhow!("patches entries require patch"))?;
                    Ok((path.to_string(), patch.to_string()))
                })
                .collect();
        }

        let path = arguments
            .get("path")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("file_patch requires path or patches"))?;
        let patch = arguments
            .get("patch")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("file_patch requires patch"))?;
        Ok(vec![(path.to_string(), patch.to_string())])
    }

    pub(super) async fn execute_file_patch(&self, arguments: &serde_json::Value) -> Result<String> {
        let requests = Self::parse_file_patch_requests(arguments)?;
        let dry_run = arguments
            .get("dry_run")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let mut seen = BTreeSet::<PathBuf>::new();
        let mut changed_files = Vec::<serde_json::Value>::new();

        for (raw_path, patch) in requests {
            let path = self.resolve_tool_mutation_path(&raw_path, "patch")?;
            if !seen.insert(path.clone()) {
                anyhow::bail!("file_patch received duplicate target '{}'", raw_path);
            }
            if !path.is_file() {
                anyhow::bail!(
                    "file_patch target must be an existing file: {}",
                    path.display()
                );
            }
            let before = tokio::fs::read_to_string(&path)
                .await
                .with_context(|| format!("Failed to read {}", path.display()))?;
            let after = crate::actions::app::apply_unified_diff_to_text(&before, &patch)
                .with_context(|| format!("Failed to apply unified diff to {}", path.display()))?;
            let changed = before != after;
            if changed && !dry_run {
                tokio::fs::write(&path, after.as_bytes())
                    .await
                    .with_context(|| format!("Failed to write patched file {}", path.display()))?;
            }
            changed_files.push(serde_json::json!({
                "path": path.display().to_string(),
                "requested_path": raw_path,
                "changed": changed,
                "dry_run": dry_run,
                "bytes_before": before.len(),
                "bytes_after": after.len(),
                "lines_before": before.lines().count(),
                "lines_after": after.lines().count(),
                "context_verified": true,
            }));
        }

        let changed_count = changed_files
            .iter()
            .filter(|file| {
                file.get("changed")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
            })
            .count();
        let detail = if dry_run {
            format!(
                "Validated {} patch target(s); {} would change.",
                changed_files.len(),
                changed_count
            )
        } else {
            format!(
                "Patched {} file(s); {} file(s) changed.",
                changed_files.len(),
                changed_count
            )
        };
        Ok(structured_tool_completion_output(
            "file_patch",
            "completed",
            detail,
            serde_json::json!({
                "dry_run": dry_run,
                "changed_count": changed_count,
                "changed_files": changed_files,
            }),
        ))
    }

    pub(super) async fn execute_file_delete(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let raw_path = arguments["path"]
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("file_delete requires path"))?;
        let path = self.resolve_tool_mutation_path(raw_path, "delete")?;
        if Self::tool_path_looks_sensitive_file(&path) {
            anyhow::bail!(
                "Refusing to delete sensitive credential file '{}'. Use the secure credential store instead.",
                path.display()
            );
        }
        let label = Self::file_write_label(&path);
        if !path.exists() {
            return Ok(structured_tool_completion_output(
                "file_delete",
                "completed",
                format!("File `{}` is already absent.", label),
                serde_json::json!({
                    "status": "not_found",
                    "deleted": false,
                    "label": label,
                    "requested_path": raw_path,
                    "terminal_observation": true,
                }),
            ));
        }
        if path.is_dir() {
            anyhow::bail!(
                "file_delete target must be a file, not a directory: {}",
                path.display()
            );
        }
        tokio::fs::remove_file(&path).await?;
        Ok(structured_tool_completion_output(
            "file_delete",
            "completed",
            format!("Deleted file `{}`.", label),
            serde_json::json!({
                "status": "deleted",
                "deleted": true,
                "label": label,
                "requested_path": raw_path,
            }),
        ))
    }
}
