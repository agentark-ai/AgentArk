//! Layered filesystem store for ArkOrbit.
//!
//! Resolution is structural and path-based:
//! L2 user files under `<DATA_DIR>/arkorbit/L2/orbits/<id>/` win over L0
//! firmware files under `src/core/arkorbit/l0` during source-tree runs, which
//! win over the embedded L0 fallback compiled into the binary.

use anyhow::{Context, Result, anyhow, bail};
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
            if rel.starts_with(".tmp/") {
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

    pub fn l0_skill_catalog(&self) -> String {
        match self.resolve_l0_path(Path::new("skills").join("SKILL.md").as_path()) {
            Ok(Some(resolved)) => String::from_utf8_lossy(&resolved.bytes).into_owned(),
            _ => SKILL_MD.to_string(),
        }
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
    let parts = path_parts(&path);
    match parts.as_slice() {
        ["index.html"] | ["orbit.json"] | ["messages.jsonl"] => Ok(path),
        [prefix, ..] if matches!(*prefix, "mod" | "data" | "assets") => Ok(path),
        _ => bail!("arkorbit: readable path must be inside the orbit file namespace"),
    }
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
    fn resolver_prefers_l2_module_over_l0() {
        let tmp = TempDir::new().unwrap();
        let store = LayeredStore::new(tmp.path());
        let orbit_id = Uuid::new_v4().to_string();
        store
            .write_orbit_file(
                &orbit_id,
                "mod/markdown/index.js",
                b"export const l2 = true;",
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
            .write_orbit_file(&orbit_id, "data/widgets.json", br#"[{"id":"weather"}]"#)
            .unwrap();
        let resolved = store
            .resolve_module(&orbit_id, "data/widgets.json")
            .unwrap()
            .unwrap();
        assert_eq!(resolved.layer, ModuleLayer::L2);
        assert_eq!(resolved.content_type, "application/json; charset=utf-8");
    }

    #[test]
    fn remove_orbit_module_dir_removes_l2_module() {
        let tmp = TempDir::new().unwrap();
        let store = LayeredStore::new(tmp.path());
        let orbit_id = Uuid::new_v4().to_string();
        store
            .write_orbit_file(
                &orbit_id,
                "mod/weather/index.js",
                b"export function render() {}",
            )
            .unwrap();

        assert!(store.remove_orbit_module_dir(&orbit_id, "weather").unwrap());
        assert!(
            !store
                .orbit_dir(&orbit_id)
                .join("mod")
                .join("weather")
                .exists()
        );
        assert!(!store.remove_orbit_module_dir(&orbit_id, "weather").unwrap());
    }

    #[test]
    fn remove_orbit_module_dir_rejects_nested_paths() {
        let tmp = TempDir::new().unwrap();
        let store = LayeredStore::new(tmp.path());
        let orbit_id = Uuid::new_v4().to_string();

        let err = store
            .remove_orbit_module_dir(&orbit_id, "weather/index.js")
            .unwrap_err();
        assert!(err.to_string().contains("one path segment"));
    }
}
