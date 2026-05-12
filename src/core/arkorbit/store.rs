//! Layered filesystem store for ArkOrbit.
//!
//! Resolution is structural and path-based:
//! L2 user files under `<DATA_DIR>/arkorbit/L2/orbits/<id>/` win over L0
//! firmware files under `src/core/arkorbit/l0` during source-tree runs, which
//! win over the embedded L0 fallback compiled into the binary.

use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;
use walkdir::WalkDir;

use super::models::{OrbitFileEntry, OrbitManifest};

const HOST_JS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/core/arkorbit/l0/runtime/host.js"
));
const MOD_RESOLVER_JS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/core/arkorbit/l0/runtime/mod-resolver.js"
));
const SSE_JS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/core/arkorbit/l0/runtime/sse.js"
));
const MARKDOWN_MANIFEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/core/arkorbit/l0/widgets/markdown/manifest.json"
));
const MARKDOWN_INDEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/core/arkorbit/l0/widgets/markdown/index.js"
));
const IFRAME_HTML_MANIFEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/core/arkorbit/l0/widgets/iframe-html/manifest.json"
));
const IFRAME_HTML_INDEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/core/arkorbit/l0/widgets/iframe-html/index.js"
));
const CHART_MANIFEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/core/arkorbit/l0/widgets/chart/manifest.json"
));
const CHART_INDEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/core/arkorbit/l0/widgets/chart/index.js"
));
const TABLE_MANIFEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/core/arkorbit/l0/widgets/table/manifest.json"
));
const TABLE_INDEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/core/arkorbit/l0/widgets/table/index.js"
));
const TODO_MANIFEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/core/arkorbit/l0/widgets/todo/manifest.json"
));
const TODO_INDEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/core/arkorbit/l0/widgets/todo/index.js"
));
const FETCH_PROXY_MANIFEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/core/arkorbit/l0/widgets/fetch-proxy/manifest.json"
));
const FETCH_PROXY_INDEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/core/arkorbit/l0/widgets/fetch-proxy/index.js"
));
const APP_SHELL_MANIFEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/core/arkorbit/l0/widgets/app-shell/manifest.json"
));
const APP_SHELL_INDEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/core/arkorbit/l0/widgets/app-shell/index.js"
));
const SKILL_MD: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/core/arkorbit/l0/skills/SKILL.md"
));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleLayer {
    L2,
    L0Disk,
    L0Embedded,
}

#[derive(Debug, Clone)]
pub struct ResolvedModule {
    pub bytes: Vec<u8>,
    pub content_type: String,
    #[allow(dead_code)]
    pub layer: ModuleLayer,
}

#[derive(Debug, Clone)]
pub struct LayeredStore {
    l2_root: PathBuf,
    l0_roots: Vec<PathBuf>,
}

impl LayeredStore {
    pub fn new(data_dir: &Path) -> Self {
        let l2_root = data_dir.join("arkorbit").join("L2");
        let mut roots = BTreeSet::new();
        roots.insert(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("core")
                .join("arkorbit")
                .join("l0"),
        );
        Self {
            l2_root,
            l0_roots: roots.into_iter().collect(),
        }
    }

    pub fn orbits_root(&self) -> PathBuf {
        self.l2_root.join("orbits")
    }

    pub fn orbit_dir(&self, orbit_id: &str) -> PathBuf {
        self.orbits_root().join(orbit_id)
    }

    pub fn validate_orbit_id(orbit_id: &str) -> Result<()> {
        let trimmed = orbit_id.trim();
        if trimmed.is_empty() || trimmed != orbit_id {
            bail!("arkorbit: orbit_id must be a non-empty UUID without whitespace");
        }
        Uuid::parse_str(trimmed).map_err(|_| anyhow!("arkorbit: orbit_id must be a UUID"))?;
        Ok(())
    }

    pub fn ensure_orbit_dir(&self, orbit_id: &str) -> Result<PathBuf> {
        Self::validate_orbit_id(orbit_id)?;
        let dir = self.orbit_dir(orbit_id);
        std::fs::create_dir_all(dir.join("mod"))?;
        std::fs::create_dir_all(dir.join("data"))?;
        std::fs::create_dir_all(dir.join("assets"))?;
        std::fs::create_dir_all(dir.join(".tmp"))?;
        Ok(dir)
    }

    pub fn read_orbit_manifest(&self, orbit_id: &str) -> Result<OrbitManifest> {
        Self::validate_orbit_id(orbit_id)?;
        let path = self.orbit_dir(orbit_id).join("orbit.json");
        let bytes =
            std::fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn write_orbit_manifest(&self, manifest: &OrbitManifest) -> Result<()> {
        Self::validate_orbit_id(&manifest.id)?;
        self.ensure_orbit_dir(&manifest.id)?;
        let path = self.orbit_dir(&manifest.id).join("orbit.json");
        let bytes = serde_json::to_vec_pretty(manifest)?;
        self.atomic_write_under_orbit(&manifest.id, &path, &bytes)
    }

    pub fn write_default_index(&self, orbit_id: &str) -> Result<()> {
        Self::validate_orbit_id(orbit_id)?;
        self.ensure_orbit_dir(orbit_id)?;
        let index = default_index_html(orbit_id);
        let path = self.orbit_dir(orbit_id).join("index.html");
        self.atomic_write_under_orbit(orbit_id, &path, index.as_bytes())
    }

    pub fn read_orbit_index(&self, orbit_id: &str) -> Result<Vec<u8>> {
        Self::validate_orbit_id(orbit_id)?;
        let path = self.orbit_dir(orbit_id).join("index.html");
        match std::fs::read(&path) {
            Ok(bytes) => Ok(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(default_index_html(orbit_id).into_bytes())
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn remove_orbit(&self, orbit_id: &str) -> Result<()> {
        Self::validate_orbit_id(orbit_id)?;
        match std::fs::remove_dir_all(self.orbit_dir(orbit_id)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn list_orbit_dirs(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let root = self.orbits_root();
        let read = match std::fs::read_dir(&root) {
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(error) => return Err(error.into()),
        };
        for entry in read {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(|value| value.to_string()) else {
                continue;
            };
            if Uuid::parse_str(&name).is_ok() {
                out.push(name);
            }
        }
        Ok(out)
    }

    pub fn list_orbit_files(&self, orbit_id: &str) -> Result<Vec<OrbitFileEntry>> {
        Self::validate_orbit_id(orbit_id)?;
        let root = self.orbit_dir(orbit_id);
        let mut files = Vec::new();
        if !root.exists() {
            return Ok(files);
        }
        for entry in WalkDir::new(&root)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let Ok(rel) = path.strip_prefix(&root) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if !orbit_file_is_user_artifact_path(&rel) {
                continue;
            }
            let bytes = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            files.push(OrbitFileEntry { path: rel, bytes });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    pub fn read_orbit_file_text(&self, orbit_id: &str, rel_path: &str) -> Result<String> {
        let rel = validate_readable_orbit_path(rel_path)?;
        let root = self.ensure_orbit_dir(orbit_id)?;
        let path = root.join(rel);
        let resolved = canonicalize_existing_under(&root, &path)?;
        Ok(std::fs::read_to_string(resolved)?)
    }

    pub fn write_orbit_file(&self, orbit_id: &str, rel_path: &str, content: &[u8]) -> Result<()> {
        let rel = validate_writable_orbit_path(rel_path)?;
        let root = self.ensure_orbit_dir(orbit_id)?;
        validate_orbit_file_content(&root, &rel, content)?;
        let path = root.join(rel);
        self.atomic_write_under_orbit(orbit_id, &path, content)
    }

    pub fn remove_orbit_module_dir(&self, orbit_id: &str, module_name: &str) -> Result<bool> {
        Self::validate_orbit_id(orbit_id)?;
        let module = clean_relative_path(module_name)?;
        let parts = path_parts(&module);
        if parts.len() != 1
            || !parts[0]
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            bail!("arkorbit: widget module name must be one path segment");
        }

        let root = self.ensure_orbit_dir(orbit_id)?;
        let mod_root = root.join("mod");
        let target = mod_root.join(parts[0]);
        let metadata = match std::fs::symlink_metadata(&target) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };

        let resolved = canonicalize_existing_under(&mod_root, &target)?;
        if resolved == mod_root.canonicalize()? {
            bail!("arkorbit: refusing to remove the mod root");
        }

        if metadata.file_type().is_symlink() || metadata.is_file() {
            std::fs::remove_file(&target)?;
        } else if metadata.is_dir() {
            std::fs::remove_dir_all(&target)?;
        } else {
            std::fs::remove_file(&target)?;
        }
        Ok(true)
    }

    pub fn resolve_module(&self, orbit_id: &str, mod_path: &str) -> Result<Option<ResolvedModule>> {
        Self::validate_orbit_id(orbit_id)?;
        let cleaned = clean_relative_path(mod_path)?;
        let parts = path_parts(&cleaned);
        if parts.is_empty() {
            return Ok(None);
        }

        if parts[0] == "runtime" || parts[0] == "skills" {
            return self.resolve_l0_path(&cleaned);
        }

        if parts[0] == "data" || parts[0] == "assets" {
            let root = self.ensure_orbit_dir(orbit_id)?;
            let l2_candidate = root.join(&cleaned);
            if l2_candidate.is_file() {
                let resolved = canonicalize_existing_under(&root, &l2_candidate)?;
                return Ok(Some(ResolvedModule {
                    content_type: content_type_for_path(&resolved),
                    bytes: std::fs::read(resolved)?,
                    layer: ModuleLayer::L2,
                }));
            }
            return Ok(None);
        }

        let module_parts = if parts[0] == "mod" {
            &parts[1..]
        } else {
            &parts[..]
        };
        if module_parts.len() < 2 {
            return Ok(None);
        }

        let l2_rel = PathBuf::from("mod").join(module_parts.join("/"));
        let root = self.ensure_orbit_dir(orbit_id)?;
        let l2_candidate = root.join(&l2_rel);
        if l2_candidate.is_file() {
            let resolved = canonicalize_existing_under(&root, &l2_candidate)?;
            return Ok(Some(ResolvedModule {
                content_type: content_type_for_path(&resolved),
                bytes: std::fs::read(resolved)?,
                layer: ModuleLayer::L2,
            }));
        }

        let l0_rel = PathBuf::from("widgets").join(module_parts.join("/"));
        self.resolve_l0_path(&l0_rel)
    }

    fn resolve_l0_path(&self, rel: &Path) -> Result<Option<ResolvedModule>> {
        let rel = clean_relative_path(rel.to_string_lossy().as_ref())?;
        for root in &self.l0_roots {
            let candidate = root.join(&rel);
            if !candidate.is_file() {
                continue;
            }
            let root_canon = match root.canonicalize() {
                Ok(root) => root,
                Err(_) => continue,
            };
            let resolved = canonicalize_existing_under(&root_canon, &candidate)?;
            return Ok(Some(ResolvedModule {
                content_type: content_type_for_path(&resolved),
                bytes: std::fs::read(resolved)?,
                layer: ModuleLayer::L0Disk,
            }));
        }
        let key = rel.to_string_lossy().replace('\\', "/");
        Ok(embedded_l0(&key).map(|content| ResolvedModule {
            bytes: content.as_bytes().to_vec(),
            content_type: content_type_for_name(&key).to_string(),
            layer: ModuleLayer::L0Embedded,
        }))
    }

    fn atomic_write_under_orbit(&self, orbit_id: &str, path: &Path, content: &[u8]) -> Result<()> {
        let root = self.ensure_orbit_dir(orbit_id)?;
        let root_canon = root.canonicalize()?;
        let Some(parent) = path.parent() else {
            bail!("arkorbit: file path has no parent");
        };
        std::fs::create_dir_all(parent)?;
        let parent_canon = parent.canonicalize()?;
        if !parent_canon.starts_with(&root_canon) {
            bail!("arkorbit: target path escapes orbit directory");
        }
        let tmp_dir = root.join(".tmp");
        std::fs::create_dir_all(&tmp_dir)?;
        let tmp = tmp_dir.join(format!("{}.tmp", Uuid::new_v4()));
        std::fs::write(&tmp, content)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

pub fn validate_writable_orbit_path(raw: &str) -> Result<PathBuf> {
    let path = clean_relative_path(raw)?;
    let parts = path_parts(&path);
    match parts.as_slice() {
        ["index.html"] | ["orbit.json"] => Ok(path),
        [prefix, ..] if matches!(*prefix, "mod" | "data" | "assets") => Ok(path),
        _ => bail!("arkorbit: path must be index.html, orbit.json, or under mod/, data/, assets/"),
    }
}

pub fn validate_readable_orbit_path(raw: &str) -> Result<PathBuf> {
    let path = clean_relative_path(raw)?;
    let normalized = path.to_string_lossy().replace('\\', "/");
    if orbit_file_is_system_path(&normalized) {
        bail!("arkorbit: system chat files are not exposed through the orbit file API");
    }
    let parts = path_parts(&path);
    match parts.as_slice() {
        ["index.html"] | ["orbit.json"] => Ok(path),
        [prefix, ..] if matches!(*prefix, "mod" | "data" | "assets") => Ok(path),
        _ => bail!("arkorbit: readable path must be inside the orbit file namespace"),
    }
}

pub fn orbit_file_is_user_artifact_path(path: &str) -> bool {
    if orbit_file_is_system_path(path) {
        return false;
    }
    path.starts_with("mod/") || path.starts_with("assets/") || path.starts_with("data/")
}

fn orbit_file_is_system_path(path: &str) -> bool {
    let normalized = path.trim().replace('\\', "/");
    normalized.is_empty()
        || normalized.starts_with(".tmp/")
        || normalized == "index.html"
        || normalized == "orbit.json"
        || normalized == "messages.jsonl"
        || normalized == "data/chat-session.txt"
        || normalized.starts_with("data/chat-history/")
}

fn clean_relative_path(raw: &str) -> Result<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("arkorbit: path must be non-empty");
    }
    if trimmed.as_bytes().contains(&0) {
        bail!("arkorbit: path contains a NUL byte");
    }
    let path = Path::new(trimmed);
    if path.is_absolute() || trimmed.starts_with('/') || trimmed.starts_with('\\') {
        bail!("arkorbit: path must be relative");
    }

    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => out.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("arkorbit: path may not escape its namespace")
            }
        }
    }
    if out.as_os_str().is_empty() {
        bail!("arkorbit: path must contain at least one segment");
    }
    Ok(out)
}

fn path_parts(path: &Path) -> Vec<&str> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect()
}

fn canonicalize_existing_under(root: &Path, candidate: &Path) -> Result<PathBuf> {
    let root_canon = root.canonicalize()?;
    let candidate_canon = candidate.canonicalize()?;
    if !candidate_canon.starts_with(&root_canon) {
        bail!("arkorbit: resolved path escapes root");
    }
    Ok(candidate_canon)
}

fn validate_orbit_file_content(root: &Path, rel: &Path, content: &[u8]) -> Result<()> {
    validate_orbit_json_content(rel, content)?;
    if !is_browser_javascript_path(rel) {
        return Ok(());
    }
    let source = std::str::from_utf8(content)
        .with_context(|| format!("arkorbit: {} must be valid UTF-8", rel.display()))?;
    validate_widget_module_contract(rel, source)?;
    validate_browser_javascript_syntax(root, rel, content)
}

fn validate_orbit_json_content(rel: &Path, content: &[u8]) -> Result<()> {
    let normalized = rel.to_string_lossy().replace('\\', "/");
    if normalized != "data/widgets.json" {
        return Ok(());
    }
    let source = std::str::from_utf8(content)
        .with_context(|| format!("arkorbit: {} must be valid UTF-8", rel.display()))?;
    validate_widget_registry_json(source)
}

fn validate_widget_registry_json(source: &str) -> Result<()> {
    let parsed = serde_json::from_str::<serde_json::Value>(source)
        .context("arkorbit: data/widgets.json must be valid JSON")?;
    let widgets = parsed
        .as_array()
        .or_else(|| parsed.get("widgets").and_then(|value| value.as_array()))
        .ok_or_else(|| {
            anyhow!("arkorbit: data/widgets.json must be an array or an object with a widgets array")
        })?;
    for (index, widget) in widgets.iter().enumerate() {
        let object = widget.as_object().ok_or_else(|| {
            anyhow!(
                "arkorbit: data/widgets.json widget at index {} must be an object",
                index
            )
        })?;
        for key in ["id", "module", "title"] {
            if let Some(value) = object.get(key) {
                if !value.is_string() {
                    bail!(
                        "arkorbit: data/widgets.json widget {} field '{}' must be a string",
                        index,
                        key
                    );
                }
            }
        }
        for key in ["left", "top", "width", "height"] {
            if let Some(value) = object.get(key) {
                let Some(number) = value.as_f64() else {
                    bail!(
                        "arkorbit: data/widgets.json widget {} field '{}' must be a finite number",
                        index,
                        key
                    );
                };
                if !number.is_finite() || number < 0.0 || number > 1_000_000.0 {
                    bail!(
                        "arkorbit: data/widgets.json widget {} field '{}' must be between 0 and 1000000",
                        index,
                        key
                    );
                }
            }
        }
    }
    Ok(())
}

fn is_browser_javascript_path(rel: &Path) -> bool {
    matches!(
        rel.extension().and_then(|value| value.to_str()),
        Some("js" | "mjs")
    )
}

fn is_widget_entry_module_path(rel: &Path) -> bool {
    let parts = path_parts(rel);
    matches!(parts.as_slice(), ["mod", _, "index.js" | "index.mjs"])
}

fn validate_widget_module_contract(rel: &Path, source: &str) -> Result<()> {
    if !is_widget_entry_module_path(rel) {
        return Ok(());
    }
    if has_named_render_export(source) {
        Ok(())
    } else {
        bail!(
            "arkorbit: widget module {} must export a named render(el, ctx) function",
            rel.display()
        )
    }
}

fn has_named_render_export(source: &str) -> bool {
    let stripped = strip_javascript_comments_and_strings(source);
    contains_exported_render_declaration(&stripped) || contains_render_in_export_list(&stripped)
}

fn contains_exported_render_declaration(source: &str) -> bool {
    let normalized = source.split_whitespace().collect::<Vec<_>>().join(" ");
    [
        "export function render",
        "export async function render",
        "export const render",
        "export let render",
        "export var render",
    ]
    .iter()
    .any(|needle| contains_phrase_with_identifier_boundary(&normalized, needle))
}

fn contains_phrase_with_identifier_boundary(source: &str, phrase: &str) -> bool {
    let mut offset = 0usize;
    while let Some(relative) = source[offset..].find(phrase) {
        let start = offset + relative;
        let end = start + phrase.len();
        let before = source[..start].chars().next_back();
        let after = source[end..].chars().next();
        let before_ok = before.map_or(true, |ch| !is_javascript_identifier_continue(ch));
        let after_ok = after.map_or(true, |ch| !is_javascript_identifier_continue(ch));
        if before_ok && after_ok {
            return true;
        }
        offset = end;
    }
    false
}

fn is_javascript_identifier_continue(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_ascii_alphanumeric()
}

fn contains_render_in_export_list(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut index = 0usize;
    while let Some(relative) = source[index..].find("export") {
        index += relative + "export".len();
        let rest = source[index..].trim_start();
        if !rest.starts_with('{') {
            continue;
        }
        let offset = source[index..].find('{').unwrap_or(0);
        let start = index + offset + 1;
        let mut depth = 1i32;
        let mut end = start;
        while end < bytes.len() {
            match bytes[end] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            end += 1;
        }
        if depth == 0 {
            let list = &source[start..end];
            if list
                .split(',')
                .map(str::trim)
                .any(export_specifier_exports_render)
            {
                return true;
            }
            index = end + 1;
        } else {
            break;
        }
    }
    false
}

fn export_specifier_exports_render(specifier: &str) -> bool {
    let parts = specifier.split_whitespace().collect::<Vec<_>>();
    match parts.as_slice() {
        ["render"] => true,
        [_, "as", "render"] => true,
        _ => false,
    }
}

fn strip_javascript_comments_and_strings(source: &str) -> String {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mode {
        Code,
        LineComment,
        BlockComment,
        Single,
        Double,
        Template,
    }

    let mut out = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut i = 0usize;
    let mut mode = Mode::Code;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        let next = bytes.get(i + 1).copied().map(char::from);
        match mode {
            Mode::Code => match (ch, next) {
                ('/', Some('/')) => {
                    out.push(' ');
                    out.push(' ');
                    i += 2;
                    mode = Mode::LineComment;
                    continue;
                }
                ('/', Some('*')) => {
                    out.push(' ');
                    out.push(' ');
                    i += 2;
                    mode = Mode::BlockComment;
                    continue;
                }
                ('\'', _) => {
                    out.push(' ');
                    mode = Mode::Single;
                }
                ('"', _) => {
                    out.push(' ');
                    mode = Mode::Double;
                }
                ('`', _) => {
                    out.push(' ');
                    mode = Mode::Template;
                }
                _ => out.push(ch),
            },
            Mode::LineComment => {
                if ch == '\n' {
                    out.push('\n');
                    mode = Mode::Code;
                } else {
                    out.push(' ');
                }
            }
            Mode::BlockComment => {
                if ch == '*' && next == Some('/') {
                    out.push(' ');
                    out.push(' ');
                    i += 2;
                    mode = Mode::Code;
                    continue;
                }
                out.push(if ch == '\n' { '\n' } else { ' ' });
            }
            Mode::Single | Mode::Double | Mode::Template => {
                let quote = match mode {
                    Mode::Single => '\'',
                    Mode::Double => '"',
                    Mode::Template => '`',
                    _ => unreachable!(),
                };
                if ch == '\\' {
                    out.push(' ');
                    if i + 1 < bytes.len() {
                        out.push(' ');
                        i += 2;
                        continue;
                    }
                } else if ch == quote {
                    out.push(' ');
                    mode = Mode::Code;
                } else {
                    out.push(if ch == '\n' { '\n' } else { ' ' });
                }
            }
        }
        i += 1;
    }
    out
}

fn validate_browser_javascript_syntax(root: &Path, rel: &Path, content: &[u8]) -> Result<()> {
    let tmp_dir = root.join(".tmp");
    std::fs::create_dir_all(&tmp_dir)?;
    let tmp = tmp_dir.join(format!("syntax-{}.mjs", Uuid::new_v4()));
    std::fs::write(&tmp, content)?;
    let outcome = run_node_syntax_check(&tmp);
    let _ = std::fs::remove_file(&tmp);
    match outcome {
        Ok(()) => Ok(()),
        Err(error) if is_node_not_found_error(&error) => {
            tracing::warn!(
                target: "arkorbit.fs",
                path = %rel.display(),
                "Skipping ArkOrbit JavaScript syntax validation because node is unavailable"
            );
            Ok(())
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "arkorbit: refusing to write invalid browser JavaScript to {}",
                rel.display()
            )
        }),
    }
}

fn run_node_syntax_check(path: &Path) -> Result<()> {
    let mut command = std::process::Command::new("node");
    command
        .arg("--check")
        .arg(path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => bail!("node_not_found"),
        Err(error) => return Err(error.into()),
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if child.try_wait()?.is_some() {
            let output = child.wait_with_output()?;
            if output.status.success() {
                return Ok(());
            }
            bail!("{}", summarize_process_output(&output));
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("syntax validation timed out");
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

fn is_node_not_found_error(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.to_string() == "node_not_found")
}

fn summarize_process_output(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let summary = stderr
        .lines()
        .chain(stdout.lines())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join(" ");
    if summary.is_empty() {
        format!("node --check exited with status {}", output.status)
    } else {
        summary
    }
}

fn embedded_l0(path: &str) -> Option<&'static str> {
    match path {
        "runtime/host.js" => Some(HOST_JS),
        "runtime/mod-resolver.js" => Some(MOD_RESOLVER_JS),
        "runtime/sse.js" => Some(SSE_JS),
        "skills/SKILL.md" => Some(SKILL_MD),
        "widgets/markdown/manifest.json" => Some(MARKDOWN_MANIFEST),
        "widgets/markdown/index.js" => Some(MARKDOWN_INDEX),
        "widgets/iframe-html/manifest.json" => Some(IFRAME_HTML_MANIFEST),
        "widgets/iframe-html/index.js" => Some(IFRAME_HTML_INDEX),
        "widgets/chart/manifest.json" => Some(CHART_MANIFEST),
        "widgets/chart/index.js" => Some(CHART_INDEX),
        "widgets/table/manifest.json" => Some(TABLE_MANIFEST),
        "widgets/table/index.js" => Some(TABLE_INDEX),
        "widgets/todo/manifest.json" => Some(TODO_MANIFEST),
        "widgets/todo/index.js" => Some(TODO_INDEX),
        "widgets/fetch-proxy/manifest.json" => Some(FETCH_PROXY_MANIFEST),
        "widgets/fetch-proxy/index.js" => Some(FETCH_PROXY_INDEX),
        "widgets/app-shell/manifest.json" => Some(APP_SHELL_MANIFEST),
        "widgets/app-shell/index.js" => Some(APP_SHELL_INDEX),
        _ => None,
    }
}

pub fn content_type_for_path(path: &Path) -> String {
    content_type_for_name(path.to_string_lossy().as_ref()).to_string()
}

pub fn content_type_for_name(name: &str) -> &'static str {
    match Path::new(name).extension().and_then(|value| value.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("txt") | Some("md") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn default_index_html(orbit_id: &str) -> String {
    format!(
        r##"<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>Orbit</title>
  <style>
    :root {{ color-scheme: dark; font-family: Inter, system-ui, sans-serif; }}
    body {{ margin: 0; min-width: 12000px; min-height: 8000px; background: #05070a; color: #edf7f4; overflow: auto; }}
    #app {{ position: relative; min-width: 12000px; min-height: 8000px; box-sizing: border-box; background: #05070a; }}
    .orbit-empty-canvas {{ position: absolute; inset: 0; overflow: hidden; background-image: linear-gradient(rgba(54,245,184,.09) 1px, transparent 1px), linear-gradient(90deg, rgba(54,245,184,.07) 1px, transparent 1px), linear-gradient(rgba(88,224,255,.035) 1px, transparent 1px), linear-gradient(90deg, rgba(88,224,255,.035) 1px, transparent 1px); background-size: 48px 48px, 48px 48px, 240px 240px, 240px 240px; }}
    .orbit-empty-canvas::before {{ content: ""; position: absolute; inset: 40px; border: 1px solid rgba(54,245,184,.18); background: linear-gradient(90deg, transparent 0 28px, rgba(54,245,184,.16) 28px 29px, transparent 29px calc(100% - 29px), rgba(54,245,184,.16) calc(100% - 29px) calc(100% - 28px), transparent calc(100% - 28px)), linear-gradient(transparent 0 28px, rgba(54,245,184,.14) 28px 29px, transparent 29px calc(100% - 29px), rgba(54,245,184,.14) calc(100% - 29px) calc(100% - 28px), transparent calc(100% - 28px)); pointer-events: none; }}
    .orbit-empty-canvas::after {{ content: ""; position: absolute; inset: 0; background: repeating-linear-gradient(0deg, rgba(237,247,244,.018) 0 1px, transparent 1px 5px); opacity: .55; pointer-events: none; }}
    .orbit-empty-topline {{ position: absolute; top: 28px; left: 28px; right: 28px; display: flex; justify-content: space-between; color: rgba(237,247,244,.58); font-size: 12px; letter-spacing: .1em; text-transform: uppercase; }}
    .orbit-empty-topline span {{ padding: 7px 10px; border-radius: 6px; border: 1px solid rgba(237,247,244,.1); background: rgba(7,13,18,.72); }}
    .orbit-empty-reticle {{ position: absolute; left: 50%; top: 46%; width: 280px; height: 280px; transform: translate(-50%, -50%); border: 1px solid rgba(54,245,184,.2); clip-path: polygon(0 0, 34% 0, 34% 6px, 6px 6px, 6px 34%, 0 34%, 0 0, 66% 0, 66% 6px, calc(100% - 6px) 6px, calc(100% - 6px) 34%, 100% 34%, 100% 0, 100% 66%, calc(100% - 6px) 66%, calc(100% - 6px) calc(100% - 6px), 66% calc(100% - 6px), 66% 100%, 100% 100%, 34% 100%, 34% calc(100% - 6px), 6px calc(100% - 6px), 6px 66%, 0 66%, 0 100%); opacity: .9; }}
    .orbit-empty-reticle span {{ position: absolute; background: rgba(88,224,255,.36); }}
    .orbit-empty-reticle span:nth-child(1) {{ left: 50%; top: 20px; bottom: 20px; width: 1px; }}
    .orbit-empty-reticle span:nth-child(2) {{ top: 50%; left: 20px; right: 20px; height: 1px; }}
    .orbit-empty-reticle span:nth-child(3) {{ left: 50%; top: 50%; width: 34px; height: 34px; transform: translate(-50%, -50%); border: 1px solid rgba(54,245,184,.42); background: transparent; }}
    .orbit-empty-reticle span:nth-child(4) {{ left: 50%; top: 50%; width: 8px; height: 8px; transform: translate(-50%, -50%); }}
    .orbit-empty-rail {{ position: absolute; background: rgba(54,245,184,.18); }}
    .orbit-empty-rail-left {{ left: 88px; top: 170px; width: 1px; height: 520px; }}
    .orbit-empty-rail-bottom {{ left: 170px; bottom: 88px; width: 640px; height: 1px; }}
    .orbit-empty-node {{ position: absolute; width: 11px; height: 11px; border: 1px solid rgba(88,224,255,.42); background: #05070a; box-shadow: 0 0 18px rgba(54,245,184,.28); }}
    .orbit-empty-node-a {{ left: 83px; top: 218px; }}
    .orbit-empty-node-b {{ left: 364px; bottom: 83px; }}
    .orbit-empty-node-c {{ right: 220px; top: 310px; }}
    h1 {{ margin: 0 0 10px; font-size: 44px; letter-spacing: 0; }}
    p {{ margin: 0; color: rgba(237, 247, 244, .76); }}
    code {{ background: rgba(255,255,255,.08); border-radius: 6px; padding: 2px 6px; }}
    svg {{ width: 100%; height: auto; fill: #36f5b8; }}
    table {{ border-collapse: collapse; min-width: 320px; }}
    th, td {{ border: 1px solid rgba(255,255,255,.18); padding: 8px 10px; text-align: left; }}
    input, button {{ min-height: 40px; border-radius: 6px; border: 1px solid rgba(255,255,255,.2); }}
    @media (max-width: 540px) {{
      h1 {{ font-size: 32px; }}
    }}
  </style>
  <script>window.__ARKORBIT_ORBIT_ID = "{orbit_id}";</script>
  <script src="/api/arkorbit/mod/{orbit_id}/runtime/host.js"></script>
</head>
<body>
  <main id="app"></main>
  <script type="module">
    window.__arkorbit.mount("markdown").catch((error) => {{
      document.querySelector("#app").textContent = error.message || String(error);
    }});
  </script>
</body>
</html>"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn writable_path_rejects_traversal() {
        let err = validate_writable_orbit_path("mod/../../x.js").unwrap_err();
        assert!(err.to_string().contains("escape"));
    }

    #[test]
    fn writable_path_accepts_mod_assets_data_and_index() {
        validate_writable_orbit_path("mod/widget/index.js").unwrap();
        validate_writable_orbit_path("assets/style.css").unwrap();
        validate_writable_orbit_path("data/state.json").unwrap();
        validate_writable_orbit_path("index.html").unwrap();
    }

    #[test]
    fn orbit_file_listing_hides_system_chat_and_manifest_files() {
        let tmp = TempDir::new().unwrap();
        let store = LayeredStore::new(tmp.path());
        let orbit_id = Uuid::new_v4().to_string();
        let root = store.ensure_orbit_dir(&orbit_id).unwrap();

        std::fs::write(root.join("messages.jsonl"), b"{}").unwrap();
        std::fs::write(root.join("index.html"), b"<html></html>").unwrap();
        std::fs::write(root.join("orbit.json"), b"{}").unwrap();
        std::fs::create_dir_all(root.join("data").join("chat-history")).unwrap();
        std::fs::write(
            root.join("data").join("chat-session.txt"),
            orbit_id.as_bytes(),
        )
        .unwrap();
        std::fs::write(
            root.join("data").join("chat-history").join("session.jsonl"),
            b"{}",
        )
        .unwrap();
        std::fs::write(root.join("data").join("widgets.json"), b"[]").unwrap();
        std::fs::create_dir_all(root.join("mod").join("demo")).unwrap();
        std::fs::write(root.join("mod").join("demo").join("index.js"), b"").unwrap();

        let files = store.list_orbit_files(&orbit_id).unwrap();
        let paths = files
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>();

        assert_eq!(paths, vec!["data/widgets.json", "mod/demo/index.js"]);
    }

    #[test]
    fn generic_file_read_rejects_system_chat_files() {
        assert!(validate_readable_orbit_path("messages.jsonl").is_err());
        assert!(validate_readable_orbit_path("data/chat-session.txt").is_err());
        assert!(validate_readable_orbit_path("data/chat-history/session.jsonl").is_err());
        validate_readable_orbit_path("data/widgets.json").unwrap();
        validate_readable_orbit_path("mod/demo/index.js").unwrap();
    }

    #[test]
    fn resolver_prefers_l2_module_over_l0() {
        let tmp = TempDir::new().unwrap();
        let store = LayeredStore::new(tmp.path());
        let orbit_id = Uuid::new_v4().to_string();
        store
            .write_orbit_file(
                &orbit_id,
                "mod/markdown/index.js",
                b"export const l2 = true; export function render(el, ctx) { el.textContent = String(ctx?.title || 'l2'); }",
            )
            .unwrap();
        let resolved = store
            .resolve_module(&orbit_id, "markdown/index.js")
            .unwrap()
            .unwrap();
        assert_eq!(resolved.layer, ModuleLayer::L2);
        assert!(String::from_utf8_lossy(&resolved.bytes).contains("l2"));
    }

    #[test]
    fn javascript_write_rejects_invalid_browser_syntax_when_validator_is_available() {
        if !node_syntax_validator_available() {
            return;
        }
        let tmp = TempDir::new().unwrap();
        let store = LayeredStore::new(tmp.path());
        let orbit_id = Uuid::new_v4().to_string();

        let err = store
            .write_orbit_file(
                &orbit_id,
                "mod/broken/index.js",
                b"export function render(el { el.textContent = 'broken'; }",
            )
            .unwrap_err();

        assert!(err.to_string().contains("invalid browser JavaScript"));
        assert!(!store
            .orbit_dir(&orbit_id)
            .join("mod")
            .join("broken")
            .join("index.js")
            .exists());
    }

    #[test]
    fn widget_module_write_requires_named_render_export() {
        let tmp = TempDir::new().unwrap();
        let store = LayeredStore::new(tmp.path());
        let orbit_id = Uuid::new_v4().to_string();

        let err = store
            .write_orbit_file(
                &orbit_id,
                "mod/broken/index.js",
                b"export default function Widget() {}",
            )
            .unwrap_err();

        assert!(err.to_string().contains("must export a named render"));
        assert!(!store
            .orbit_dir(&orbit_id)
            .join("mod")
            .join("broken")
            .join("index.js")
            .exists());
    }

    #[test]
    fn widget_module_contract_accepts_common_named_render_exports() {
        assert!(has_named_render_export(
            "export function render(el, ctx) {}"
        ));
        assert!(has_named_render_export(
            "async function draw() {}\nexport { draw as render };"
        ));
        assert!(has_named_render_export(
            "const render = () => {};\nexport { render };"
        ));
        assert!(!has_named_render_export(
            "export default function render() {}"
        ));
        assert!(!has_named_render_export(
            "// export function render() {}\nexport const other = 1;"
        ));
    }

    #[test]
    fn resolver_serves_embedded_runtime() {
        let tmp = TempDir::new().unwrap();
        let store = LayeredStore::new(tmp.path());
        let orbit_id = Uuid::new_v4().to_string();
        let resolved = store
            .resolve_module(&orbit_id, "runtime/host.js")
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&resolved.bytes).contains("__arkorbit"));
    }

    #[test]
    fn resolver_serves_orbit_data_files() {
        let tmp = TempDir::new().unwrap();
        let store = LayeredStore::new(tmp.path());
        let orbit_id = Uuid::new_v4().to_string();
        store
            .write_orbit_file(&orbit_id, "data/widgets.json", br#"[{"id":"demo"}]"#)
            .unwrap();
        let resolved = store
            .resolve_module(&orbit_id, "data/widgets.json")
            .unwrap()
            .unwrap();
        assert_eq!(resolved.layer, ModuleLayer::L2);
        assert_eq!(resolved.content_type, "application/json; charset=utf-8");
    }

    #[test]
    fn widget_registry_write_rejects_invalid_json() {
        let tmp = TempDir::new().unwrap();
        let store = LayeredStore::new(tmp.path());
        let orbit_id = Uuid::new_v4().to_string();

        let err = store
            .write_orbit_file(&orbit_id, "data/widgets.json", br#"[{"id":"demo"}"#)
            .unwrap_err();

        assert!(err.to_string().contains("must be valid JSON"));
        assert!(!store
            .orbit_dir(&orbit_id)
            .join("data")
            .join("widgets.json")
            .exists());
    }

    #[test]
    fn widget_registry_write_accepts_array_or_wrapped_widgets() {
        let tmp = TempDir::new().unwrap();
        let store = LayeredStore::new(tmp.path());
        let orbit_id = Uuid::new_v4().to_string();

        store
            .write_orbit_file(
                &orbit_id,
                "data/widgets.json",
                br#"[{"id":"demo","module":"demo","left":12}]"#,
            )
            .unwrap();
        store
            .write_orbit_file(
                &orbit_id,
                "data/widgets.json",
                br#"{"widgets":[{"id":"demo","module":"demo","top":16}]}"#,
            )
            .unwrap();
    }

    #[test]
    fn widget_registry_write_rejects_non_widget_shapes() {
        let tmp = TempDir::new().unwrap();
        let store = LayeredStore::new(tmp.path());
        let orbit_id = Uuid::new_v4().to_string();

        let err = store
            .write_orbit_file(&orbit_id, "data/widgets.json", br#"{"items":[]}"#)
            .unwrap_err();

        assert!(err.to_string().contains("widgets array"));
    }

    #[test]
    fn remove_orbit_module_dir_removes_l2_module() {
        let tmp = TempDir::new().unwrap();
        let store = LayeredStore::new(tmp.path());
        let orbit_id = Uuid::new_v4().to_string();
        store
            .write_orbit_file(
                &orbit_id,
                "mod/demo/index.js",
                b"export function render() {}",
            )
            .unwrap();

        assert!(store.remove_orbit_module_dir(&orbit_id, "demo").unwrap());
        assert!(!store.orbit_dir(&orbit_id).join("mod").join("demo").exists());
        assert!(!store.remove_orbit_module_dir(&orbit_id, "demo").unwrap());
    }

    #[test]
    fn remove_orbit_module_dir_rejects_nested_paths() {
        let tmp = TempDir::new().unwrap();
        let store = LayeredStore::new(tmp.path());
        let orbit_id = Uuid::new_v4().to_string();

        let err = store
            .remove_orbit_module_dir(&orbit_id, "demo/index.js")
            .unwrap_err();
        assert!(err.to_string().contains("one path segment"));
    }

    fn node_syntax_validator_available() -> bool {
        std::process::Command::new("node")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}
