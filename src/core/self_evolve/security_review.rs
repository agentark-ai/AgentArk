//! Post-change security scanner for self-evolve.
//!
//! Scans all changed files before committing to detect
//! security issues in generated code.

use std::path::Path;

/// Severity of a security finding.
#[derive(Debug, Clone, PartialEq)]
pub enum Severity {
    /// Informational warning.
    Warning,
    /// Blocking finding.
    Block,
}

/// A single security finding.
#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: Severity,
    pub file: String,
    pub line: Option<usize>,
    pub message: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sev = match self.severity {
            Severity::Warning => "WARN",
            Severity::Block => "BLOCK",
        };
        if let Some(line) = self.line {
            write!(f, "[{}] {}:{} — {}", sev, self.file, line, self.message)
        } else {
            write!(f, "[{}] {} — {}", sev, self.file, self.message)
        }
    }
}

/// Review all changed files for security issues.
/// Returns a list of findings. If any are `Block` severity, the
/// evolve session should abort.
pub async fn review(changed_files: &[std::path::PathBuf], project_root: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();

    for file_path in changed_files {
        let relative = file_path
            .strip_prefix(project_root)
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();

        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let content = match tokio::fs::read_to_string(file_path).await {
            Ok(c) => c,
            Err(_) => continue, // Binary or deleted file
        };

        match ext {
            "rs" => check_rust(&content, &relative, &mut findings),
            "ts" | "tsx" | "js" | "jsx" => check_typescript(&content, &relative, &mut findings),
            "toml" => check_toml(&content, &relative, &mut findings),
            _ => {}
        }
    }

    findings
}

/// Check if any findings are blocking.
pub fn has_blocking(findings: &[Finding]) -> bool {
    findings.iter().any(|f| f.severity == Severity::Block)
}

// ---------------------------------------------------------------------------
// Rust-specific checks
// ---------------------------------------------------------------------------

fn check_rust(content: &str, file: &str, findings: &mut Vec<Finding>) {
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        let line_num = i + 1;

        // unsafe blocks
        if trimmed.starts_with("unsafe ") || trimmed.contains("unsafe {") {
            // Allow in test modules
            if !is_in_test_module(content, i) {
                findings.push(Finding {
                    severity: Severity::Block,
                    file: file.to_string(),
                    line: Some(line_num),
                    message: "unsafe block detected in non-test code".to_string(),
                });
            }
        }

        // unwrap() / expect() in non-test code
        if (trimmed.contains(".unwrap()") || trimmed.contains(".expect("))
            && !is_in_test_module(content, i)
            && !trimmed.starts_with("//")
        {
            findings.push(Finding {
                severity: Severity::Warning,
                file: file.to_string(),
                line: Some(line_num),
                message: "unwrap()/expect() in non-test code — prefer ? operator".to_string(),
            });
        }

        // Hardcoded API keys / secrets
        if check_hardcoded_secret(trimmed) {
            findings.push(Finding {
                severity: Severity::Block,
                file: file.to_string(),
                line: Some(line_num),
                message: "Possible hardcoded secret/API key detected".to_string(),
            });
        }

        // Command with string interpolation (shell injection risk)
        if trimmed.contains("Command::new") && trimmed.contains("format!") {
            findings.push(Finding {
                severity: Severity::Warning,
                file: file.to_string(),
                line: Some(line_num),
                message: "Command with format! — potential shell injection".to_string(),
            });
        }

        // Missing timeout on reqwest
        if trimmed.contains("reqwest::Client::new()") {
            // Check if timeout is set nearby (within 5 lines)
            let nearby = content
                .lines()
                .skip(i)
                .take(5)
                .collect::<Vec<_>>()
                .join(" ");
            if !nearby.contains("timeout") {
                findings.push(Finding {
                    severity: Severity::Warning,
                    file: file.to_string(),
                    line: Some(line_num),
                    message: "reqwest::Client without timeout — add .timeout()".to_string(),
                });
            }
        }

        // Raw SQL without parameterization
        if (trimmed.contains("execute(") || trimmed.contains("query("))
            && trimmed.contains("format!")
            && (trimmed.contains("SELECT")
                || trimmed.contains("INSERT")
                || trimmed.contains("UPDATE")
                || trimmed.contains("DELETE"))
        {
            findings.push(Finding {
                severity: Severity::Block,
                file: file.to_string(),
                line: Some(line_num),
                message: "Raw SQL with format! — use parameterized queries".to_string(),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// TypeScript-specific checks
// ---------------------------------------------------------------------------

fn check_typescript(content: &str, file: &str, findings: &mut Vec<Finding>) {
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        let line_num = i + 1;

        // eval()
        if trimmed.contains("eval(") && !trimmed.starts_with("//") {
            findings.push(Finding {
                severity: Severity::Block,
                file: file.to_string(),
                line: Some(line_num),
                message: "eval() detected — XSS/code injection risk".to_string(),
            });
        }

        // innerHTML
        if trimmed.contains("innerHTML") && !trimmed.starts_with("//") {
            findings.push(Finding {
                severity: Severity::Warning,
                file: file.to_string(),
                line: Some(line_num),
                message: "innerHTML assignment — potential XSS".to_string(),
            });
        }

        // Hardcoded secrets
        if check_hardcoded_secret(trimmed) {
            findings.push(Finding {
                severity: Severity::Block,
                file: file.to_string(),
                line: Some(line_num),
                message: "Possible hardcoded secret/API key detected".to_string(),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// TOML checks
// ---------------------------------------------------------------------------

fn check_toml(content: &str, file: &str, findings: &mut Vec<Finding>) {
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        let line_num = i + 1;

        if check_hardcoded_secret(trimmed) {
            findings.push(Finding {
                severity: Severity::Block,
                file: file.to_string(),
                line: Some(line_num),
                message: "Possible hardcoded secret in config file".to_string(),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check if a line looks like it contains a hardcoded secret.
fn check_hardcoded_secret(line: &str) -> bool {
    let line_lower = line.to_lowercase();

    // Patterns that indicate hardcoded secrets
    let secret_patterns = [
        "api_key = \"sk-",
        "api_key = \"ghp_",
        "api_key = \"gho_",
        "api_key = \"xoxb-",
        "api_key = \"xoxp-",
        "password = \"",
        "secret = \"",
        "token = \"eyj",
        "\"bearer ",
    ];

    for pattern in &secret_patterns {
        if line_lower.contains(pattern) {
            // Ignore comments
            let stripped = line.trim();
            if stripped.starts_with("//")
                || stripped.starts_with('#')
                || stripped.starts_with("///")
            {
                return false;
            }
            return true;
        }
    }

    false
}

/// Rough check if line index `i` is inside a #[cfg(test)] module.
fn is_in_test_module(content: &str, line_idx: usize) -> bool {
    // Walk backwards from line_idx looking for #[cfg(test)]
    let preceding: Vec<&str> = content.lines().take(line_idx).collect();
    for line in preceding.iter().rev() {
        let trimmed = line.trim();
        if trimmed == "#[cfg(test)]" {
            return true;
        }
        // Stop at module boundaries
        if (trimmed.starts_with("pub mod ") || trimmed.starts_with("mod "))
            && !trimmed.contains("test")
        {
            return false;
        }
    }
    false
}
