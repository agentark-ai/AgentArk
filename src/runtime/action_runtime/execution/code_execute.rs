use super::super::*;

impl ActionRuntime {
    /// Resolve a language name to (docker_image, file_extension, build_cmd, run_cmd).
    /// build_cmd is optional (for compiled languages like Java, Go, Rust, C).
    /// Returns None only if the language is completely unrecognized.
    pub(in crate::runtime) fn resolve_language(
        lang: &str,
    ) -> Option<(
        &'static str,
        &'static str,
        Option<&'static str>,
        &'static str,
    )> {
        // (image, extension, optional_build_cmd, run_cmd)
        // {file} is replaced with the sandbox source path at runtime.
        // {sandbox_dir} is replaced with the writable execution workspace.
        match lang {
            // Interpreted
            "python" | "python3" | "py" => Some((
                "python:3-slim",
                "py",
                None,
                "PIP_ROOT_USER_ACTION=ignore PIP_DISABLE_PIP_VERSION_CHECK=1 python3 {file}",
            )),
            "javascript" | "js" | "node" => Some(("node:22-slim", "js", None, "node {file}")),
            "typescript" | "ts" => Some((
                "node:22-slim",
                "ts",
                Some("npm i -g tsx 2>/dev/null"),
                "npx tsx {file}",
            )),
            "bash" | "sh" | "shell" => Some(("bash:latest", "sh", None, "bash {file}")),
            "ruby" | "rb" => Some(("ruby:3-slim", "rb", None, "ruby {file}")),
            "php" => Some(("php:8-cli", "php", None, "php {file}")),
            "perl" | "pl" => Some(("perl:5-slim", "pl", None, "perl {file}")),
            "lua" => Some(("nickblah/lua:5.4", "lua", None, "lua {file}")),
            "r" | "rlang" => Some(("r-base:latest", "R", None, "Rscript {file}")),

            // Compiled
            "java" => Some((
                "eclipse-temurin:21-jdk",
                "java",
                Some("javac {file}"),
                "java -cp {sandbox_dir} Main",
            )),
            "c" => Some((
                "gcc:latest",
                "c",
                Some("gcc {file} -o {sandbox_dir}/a.out -lm"),
                "{sandbox_dir}/a.out",
            )),
            "cpp" | "c++" => Some((
                "gcc:latest",
                "cpp",
                Some("g++ {file} -o {sandbox_dir}/a.out -lm"),
                "{sandbox_dir}/a.out",
            )),
            "go" | "golang" => Some(("golang:1-bookworm", "go", None, "go run {file}")),
            "rust" | "rs" => Some((
                "rust:1-slim-bookworm",
                "rs",
                Some("rustc {file} -o {sandbox_dir}/a.out"),
                "{sandbox_dir}/a.out",
            )),
            "swift" => Some(("swift:latest", "swift", None, "swift {file}")),
            "kotlin" | "kt" => Some((
                "zenika/kotlin:latest",
                "kt",
                Some("kotlinc {file} -include-runtime -d {sandbox_dir}/out.jar 2>/dev/null"),
                "java -jar {sandbox_dir}/out.jar",
            )),

            // Jupyter notebook - execute in-place and output results
            "jupyter" | "notebook" | "ipynb" => Some((
                "python:3-slim",
                "ipynb",
                Some(
                    "PIP_ROOT_USER_ACTION=ignore PIP_DISABLE_PIP_VERSION_CHECK=1 python3 -m pip install --no-cache-dir -q jupyter nbconvert nbformat matplotlib pandas numpy scikit-learn seaborn 2>/dev/null",
                ),
                "jupyter nbconvert --to notebook --execute --inplace {file} 2>&1 && python3 -c \"import json; nb=json.load(open('{file}')); [print(o.get('text','')) for c in nb['cells'] for o in c.get('outputs',[]) if o.get('output_type')=='stream']\" ",
            )),

            _ => None,
        }
    }

    pub(in crate::runtime) fn code_execute_contract_phase(
        arguments: &serde_json::Value,
    ) -> Option<&'static str> {
        let phase = arguments
            .get("execution_contract")
            .and_then(|value| value.get("phase"))
            .and_then(|value| value.as_str())?
            .trim()
            .to_ascii_lowercase();
        match phase.as_str() {
            "bootstrap" => Some("bootstrap"),
            "validate" => Some("validate"),
            "poll" => Some("poll"),
            _ => None,
        }
    }

    pub(in crate::runtime) fn code_execute_contract_flag(
        arguments: &serde_json::Value,
        key: &str,
    ) -> bool {
        arguments
            .get("execution_contract")
            .and_then(|value| value.get(key))
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    }

    pub(in crate::runtime) fn text_contains_network_endpoint(text: &str) -> bool {
        let lower = text.to_ascii_lowercase();
        if lower.contains("://") || lower.contains("localhost") || lower.contains("::1") {
            return true;
        }

        for candidate in lower.split(|c: char| !(c.is_ascii_digit() || c == '.')) {
            let octets: Vec<&str> = candidate.split('.').collect();
            if octets.len() == 4
                && octets
                    .iter()
                    .all(|part| !part.is_empty() && part.len() <= 3 && part.parse::<u8>().is_ok())
            {
                return true;
            }
        }

        false
    }

    pub(in crate::runtime) fn json_value_contains_network_endpoint(
        value: &serde_json::Value,
    ) -> bool {
        match value {
            serde_json::Value::String(text) => Self::text_contains_network_endpoint(text),
            serde_json::Value::Array(values) => values
                .iter()
                .any(Self::json_value_contains_network_endpoint),
            serde_json::Value::Object(values) => values
                .values()
                .any(Self::json_value_contains_network_endpoint),
            _ => false,
        }
    }

    pub(in crate::runtime) fn code_execute_effective_network_access(
        arguments: &serde_json::Value,
    ) -> bool {
        arguments
            .get("network_access")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
            || Self::code_execute_contract_flag(arguments, "target_connectivity_required")
            || Self::json_value_contains_network_endpoint(arguments)
    }

    pub(in crate::runtime) fn build_code_execute_execution_metadata(
        arguments: &serde_json::Value,
        success: bool,
        output_file_count: usize,
    ) -> serde_json::Value {
        let phase = Self::code_execute_contract_phase(arguments);
        let target_validated = success
            && Self::code_execute_contract_flag(arguments, "target_validated_when_successful");
        let explicit_ready_for_watch = success
            && Self::code_execute_contract_flag(arguments, "ready_for_watch_when_successful");
        let ready_for_watch = explicit_ready_for_watch
            || (success && phase == Some("poll"))
            || (success && phase == Some("validate") && target_validated);
        let setup_only = success && phase == Some("bootstrap") && !ready_for_watch;

        serde_json::json!({
            "phase": phase,
            "setup_only": setup_only,
            "target_validated": target_validated,
            "ready_for_watch": ready_for_watch,
            "target_connectivity_required": Self::code_execute_contract_flag(
                arguments,
                "target_connectivity_required",
            ),
            "network_access_requested": Self::code_execute_effective_network_access(arguments),
            "output_file_count": output_file_count,
        })
    }

    pub(in crate::runtime) fn code_execute_high_risk_install_is_explicitly_approved(
        auth_context: &ActionAuthorizationContext,
    ) -> bool {
        auth_context.current_turn_is_explicit_approval
            && auth_context.direct_user_intent
            && auth_context
                .principal
                .as_ref()
                .is_some_and(|principal| principal.trusted)
            && matches!(
                auth_context.surface,
                ActionExecutionSurface::Chat | ActionExecutionSurface::Api
            )
    }

    pub(in crate::runtime) fn detect_risky_code_execute_install_request(
        code: &str,
    ) -> Option<String> {
        let lower = code.to_ascii_lowercase();
        let fetches_remote_script = lower.contains("curl ")
            || lower.contains("curl\t")
            || lower.contains("wget ")
            || lower.contains("wget\t")
            || lower.contains("irm ")
            || lower.contains("iwr ")
            || lower.contains("invoke-webrequest")
            || lower.contains("invoke-restmethod");
        let pipes_to_interpreter = lower.contains("| sh")
            || lower.contains("| bash")
            || lower.contains("| zsh")
            || lower.contains("| powershell")
            || lower.contains("| pwsh")
            || lower.contains("| iex");
        let evals_remote_output =
            (lower.contains("eval $(") || lower.contains("iex (")) && fetches_remote_script;
        if fetches_remote_script && (pipes_to_interpreter || evals_remote_output) {
            return Some("remote script installer".to_string());
        }
        None
    }

    pub(in crate::runtime) fn build_code_execute_dependency_metadata(
        python_packages: &[String],
        node_packages: &[String],
    ) -> serde_json::Value {
        let mut installers = Vec::new();
        if !python_packages.is_empty() {
            installers.push(serde_json::json!({
                "manager": "pip",
                "sandbox_only": true,
                "auto_allowed": true,
                "packages": python_packages,
            }));
        }
        if !node_packages.is_empty() {
            installers.push(serde_json::json!({
                "manager": "npm",
                "sandbox_only": true,
                "auto_allowed": true,
                "packages": node_packages,
            }));
        }
        serde_json::json!({ "installers": installers })
    }

    pub(in crate::runtime) fn code_execute_dependency_summary(
        python_packages: &[String],
        node_packages: &[String],
    ) -> Option<String> {
        let mut sections = Vec::new();
        if !python_packages.is_empty() {
            sections.push(format!("pip: {}", python_packages.join(", ")));
        }
        if !node_packages.is_empty() {
            sections.push(format!("npm: {}", node_packages.join(", ")));
        }
        if sections.is_empty() {
            None
        } else {
            Some(format!(
                "AgentArk auto-installed sandbox dependencies: {}.",
                sections.join("; ")
            ))
        }
    }

    pub(in crate::runtime) fn code_execute_text_payloads(
        output: &str,
        stderr: &str,
    ) -> Vec<serde_json::Value> {
        [("output", output), ("error", stderr)]
            .into_iter()
            .filter_map(|(path, text)| {
                if text.trim().is_empty() {
                    return None;
                }
                Some(serde_json::json!({
                    "path": path,
                    "chars": text.chars().count(),
                    "included_chars": text.chars().count(),
                    "truncated": false,
                    "text": text,
                }))
            })
            .collect()
    }

    /// Detect non-stdlib Python imports and return a pip install command.
    /// Scans `import X` and `from X import` statements, filters out stdlib modules.
    pub(in crate::runtime) fn detect_python_dep_packages(code: &str) -> Vec<String> {
        // Python stdlib modules (comprehensive but not exhaustive - errs on side of not installing)
        const STDLIB: &[&str] = &[
            "abc",
            "aifc",
            "argparse",
            "array",
            "ast",
            "asynchat",
            "asyncio",
            "asyncore",
            "atexit",
            "base64",
            "bdb",
            "binascii",
            "binhex",
            "bisect",
            "builtins",
            "bz2",
            "calendar",
            "cgi",
            "cgitb",
            "chunk",
            "cmath",
            "cmd",
            "code",
            "codecs",
            "codeop",
            "collections",
            "colorsys",
            "compileall",
            "concurrent",
            "configparser",
            "contextlib",
            "contextvars",
            "copy",
            "copyreg",
            "cProfile",
            "crypt",
            "csv",
            "ctypes",
            "curses",
            "dataclasses",
            "datetime",
            "dbm",
            "decimal",
            "difflib",
            "dis",
            "distutils",
            "doctest",
            "email",
            "encodings",
            "enum",
            "errno",
            "faulthandler",
            "fcntl",
            "filecmp",
            "fileinput",
            "fnmatch",
            "formatter",
            "fractions",
            "ftplib",
            "functools",
            "gc",
            "getopt",
            "getpass",
            "gettext",
            "glob",
            "grp",
            "gzip",
            "hashlib",
            "heapq",
            "hmac",
            "html",
            "http",
            "idlelib",
            "imaplib",
            "imghdr",
            "imp",
            "importlib",
            "inspect",
            "io",
            "ipaddress",
            "itertools",
            "json",
            "keyword",
            "lib2to3",
            "linecache",
            "locale",
            "logging",
            "lzma",
            "mailbox",
            "mailcap",
            "marshal",
            "math",
            "mimetypes",
            "mmap",
            "modulefinder",
            "multiprocessing",
            "netrc",
            "nis",
            "nntplib",
            "numbers",
            "operator",
            "optparse",
            "os",
            "ossaudiodev",
            "parser",
            "pathlib",
            "pdb",
            "pickle",
            "pickletools",
            "pipes",
            "pkgutil",
            "platform",
            "plistlib",
            "poplib",
            "posix",
            "posixpath",
            "pprint",
            "profile",
            "pstats",
            "pty",
            "pwd",
            "py_compile",
            "pyclbr",
            "pydoc",
            "queue",
            "quopri",
            "random",
            "re",
            "readline",
            "reprlib",
            "resource",
            "rlcompleter",
            "runpy",
            "sched",
            "secrets",
            "select",
            "selectors",
            "shelve",
            "shlex",
            "shutil",
            "signal",
            "site",
            "smtpd",
            "smtplib",
            "sndhdr",
            "socket",
            "socketserver",
            "ssl",
            "stat",
            "statistics",
            "string",
            "stringprep",
            "struct",
            "subprocess",
            "sunau",
            "symtable",
            "sys",
            "sysconfig",
            "syslog",
            "tabnanny",
            "tarfile",
            "telnetlib",
            "tempfile",
            "termios",
            "test",
            "textwrap",
            "threading",
            "time",
            "timeit",
            "tkinter",
            "token",
            "tokenize",
            "trace",
            "traceback",
            "tracemalloc",
            "tty",
            "turtle",
            "turtledemo",
            "types",
            "typing",
            "unicodedata",
            "unittest",
            "urllib",
            "uu",
            "uuid",
            "venv",
            "warnings",
            "wave",
            "weakref",
            "webbrowser",
            "winreg",
            "winsound",
            "wsgiref",
            "xdrlib",
            "xml",
            "xmlrpc",
            "zipapp",
            "zipfile",
            "zipimport",
            "zlib",
            "_thread",
            "__future__",
        ];

        // Well-known pip package name mappings (import name -> pip package)
        fn pip_name(module: &str) -> String {
            match module {
                "PIL" | "Pillow" => "Pillow".to_string(),
                "cv2" => "opencv-python-headless".to_string(),
                "sklearn" | "scikit_learn" => "scikit-learn".to_string(),
                "bs4" => "beautifulsoup4".to_string(),
                "yaml" => "pyyaml".to_string(),
                "dotenv" => "python-dotenv".to_string(),
                "gi" => "PyGObject".to_string(),
                "attr" | "attrs" => "attrs".to_string(),
                "dateutil" => "python-dateutil".to_string(),
                "jwt" => "PyJWT".to_string(),
                "crypto" | "Crypto" => "pycryptodome".to_string(),
                "serial" => "pyserial".to_string(),
                "usb" => "pyusb".to_string(),
                "wx" => "wxPython".to_string(),
                "skimage" => "scikit-image".to_string(),
                _ => module.to_string(),
            }
        }

        // Valid pip package names: alphanumeric, hyphens, underscores, dots
        fn is_valid_package_name(name: &str) -> bool {
            !name.is_empty()
                && name.len() <= 100
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
                && name.chars().next().is_some_and(|c| c.is_alphanumeric())
        }

        let mut deps = std::collections::HashSet::new();

        for line in code.lines() {
            let line = line.trim();
            // Skip lines inside strings/comments (heuristic: skip if line starts with #, ', ", or is indented code with non-import content)
            if line.starts_with('#') || line.starts_with('"') || line.starts_with('\'') {
                continue;
            }
            // import X, Y, Z
            if let Some(rest) = line.strip_prefix("import ") {
                for part in rest.split(',') {
                    let module = part
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .split('.')
                        .next()
                        .unwrap_or("");
                    if !module.is_empty()
                        && !STDLIB.contains(&module)
                        && is_valid_package_name(module)
                    {
                        deps.insert(pip_name(module));
                    }
                }
            }
            // from X import ...
            else if let Some(rest) = line.strip_prefix("from ") {
                let module = rest
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .split('.')
                    .next()
                    .unwrap_or("");
                if !module.is_empty() && !STDLIB.contains(&module) && is_valid_package_name(module)
                {
                    deps.insert(pip_name(module));
                }
            }
        }

        let mut dep_list: Vec<String> = deps.into_iter().collect();
        dep_list.sort();
        dep_list
    }

    /// Detect non-builtin Node.js requires/imports and return an npm install command.
    pub(in crate::runtime) fn detect_node_dep_packages(code: &str) -> Vec<String> {
        // Node.js built-in modules
        const BUILTINS: &[&str] = &[
            "assert",
            "buffer",
            "child_process",
            "cluster",
            "console",
            "constants",
            "crypto",
            "dgram",
            "dns",
            "domain",
            "events",
            "fs",
            "http",
            "https",
            "module",
            "net",
            "os",
            "path",
            "perf_hooks",
            "process",
            "punycode",
            "querystring",
            "readline",
            "repl",
            "stream",
            "string_decoder",
            "sys",
            "timers",
            "tls",
            "tty",
            "url",
            "util",
            "v8",
            "vm",
            "worker_threads",
            "zlib",
        ];

        let mut deps = std::collections::HashSet::new();

        for line in code.lines() {
            let line = line.trim();
            // require('pkg') or require("pkg")
            if line.contains("require(") {
                for cap in line.split("require(").skip(1) {
                    let pkg = cap
                        .trim_start_matches(['\'', '"'])
                        .split(['\'', '"'])
                        .next()
                        .unwrap_or("");
                    let root = pkg.split('/').next().unwrap_or("");
                    if !root.is_empty() && !root.starts_with('.') && !BUILTINS.contains(&root) {
                        deps.insert(root.to_string());
                    }
                }
            }
            // import ... from 'pkg'
            if line.starts_with("import ") {
                if let Some(from_part) = line.rsplit("from ").next() {
                    let pkg = from_part.trim().trim_matches([' ', '\'', '"', ';']);
                    let root = pkg.split('/').next().unwrap_or("");
                    if !root.is_empty() && !root.starts_with('.') && !BUILTINS.contains(&root) {
                        deps.insert(root.to_string());
                    }
                }
            }
        }

        let mut dep_list: Vec<String> = deps.into_iter().collect();
        dep_list.sort();
        dep_list
    }

    /// Execute code in an isolated Docker container.
    /// Supports any language with a Docker image - auto-pulls if needed.
    /// Container is ephemeral - fully destroyed after execution.
    /// Output files (images, CSVs, etc.) are extracted before container cleanup.
    #[cfg(feature = "docker")]
    pub(in crate::runtime) async fn execute_code_docker(
        &self,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Result<String> {
        if let Err(error) = self.prune_stale_code_execute_artifacts().await {
            tracing::warn!(
                "Failed to prune stale code execution artifacts before sandbox run: {}",
                error
            );
        }
        let language = arguments["language"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'language' argument"))?
            .to_lowercase();
        let code_raw = arguments["code"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'code' argument"))?;
        if let Some(reason) = Self::detect_risky_code_execute_install_request(code_raw) {
            if Self::code_execute_high_risk_install_is_explicitly_approved(auth_context) {
                tracing::info!(
                    "Allowing high-risk code_execute installer after explicit approval turn: {}",
                    reason
                );
            } else {
                anyhow::bail!(
                    "High-risk installer path detected inside `code_execute`: {}. Ordinary sandbox-local pip/npm installs from standard registries are auto-allowed. I did not run this installer automatically. Reply with approval in this chat if you want me to allow this exact installer path, or rewrite it to use standard registry packages inside the sandbox.",
                    reason
                );
            }
        }

        // Strip Jupyter magic commands (!pip, !apt, !conda, %pip, %conda, etc.)
        // LLMs often generate these in regular Python scripts - our auto-dependency
        // detection handles installs, so these lines are unnecessary and cause SyntaxError.
        let code = if matches!(language.as_str(), "python" | "python3" | "py") {
            let cleaned: Vec<&str> = code_raw
                .lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    !trimmed.starts_with("!pip ")
                        && !trimmed.starts_with("!pip3 ")
                        && !trimmed.starts_with("!apt ")
                        && !trimmed.starts_with("!apt-get ")
                        && !trimmed.starts_with("!conda ")
                        && !trimmed.starts_with("%pip ")
                        && !trimmed.starts_with("%conda ")
                        && !trimmed.starts_with("!sudo ")
                })
                .collect();
            cleaned.join("\n")
        } else {
            code_raw.to_string()
        };
        let code = code.as_str();

        let (image, ext, build_cmd, run_cmd) = Self::resolve_language(&language)
            .ok_or_else(|| anyhow::anyhow!(
                "Unsupported language '{}'. Supported: python, javascript, typescript, bash, ruby, php, perl, lua, r, java, c, cpp, go, rust, swift, kotlin",
                language
            ))?;

        let code_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, code);
        let file_path = format!("{}/code.{}", CODE_EXECUTE_SANDBOX_DIR, ext);
        // Java needs the file named Main.java
        let file_path = if language == "java" {
            format!("{}/Main.java", CODE_EXECUTE_SANDBOX_DIR)
        } else {
            file_path
        };

        // Build file injection commands for uploaded files.
        // Upload IDs are resolved through storage, validated against the managed uploads root,
        // then base64-encoded and decoded into /data/ inside the container.
        let sandbox_files = self.collect_code_execute_files(arguments).await?;
        let mut file_inject_cmds = String::new();
        if !sandbox_files.is_empty() {
            file_inject_cmds.push_str("mkdir -p /data && ");
            for upload in sandbox_files {
                let data_b64 = base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &upload.bytes,
                );
                file_inject_cmds.push_str(&format!(
                    "echo '{}' | base64 -d > /data/{} && ",
                    data_b64, upload.filename
                ));
                tracing::info!(
                    "Injecting uploaded file into container: {} ({} bytes)",
                    upload.filename,
                    upload.bytes.len()
                );
            }
        }

        // Auto-detect dependencies for Python/Node and install them inside the sandbox only.
        let python_packages = if matches!(language.as_str(), "python" | "python3" | "py") {
            Self::detect_python_dep_packages(code)
        } else {
            Vec::new()
        };
        let node_packages = if matches!(
            language.as_str(),
            "javascript" | "js" | "node" | "typescript" | "ts"
        ) {
            Self::detect_node_dep_packages(code)
        } else {
            Vec::new()
        };
        let auto_install_cmd = if !python_packages.is_empty() {
            tracing::info!("Auto-detected Python deps: {:?}", python_packages);
            format!(
                "PIP_ROOT_USER_ACTION=ignore PIP_DISABLE_PIP_VERSION_CHECK=1 python3 -m pip install --no-cache-dir -q {} && ",
                python_packages.join(" ")
            )
        } else if !node_packages.is_empty() {
            tracing::info!("Auto-detected Node.js deps: {:?}", node_packages);
            format!(
                "npm install --no-fund --no-audit -q {} 2>/dev/null && ",
                node_packages.join(" ")
            )
        } else {
            String::new()
        };

        let run = run_cmd
            .replace("{file}", &file_path)
            .replace("{sandbox_dir}", CODE_EXECUTE_SANDBOX_DIR);
        let workspace_bootstrap = format!(
            "mkdir -p '{sandbox}' '{home}' '{tmp}' '{cache}' '{pip_cache}' '{cache}/npm' && export HOME='{home}' TMPDIR='{tmp}' TMP='{tmp}' TEMP='{tmp}' XDG_CACHE_HOME='{cache}' PIP_CACHE_DIR='{pip_cache}' npm_config_cache='{cache}/npm' NPM_CONFIG_CACHE='{cache}/npm' && cd '{sandbox}' && ",
            sandbox = CODE_EXECUTE_SANDBOX_DIR,
            home = CODE_EXECUTE_HOME_DIR,
            tmp = CODE_EXECUTE_TMP_DIR,
            cache = CODE_EXECUTE_CACHE_DIR,
            pip_cache = CODE_EXECUTE_PIP_CACHE_DIR,
        );
        let main_cmd = if let Some(build) = build_cmd {
            let build = build
                .replace("{file}", &file_path)
                .replace("{sandbox_dir}", CODE_EXECUTE_SANDBOX_DIR);
            format!(
                "{}{}{}echo '{}' | base64 -d > {} && {} && {}",
                workspace_bootstrap,
                file_inject_cmds,
                auto_install_cmd,
                code_b64,
                file_path,
                build,
                run
            )
        } else {
            format!(
                "{}{}{}echo '{}' | base64 -d > {} && {}",
                workspace_bootstrap, file_inject_cmds, auto_install_cmd, code_b64, file_path, run
            )
        };

        // Append file extraction: finds files created/modified during execution,
        // excludes build artifacts, base64-encodes each file (up to 5MB) and
        // outputs them with markers so we can extract before container dies.
        // For notebooks (.ipynb), we also capture the executed notebook itself.
        let is_notebook = ext == "ipynb";
        let notebook_extra = if is_notebook {
            // Also extract the executed notebook file
            format!(
                r#" echo "FILE:$(basename {file}):$(base64 {file} | tr -d '\n')";"#,
                file = file_path
            )
        } else {
            String::new()
        };
        let shell_cmd = format!(
            r#"{}; __AGENTARK_EXIT=$?; echo; echo '__AGENTARK_OUTPUT_FILES__';{} find {sandbox_dir} -maxdepth 3 -type f ! -name 'code.*' ! -name 'a.out' ! -name 'Main.*' ! -name '*.class' ! -name 'out.jar' ! -name '*.ipynb' -newer {} 2>/dev/null | head -20 | while IFS= read -r __f; do __sz=$(stat -c%s "$__f" 2>/dev/null || echo 999999999); if [ "$__sz" -lt 5242880 ]; then echo "FILE:$(basename "$__f"):$(base64 "$__f" | tr -d '\n')"; fi; done; exit $__AGENTARK_EXIT"#,
            main_cmd,
            notebook_extra,
            file_path,
            sandbox_dir = CODE_EXECUTE_SANDBOX_DIR
        );

        // Notebooks get 10 min (install deps + execute all cells + ML training).
        // Compiled languages get 120s (build + run), interpreted get 60s. If
        // runtime auto-installs dependencies, raise the default so the control
        // plane does not time out ordinary sandbox package bootstrap.
        let base_timeout = if is_notebook {
            600
        } else if build_cmd.is_some() {
            120
        } else {
            60
        };
        let dependency_bootstrap = !python_packages.is_empty() || !node_packages.is_empty();
        let default_timeout = if dependency_bootstrap {
            base_timeout.max(180)
        } else {
            base_timeout
        };
        let timeout_limit = if is_notebook { 900 } else { 600 };
        let timeout = arguments
            .get("timeout_secs")
            .and_then(|value| value.as_u64())
            .unwrap_or(default_timeout)
            .clamp(1, timeout_limit);

        // Optional env vars for execution (resolved placeholders are already applied by the runtime).
        let env_vec: Option<Vec<String>> = arguments
            .get("env")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| format!("{}={}", k, s)))
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty());
        let network_access = Self::code_execute_effective_network_access(arguments);
        let isolation = if network_access {
            ContainerIsolation::StandardWithNetwork
        } else {
            ContainerIsolation::Standard
        };

        let raw_result = self
            .run_isolated_container(
                "code_execute",
                image,
                vec!["sh".to_string(), "-c".to_string(), shell_cmd],
                env_vec,
                timeout,
                isolation,
            )
            .await?;

        // Parse result and extract output files from stdout
        let parsed: serde_json::Value = serde_json::from_str(&raw_result)?;
        let output = parsed["output"].as_str().unwrap_or("");

        let exec_id = uuid::Uuid::new_v4().to_string();
        let output_dir = self.data_dir().join("outputs").join(&exec_id);
        tokio::fs::create_dir_all(&output_dir)
            .await
            .with_context(|| {
                format!(
                    "Failed to create output directory '{}'",
                    output_dir.display()
                )
            })?;

        let install_summary =
            Self::code_execute_dependency_summary(&python_packages, &node_packages);
        let install_metadata =
            Self::build_code_execute_dependency_metadata(&python_packages, &node_packages);
        let (user_output, saved_files) = if let Some(marker_pos) =
            output.find("__AGENTARK_OUTPUT_FILES__")
        {
            let mut user_output = output[..marker_pos].trim_end().to_string();
            if let Some(summary) = install_summary.as_deref() {
                if user_output.is_empty() {
                    user_output = summary.to_string();
                } else if !user_output.contains(summary) {
                    user_output = format!("{}\n{}", summary, user_output);
                }
            }
            let files_section = &output[marker_pos..];

            let mut saved = Vec::new();

            // Save the code file first so user can download it
            {
                let code_filename = format!("code.{}", ext);
                let code_path = output_dir.join(&code_filename);
                tokio::fs::write(&code_path, code).await.with_context(|| {
                    format!("Failed to save code artifact '{}'", code_path.display())
                })?;
                saved.push(format!("/api/outputs/{}/{}", exec_id, code_filename));
                tracing::debug!("Saved code file: {}", code_path.display());
            }

            // Extract output files from container stdout
            for line in files_section.lines() {
                if let Some(rest) = line.strip_prefix("FILE:") {
                    let parts: Vec<&str> = rest.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        let filename = parts[0];
                        let b64_data = parts[1];
                        use base64::Engine as _;
                        if let Ok(data) = base64::engine::general_purpose::STANDARD.decode(b64_data)
                        {
                            let out_path = output_dir.join(filename);
                            if let Ok(()) = tokio::fs::write(&out_path, &data).await {
                                let web_path = format!("/api/outputs/{}/{}", exec_id, filename);
                                saved.push(web_path);
                                tracing::info!(
                                    "Extracted output file: {} ({} bytes)",
                                    out_path.display(),
                                    data.len()
                                );
                            }
                        }
                    }
                }
            }

            (user_output, saved)
        } else {
            // No file marker found - still save the code file
            let mut saved = Vec::new();
            let code_filename = format!("code.{}", ext);
            let code_path = output_dir.join(&code_filename);
            tokio::fs::write(&code_path, code).await.with_context(|| {
                format!("Failed to save code artifact '{}'", code_path.display())
            })?;
            saved.push(format!("/api/outputs/{}/{}", exec_id, code_filename));
            let mut user_output = output.to_string();
            if let Some(summary) = install_summary.as_deref() {
                if user_output.trim().is_empty() {
                    user_output = summary.to_string();
                } else if !user_output.contains(summary) {
                    user_output = format!("{}\n{}", summary, user_output);
                }
            }
            (user_output, saved)
        };

        // Build final result with file paths. Keep stdout/stderr mirrored as
        // structured text payloads so repair prompts preserve exact errors.
        let error_value = parsed
            .get("error")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let error_text = error_value.as_str().unwrap_or("");
        let text_payloads = Self::code_execute_text_payloads(&user_output, error_text);
        let mut result = serde_json::json!({
            "output": user_output,
            "error": error_value,
            "exit_code": parsed.get("exit_code").cloned().unwrap_or(serde_json::json!(-1)),
            "dependency_installs": install_metadata,
            "agentark_execution": Self::build_code_execute_execution_metadata(
                arguments,
                parsed
                    .get("exit_code")
                    .and_then(|value| value.as_i64())
                    .unwrap_or(-1)
                    == 0,
                saved_files.len(),
            ),
        });
        if !text_payloads.is_empty() {
            result["text_payloads"] = serde_json::json!(text_payloads);
        }

        let exit_code = parsed
            .get("exit_code")
            .and_then(|value| value.as_i64())
            .unwrap_or(-1);
        if exit_code != 0 {
            let combined_failure_text = format!(
                "{}\n{}",
                result
                    .get("output")
                    .and_then(|value| value.as_str())
                    .unwrap_or(""),
                result
                    .get("error")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
            );
            if let Some(binary) = Self::detect_missing_binary_from_output(&combined_failure_text) {
                result["missing_capabilities"] = serde_json::json!([{
                    "kind": "binary",
                    "name": binary,
                    "approval_required": true,
                    "route": "host_install_approval",
                    "reason": "The sandbox execution failed because this binary is not available. AgentArk will not install OS/host packages without explicit approval."
                }]);
            }
        }

        if !saved_files.is_empty() {
            result["files"] = serde_json::json!(saved_files);
        }

        Ok(serde_json::to_string(&result)?)
    }

    /// Fallback: execute code natively in an isolated temp directory (no Docker)
    pub(in crate::runtime) async fn execute_code_native(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        if let Err(error) = self.prune_stale_code_execute_artifacts().await {
            tracing::warn!(
                "Failed to prune stale code execution artifacts before native sandbox run: {}",
                error
            );
        }
        let language = arguments["language"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'language' argument"))?
            .to_lowercase();
        let code = arguments["code"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'code' argument"))?;

        // Native fallback: try to find the runtime on the host
        let (program, args): (&str, Vec<String>) = match language.as_str() {
            "python" | "python3" | "py" => ("python3", vec!["-c".to_string(), code.to_string()]),
            "javascript" | "js" | "node" => ("node", vec!["-e".to_string(), code.to_string()]),
            "bash" | "sh" | "shell" => ("bash", vec!["-c".to_string(), code.to_string()]),
            "ruby" | "rb" => ("ruby", vec!["-e".to_string(), code.to_string()]),
            "php" => ("php", vec!["-r".to_string(), code.to_string()]),
            "perl" | "pl" => ("perl", vec!["-e".to_string(), code.to_string()]),
            _ => {
                return Err(anyhow::anyhow!(
                "Native fallback only supports interpreted languages. Docker required for '{}'.",
                language
            ));
            }
        };

        // Create isolated temp directory for execution
        let temp_dir = std::env::temp_dir().join(format!("agentark-exec-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&temp_dir).await?;

        // Execute with timeout, cleared env, isolated working dir
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(&args)
            .current_dir(&temp_dir)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", temp_dir.to_string_lossy().to_string())
            .env("TMPDIR", temp_dir.to_string_lossy().to_string())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        for (key, value) in Self::collect_native_env_overrides(arguments)? {
            cmd.env(key, value);
        }

        let result = tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output()).await;

        // Always clean up the temp directory
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;

        let output = result
            .map_err(|_| anyhow::anyhow!("Code execution timed out after 30 seconds"))?
            .map_err(|e| anyhow::anyhow!("Failed to execute {}: {}", program, e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        let text_payloads = Self::code_execute_text_payloads(&stdout, &stderr);
        let mut result = serde_json::json!({
            "output": stdout,
            "error": if stderr.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(stderr) },
            "exit_code": exit_code,
            "agentark_execution": Self::build_code_execute_execution_metadata(
                arguments,
                exit_code == 0,
                0,
            ),
        });
        if !text_payloads.is_empty() {
            result["text_payloads"] = serde_json::json!(text_payloads);
        }

        Ok(serde_json::to_string(&result)?)
    }
}
