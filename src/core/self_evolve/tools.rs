//! Tool implementations for the self-evolve inner agent.
//!
//! Provides source file operations, build/test commands, web search,
//! and git operations — all sandboxed to the project root.

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use tokio::process::Command;

// ---------------------------------------------------------------------------
// Path validation
// ---------------------------------------------------------------------------

/// Resolve a relative path against the project root, rejecting traversal.
fn safe_resolve(project_root: &Path, relative: &str) -> Result<PathBuf> {
    let cleaned = relative
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string();

    // Block obvious traversal
    if cleaned.contains("..") {
        return Err(anyhow!("Path traversal blocked: {}", relative));
    }

    let resolved = project_root.join(&cleaned);
    let canonical_root = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let canonical = resolved.canonicalize().unwrap_or_else(|_| resolved.clone());

    if !canonical.starts_with(&canonical_root) {
        return Err(anyhow!("Path escapes project root: {}", resolved.display()));
    }

    Ok(resolved)
}

/// Check if a path is a blocked sensitive file.
fn is_blocked_path(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let path_str = path.to_string_lossy();

    // Sensitive files
    name == ".env"
        || name.starts_with(".env.")
        || name == "secrets.enc"
        || name == ".keyfile"
        || ext == "key"
        || ext == "pem"
        || path_str.contains("config/secrets")
}

// ---------------------------------------------------------------------------
// Source file tools
// ---------------------------------------------------------------------------

/// Read a source file. Returns its contents as a string.
pub async fn source_read(project_root: &Path, path: &str) -> Result<String> {
    let resolved = safe_resolve(project_root, path)?;
    if is_blocked_path(&resolved) {
        return Err(anyhow!("Access denied: sensitive file"));
    }
    let content = tokio::fs::read_to_string(&resolved)
        .await
        .map_err(|e| anyhow!("Failed to read {}: {}", path, e))?;
    Ok(content)
}

/// Capture file state before mutation. Returns None when file does not exist.
pub async fn source_capture(project_root: &Path, path: &str) -> Result<Option<String>> {
    let resolved = safe_resolve(project_root, path)?;
    if is_blocked_path(&resolved) {
        return Err(anyhow!("Access denied: sensitive file"));
    }
    if !resolved.exists() {
        return Ok(None);
    }
    let content = tokio::fs::read_to_string(&resolved)
        .await
        .map_err(|e| anyhow!("Failed to read {}: {}", path, e))?;
    Ok(Some(content))
}

/// Write an entire file. Creates parent directories if needed.
pub async fn source_write(project_root: &Path, path: &str, content: &str) -> Result<String> {
    let resolved = safe_resolve(project_root, path)?;
    if is_blocked_path(&resolved) {
        return Err(anyhow!("Access denied: sensitive file"));
    }
    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&resolved, content).await?;
    Ok(format!("Wrote {} ({} bytes)", path, content.len()))
}

/// Apply a search-and-replace edit to a file.
pub async fn source_edit(
    project_root: &Path,
    path: &str,
    search: &str,
    replace: &str,
) -> Result<String> {
    let resolved = safe_resolve(project_root, path)?;
    if is_blocked_path(&resolved) {
        return Err(anyhow!("Access denied: sensitive file"));
    }
    let content = tokio::fs::read_to_string(&resolved)
        .await
        .map_err(|e| anyhow!("Failed to read {}: {}", path, e))?;

    let count = content.matches(search).count();
    if count == 0 {
        return Err(anyhow!(
            "Search string not found in {}. File has {} bytes. First 200 chars:\n{}",
            path,
            content.len(),
            &content[..content.len().min(200)]
        ));
    }

    let updated = content.replacen(search, replace, 1);
    tokio::fs::write(&resolved, &updated).await?;
    Ok(format!(
        "Edited {} (replaced 1 of {} occurrences, {} bytes → {} bytes)",
        path,
        count,
        content.len(),
        updated.len()
    ))
}

/// Restore a file to a previously captured state.
pub async fn source_restore(
    project_root: &Path,
    path: &str,
    original: &Option<String>,
) -> Result<()> {
    let resolved = safe_resolve(project_root, path)?;
    if is_blocked_path(&resolved) {
        return Err(anyhow!("Access denied: sensitive file"));
    }
    match original {
        Some(content) => {
            if let Some(parent) = resolved.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(&resolved, content).await?;
        }
        None => {
            if resolved.exists() {
                tokio::fs::remove_file(&resolved).await?;
            }
        }
    }
    Ok(())
}

/// List directory contents with optional glob pattern.
pub async fn source_list(project_root: &Path, path: &str, pattern: Option<&str>) -> Result<String> {
    let resolved = safe_resolve(project_root, path)?;
    if !resolved.is_dir() {
        return Err(anyhow!("{} is not a directory", path));
    }

    let mut entries = Vec::new();
    let mut read_dir = tokio::fs::read_dir(&resolved).await?;
    while let Some(entry) = read_dir.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(pat) = pattern {
            if !name.contains(pat) {
                continue;
            }
        }
        let meta = entry.metadata().await?;
        let kind = if meta.is_dir() { "dir" } else { "file" };
        let size = if meta.is_file() { meta.len() } else { 0 };
        entries.push(format!("{:<6} {:>8}  {}", kind, size, name));
    }
    entries.sort();
    Ok(entries.join("\n"))
}

/// Search for a pattern across the codebase (simple grep).
pub async fn source_search(
    project_root: &Path,
    pattern: &str,
    glob: Option<&str>,
) -> Result<String> {
    let glob_filter = glob.unwrap_or("*.rs");

    // Prefer ripgrep for cross-platform speed. Fallback to grep when unavailable.
    let output = match Command::new("rg")
        .arg("-n")
        .arg("--glob")
        .arg(glob_filter)
        .arg(pattern)
        .arg(".")
        .current_dir(project_root)
        .output()
        .await
    {
        Ok(o) => o,
        Err(_) => {
            Command::new("grep")
                .arg("-rn")
                .arg("--include")
                .arg(glob_filter)
                .arg(pattern)
                .arg(".")
                .current_dir(project_root)
                .output()
                .await?
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let status_code = output.status.code().unwrap_or(-1);

    if status_code > 1 {
        return Err(anyhow!(
            "Search command failed (status {}): {}",
            status_code,
            stderr.trim()
        ));
    }

    // Limit output to prevent flooding
    let lines: Vec<&str> = stdout.lines().take(50).collect();
    if lines.is_empty() {
        Ok(format!("No matches for '{}' in {}", pattern, glob_filter))
    } else {
        let truncated = if stdout.lines().count() > 50 {
            "\n... (truncated, >50 matches)"
        } else {
            ""
        };
        Ok(format!("{}{}", lines.join("\n"), truncated))
    }
}

// ---------------------------------------------------------------------------
// Build / test commands
// ---------------------------------------------------------------------------

/// Helper: run a command and capture stdout+stderr, with timeout.
async fn run_command(
    program: &str,
    args: &[&str],
    cwd: &Path,
    timeout_secs: u64,
) -> Result<(bool, String)> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        Command::new(program).args(args).current_dir(cwd).output(),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "Command timed out after {}s: {} {}",
            timeout_secs,
            program,
            args.join(" ")
        )
    })?
    .map_err(|e| anyhow!("Failed to run {} {}: {}", program, args.join(" "), e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = if stderr.is_empty() {
        stdout.to_string()
    } else {
        format!("{}\n{}", stdout, stderr)
    };

    // Truncate very long output
    let truncated = if combined.len() > 8000 {
        format!(
            "{}...\n(truncated, {} total bytes)",
            &combined[..8000],
            combined.len()
        )
    } else {
        combined
    };

    Ok((output.status.success(), truncated))
}

/// Run `cargo check` for fast syntax validation.
pub async fn build_check(project_root: &Path) -> Result<String> {
    let (ok, output) = run_command("cargo", &["check"], project_root, 120).await?;
    if ok {
        Ok(format!("cargo check: PASSED\n{}", output))
    } else {
        Ok(format!("cargo check: FAILED\n{}", output))
    }
}

/// Run `cargo test`.
pub async fn run_tests(project_root: &Path) -> Result<String> {
    let (ok, output) = run_command("cargo", &["test"], project_root, 300).await?;
    if ok {
        Ok(format!("cargo test: PASSED\n{}", output))
    } else {
        Ok(format!("cargo test: FAILED\n{}", output))
    }
}

/// Run `cargo clippy -- -D warnings`.
pub async fn lint_check(project_root: &Path) -> Result<String> {
    let (ok, output) = run_command(
        "cargo",
        &["clippy", "--", "-D", "warnings"],
        project_root,
        120,
    )
    .await?;
    if ok {
        Ok(format!("cargo clippy: PASSED\n{}", output))
    } else {
        Ok(format!("cargo clippy: FAILED\n{}", output))
    }
}

/// Run `npm run build` in the frontend directory.
pub async fn frontend_build(project_root: &Path) -> Result<String> {
    let frontend_dir = project_root.join("frontend");
    if !frontend_dir.exists() {
        return Ok("frontend/ directory not found, skipping".to_string());
    }
    let (ok, output) = run_command("npm", &["run", "build"], &frontend_dir, 120).await?;
    if ok {
        Ok(format!("npm run build: PASSED\n{}", output))
    } else {
        Ok(format!("npm run build: FAILED\n{}", output))
    }
}

/// Perform a web search using the project's configured/default search stack.
pub async fn web_search(query: &str) -> Result<String> {
    let args = crate::actions::search::SearchArgs {
        query: query.to_string(),
        num_results: 5,
        backend: None,
    };
    let config = crate::actions::search::SearchConfig::default();
    crate::actions::search::execute_search(&args, &config).await
}

// ---------------------------------------------------------------------------
// Git operations (internal — used by the agent loop, not exposed as LLM tools)
// ---------------------------------------------------------------------------

/// Get the list of files changed (unstaged + staged).
pub async fn git_changed_files(project_root: &Path) -> Result<Vec<PathBuf>> {
    let (_, output) =
        run_command("git", &["diff", "--name-only", "HEAD"], project_root, 10).await?;

    // Also get untracked files
    let (_, untracked) = run_command(
        "git",
        &["ls-files", "--others", "--exclude-standard"],
        project_root,
        10,
    )
    .await?;

    let mut files: Vec<PathBuf> = output
        .lines()
        .chain(untracked.lines())
        .filter(|l| !l.is_empty())
        .map(|l| project_root.join(l.trim()))
        .collect();
    files.sort();
    files.dedup();
    Ok(files)
}

/// Generate a human-readable diff summary.
pub async fn git_diff_summary(project_root: &Path) -> Result<String> {
    let (_, diff) = run_command("git", &["diff", "--stat"], project_root, 10).await?;

    let (_, diff_content) = run_command("git", &["diff"], project_root, 10).await?;

    Ok(format!(
        "Changes summary:\n{}\n\nFull diff:\n{}",
        diff, diff_content
    ))
}

/// Read current branch name for push guidance.
pub async fn git_current_branch(project_root: &Path) -> Result<String> {
    let (_, branch) = run_command(
        "git",
        &["rev-parse", "--abbrev-ref", "HEAD"],
        project_root,
        10,
    )
    .await?;

    let branch = branch.trim();
    if branch.is_empty() {
        return Err(anyhow!("Unable to determine current branch"));
    }
    Ok(branch.to_string())
}
