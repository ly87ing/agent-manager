//! Provider detection layer — port of src/manage_runtime_probe.py
//! (ProviderInventory + classify_provider_state + is_agent_installed) and the
//! probe helpers in src/manage_doctor.py (command_exists / command_version).
//!
//! All probes are read-only and local: PATH search, realpath, brew/uv/gh list
//! output parsing. No network. Python's 10s probe timeouts are not ported
//! (std has no kill-on-timeout without extra deps); the listed tools answer
//! locally and instantly in practice.

use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::path::Path;

pub const SUPPORTED_UNSUPPORTED_ACTIONS: &[&str] = &["install", "update", "uninstall", "repair"];

fn run_cmd(cmd: &[&str]) -> (bool, String) {
    let output = std::process::Command::new(cmd[0])
        .args(&cmd[1..])
        .output();
    match output {
        Ok(out) => (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).to_string(),
        ),
        Err(_) => (false, String::new()),
    }
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// shutil.which port: absolute path check, else PATH scan.
pub fn which(command: &str) -> Option<String> {
    if command.is_empty() {
        return None;
    }
    let candidate = Path::new(command);
    if candidate.is_absolute() {
        return if is_executable(candidate) {
            Some(command.to_string())
        } else {
            None
        };
    }
    let path_var = std::env::var("PATH").unwrap_or_else(|_| std::env::consts::EXE_SUFFIX.to_string());
    for dir in path_var.split(':') {
        if dir.is_empty() {
            continue;
        }
        let full = Path::new(dir).join(command);
        if is_executable(&full) {
            return Some(full.to_string_lossy().to_string());
        }
    }
    None
}

/// os.path.realpath port: canonicalize when possible, else echo the input.
pub fn realpath(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string())
}

pub fn expanduser(path: &str) -> String {
    if path == "~" || path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}{}", home, &path[1..]);
        }
    }
    path.to_string()
}

pub fn normalize_path_for_match(path: &str) -> String {
    path.replace('\\', "/")
}

pub fn path_looks_brew_managed(path: &str) -> bool {
    let normalized = normalize_path_for_match(path);
    normalized.contains("/Cellar/") || normalized.contains("/Caskroom/")
}

pub fn path_indicates_npm(path: &str) -> bool {
    let normalized = normalize_path_for_match(path);
    !normalized.is_empty()
        && (normalized.contains("/node/")
            || normalized.contains("/npm/")
            || normalized.contains("/node_modules/")
            || normalized.ends_with("node")
            || normalized.ends_with("npm")
            || normalized.ends_with("npm.cmd"))
}

pub fn detect_npm_from_paths(bin_path: Option<&str>, resolved_bin_path: &str, brew_detected: bool) -> bool {
    if let Some(bin_path) = bin_path {
        if path_indicates_npm(bin_path) {
            return true;
        }
    }
    if resolved_bin_path.is_empty() {
        return false;
    }
    if brew_detected && path_looks_brew_managed(resolved_bin_path) {
        return false;
    }
    path_indicates_npm(resolved_bin_path)
}

/// get_pkg_name port。Python 侧是 re.search(r"(?:install|uninstall)(?:\s+--cask)?\s+([^\s@]+)")：
/// 原串扫描（非分词），最早命中位置，捕获非空白非 @ 的最长连续段；brew 命令去掉 tap 前缀。
pub fn get_pkg_name(cmd: &str) -> Option<String> {
    if cmd.is_empty() {
        return None;
    }
    let bytes = cmd.as_bytes();
    let mut match_start = None;
    for index in 0..bytes.len() {
        if cmd[index..].starts_with("install") || cmd[index..].starts_with("uninstall") {
            match_start = Some(index + if cmd[index..].starts_with("uninstall") { 9 } else { 7 });
            break;
        }
    }
    let mut index = match_start?;
    let skip_ws = |mut i: usize| {
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        i
    };
    index = skip_ws(index);
    if cmd[index..].starts_with("--cask") {
        index = skip_ws(index + 6);
    }
    let begin = index;
    while index < bytes.len() && !matches!(bytes[index], b' ' | b'\t' | b'\n' | b'\r' | b'@') {
        index += 1;
    }
    if begin == index {
        return None;
    }
    let pkg_name = &cmd[begin..index];
    if cmd.trim_start().starts_with("brew ") {
        return Some(pkg_name.rsplit('/').next().unwrap_or(pkg_name).to_string());
    }
    Some(pkg_name.to_string())
}

/// Minimal shlex.split：whitespace 分词，单双引用与反斜杠转义。
pub fn shell_split(cmd: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = cmd.chars().peekable();
    let mut quote: Option<char> = None;
    let mut has_content = false;
    while let Some(ch) = chars.next() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                } else if ch == '\\' && q == '"' {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                } else {
                    current.push(ch);
                }
                has_content = true;
            }
            None => {
                if ch.is_whitespace() {
                    if has_content || !current.is_empty() {
                        tokens.push(std::mem::take(&mut current));
                        has_content = false;
                    }
                } else if ch == '\'' || ch == '"' {
                    quote = Some(ch);
                    has_content = true;
                } else if ch == '\\' {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                    has_content = true;
                } else {
                    current.push(ch);
                    has_content = true;
                }
            }
        }
    }
    if has_content || !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// get_uv_tool_name port（fail-close 白名单 flag 扫描）。
pub fn get_uv_tool_name(cmd: &str) -> Option<String> {
    let tokens = shell_split(cmd);
    if tokens.len() < 4 || tokens[0] != "uv" || tokens[1..3] != ["tool", "install"] {
        return None;
    }
    let mut positional = Vec::new();
    let mut index = 3;
    while index < tokens.len() {
        let token = &tokens[index];
        if token.is_empty() {
            index += 1;
            continue;
        }
        if token.starts_with('-') {
            if token == "--python" {
                index += 2;
                continue;
            }
            if token.starts_with("--python=") {
                index += 1;
                continue;
            }
            return None;
        }
        positional.push(token.clone());
        index += 1;
    }
    if positional.len() != 1 {
        return None;
    }
    Some(positional.remove(0))
}

/// sync_tools._extract_npm_package_name port（scoped 包只有第二个 @ 才是版本分隔）。
pub fn extract_npm_package_name(command: &str) -> Option<String> {
    let tokens = shell_split(command);
    if tokens.len() < 3 || tokens[0] != "npm" || tokens[1] != "install" {
        return None;
    }
    for token in &tokens[2..] {
        if token.starts_with('-') {
            continue;
        }
        if let Some(stripped) = token.strip_prefix('@') {
            return Some(match stripped.rfind('@') {
                Some(pos) => format!("@{}", &stripped[..pos]),
                None => token.clone(),
            });
        }
        if token.contains('@') {
            return Some(token.split('@').next().unwrap_or("").to_string());
        }
        return Some(token.clone());
    }
    None
}

pub fn canonical_provider_name(name: &str) -> &str {
    if name.starts_with("brew") {
        "brew"
    } else {
        name
    }
}

pub fn provider_is_managed(provider_cfg: Option<&Value>) -> bool {
    provider_cfg
        .and_then(|cfg| cfg.get("managed"))
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

#[derive(Debug, Default, PartialEq)]
pub struct Classified {
    pub managed: Vec<String>,
    pub deprecated: Vec<String>,
    pub system_only: Vec<String>,
    pub unsupported: Vec<String>,
}

pub fn classify_provider_state(actual_providers: &[String], definition: &Map<String, Value>) -> Classified {
    let provider_defs = definition
        .get("providers")
        .and_then(Value::as_object);
    let mut state = Classified::default();
    for provider_name in actual_providers {
        if provider_name == "system" {
            state.system_only.push(provider_name.clone());
            continue;
        }
        let provider_cfg = provider_defs
            .and_then(|defs| defs.get(canonical_provider_name(provider_name)));
        match provider_cfg {
            None => state.unsupported.push(provider_name.clone()),
            Some(cfg) => {
                if provider_is_managed(Some(cfg)) {
                    state.managed.push(provider_name.clone());
                } else {
                    state.deprecated.push(provider_name.clone());
                }
            }
        }
    }
    state
}

pub struct Inventory {
    brew_formulae: Option<BTreeSet<String>>,
    brew_casks: Option<BTreeSet<String>>,
    gh_extensions: Option<String>,
    uv_tools: Option<BTreeSet<String>>,
}

impl Inventory {
    pub fn new() -> Self {
        Inventory {
            brew_formulae: None,
            brew_casks: None,
            gh_extensions: None,
            uv_tools: None,
        }
    }

    pub fn brew_formulae(&mut self) -> &BTreeSet<String> {
        if self.brew_formulae.is_none() {
            let mut set = BTreeSet::new();
            if which("brew").is_some() {
                let (ok, out) = run_cmd(&["brew", "list", "--formula", "-1"]);
                if ok {
                    for line in out.lines() {
                        if !line.trim().is_empty() {
                            set.insert(line.trim().to_string());
                        }
                    }
                }
            }
            self.brew_formulae = Some(set);
        }
        self.brew_formulae.as_ref().unwrap()
    }

    pub fn brew_casks(&mut self) -> &BTreeSet<String> {
        if self.brew_casks.is_none() {
            let mut set = BTreeSet::new();
            if which("brew").is_some() {
                let (ok, out) = run_cmd(&["brew", "list", "--cask", "-1"]);
                if ok {
                    for line in out.lines() {
                        if !line.trim().is_empty() {
                            set.insert(line.trim().to_string());
                        }
                    }
                }
            }
            self.brew_casks = Some(set);
        }
        self.brew_casks.as_ref().unwrap()
    }

    pub fn gh_extensions(&mut self) -> &str {
        if self.gh_extensions.is_none() {
            let mut text = String::new();
            if which("gh").is_some() {
                let (ok, out) = run_cmd(&["gh", "extension", "list"]);
                if ok {
                    text = out;
                }
            }
            self.gh_extensions = Some(text);
        }
        self.gh_extensions.as_ref().unwrap()
    }

    pub fn uv_tools(&mut self) -> &BTreeSet<String> {
        if self.uv_tools.is_none() {
            let mut set = BTreeSet::new();
            if which("uv").is_some() {
                let (ok, out) = run_cmd(&["uv", "tool", "list"]);
                if ok {
                    for line in out.lines() {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        if let Some(name) = line.split_whitespace().next() {
                            if !name.is_empty() {
                                set.insert(name.to_string());
                            }
                        }
                    }
                }
            }
            self.uv_tools = Some(set);
        }
        self.uv_tools.as_ref().unwrap()
    }

    /// detect_all_providers port（顺序即语义：detect_paths → brew → npm → gh → uv → system）。
    pub fn detect_all_providers(&mut self, bin_name: &str, config: &Map<String, Value>) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        let add_found = |found: &mut Vec<String>, name: &str| {
            if !name.is_empty() && !found.iter().any(|item| item == name) {
                found.push(name.to_string());
            }
        };

        let bin_path = which(bin_name);
        let resolved_bin_path = bin_path.as_deref().map(realpath).unwrap_or_default();
        let mut candidate_paths = BTreeSet::new();
        for path in [bin_path.as_deref(), Some(resolved_bin_path.as_str())].into_iter().flatten() {
            if !path.is_empty() {
                candidate_paths.insert(normalize_path_for_match(&expanduser(path)));
            }
        }

        if let Some(providers) = config.get("providers").and_then(Value::as_object) {
            for (provider_name, provider_cfg) in providers {
                let detect_paths: BTreeSet<String> = provider_cfg
                    .get("detect_paths")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .filter(|p| !p.is_empty())
                            .map(|p| normalize_path_for_match(&expanduser(p)))
                            .collect()
                    })
                    .unwrap_or_default();
                if !candidate_paths.is_empty() && !detect_paths.is_disjoint(&candidate_paths) {
                    add_found(&mut found, provider_name);
                }
                if let Some(detect_uv_tool) = provider_cfg.get("detect_uv_tool").and_then(Value::as_str) {
                    if !detect_uv_tool.is_empty() && self.uv_tools().contains(detect_uv_tool) {
                        add_found(&mut found, provider_name);
                    }
                }
            }
        }

        let brew_pkg = config
            .get("providers")
            .and_then(Value::as_object)
            .and_then(|providers| providers.get("brew"))
            .and_then(|brew| brew.get("install"))
            .and_then(Value::as_str)
            .and_then(get_pkg_name);
        if let Some(pkg) = brew_pkg {
            if self.brew_formulae().contains(&pkg) {
                add_found(&mut found, "brew");
            } else if self.brew_casks().contains(&pkg) {
                add_found(&mut found, "brew-cask");
            }
        }

        let brew_detected = found.iter().any(|name| name.starts_with("brew"));
        if detect_npm_from_paths(bin_path.as_deref(), &resolved_bin_path, brew_detected) {
            add_found(&mut found, "npm");
        }

        if let Some(ext_name) = config.get("ext_name").and_then(Value::as_str) {
            if !ext_name.is_empty()
                && self
                    .gh_extensions()
                    .lines()
                    .any(|line| line.trim_start().starts_with(ext_name))
            {
                add_found(&mut found, "gh");
            }
        }

        let uv_pkg = config
            .get("providers")
            .and_then(Value::as_object)
            .and_then(|providers| providers.get("uv"))
            .and_then(|uv| uv.get("install"))
            .and_then(Value::as_str)
            .and_then(get_uv_tool_name);
        if let Some(pkg) = uv_pkg {
            if self.uv_tools().contains(&pkg) {
                add_found(&mut found, "uv");
            }
        }

        let app_path_exists = config
            .get("app_path")
            .and_then(Value::as_str)
            .map(|p| Path::new(&expanduser(p)).exists())
            .unwrap_or(false);
        if (app_path_exists || bin_path.is_some()) && found.is_empty() {
            add_found(&mut found, "system");
        }

        found
    }
}

/// is_agent_installed port：bin（which）→ app_path → 各 provider 的 detect_paths。
pub fn is_agent_installed(definition: &Map<String, Value>) -> bool {
    if let Some(bin_name) = definition.get("bin").and_then(Value::as_str) {
        if !bin_name.is_empty() && which(bin_name).is_some() {
            return true;
        }
    }
    if let Some(app_path) = definition.get("app_path").and_then(Value::as_str) {
        if Path::new(&expanduser(app_path)).exists() {
            return true;
        }
    }
    if let Some(providers) = definition.get("providers").and_then(Value::as_object) {
        for (_name, provider) in providers {
            if let Some(paths) = provider.get("detect_paths").and_then(Value::as_array) {
                for raw in paths {
                    if let Some(raw) = raw.as_str() {
                        if Path::new(&expanduser(raw)).exists() {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// command_version port（manage_doctor）：bin --version，stdout 或 stderr 的第一行；
/// 空输出或执行失败均为 "可用"。
pub fn command_version(command: &str) -> String {
    let output = std::process::Command::new(command)
        .arg("--version")
        .output();
    let output = match output {
        Ok(output) => output,
        Err(_) => return "可用".to_string(),
    };
    let text = if !output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stdout).to_string()
    } else {
        String::from_utf8_lossy(&output.stderr).to_string()
    };
    let first = text.trim().lines().next().unwrap_or("").trim();
    if first.is_empty() {
        "可用".to_string()
    } else {
        first.to_string()
    }
}

/// _extract_version_token：第一个 \d+(\.\d+)+ 片段。
pub fn extract_version_token(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_digit() {
            let mut end = index + 1;
            while end < bytes.len()
                && (bytes[end].is_ascii_digit() || (bytes[end] == b'.' && end + 1 < bytes.len() && bytes[end + 1].is_ascii_digit()))
            {
                end += 1;
            }
            let token = &text[index..end];
            if token.contains('.') {
                return Some(token.to_string());
            }
            index = end;
        } else {
            index += 1;
        }
    }
    None
}

/// resolve_provider_command port：darwin→(darwin,posix,default)，linux→(linux,posix,default)，
/// windows→(windows,default)，其余→(posix,default)。Rust std::env::consts::OS 映射。
pub fn resolve_provider_command(value: &Value) -> Option<String> {
    resolve_provider_command_for(value, std::env::consts::OS)
}

pub fn resolve_provider_command_for(value: &Value, platform: &str) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    let mapping = value.as_object()?;
    let keys: &[&str] = if platform == "macos" || platform == "darwin" {
        &["darwin", "posix", "default"]
    } else if platform == "linux" {
        &["linux", "posix", "default"]
    } else if platform.starts_with("win") {
        &["windows", "default"]
    } else {
        &["posix", "default"]
    };
    for key in keys {
        if let Some(text) = mapping.get(*key).and_then(Value::as_str) {
            return Some(text.to_string());
        }
    }
    None
}

/// get_unsupported_actions port（manage_doctor）：fail-close 形状校验。
pub fn get_unsupported_actions(provider_cfg: Option<&Value>) -> Result<BTreeSet<String>, String> {
    let mut out = BTreeSet::new();
    let cfg = match provider_cfg {
        Some(cfg) => cfg,
        None => return Ok(out),
    };
    let raw = match cfg.get("unsupported_actions") {
        None => return Ok(out),
        Some(raw) => raw,
    };
    let items = raw
        .as_array()
        .ok_or_else(|| "unsupported_actions 必须是字符串列表。".to_string())?;
    for item in items {
        let name = item
            .as_str()
            .ok_or_else(|| "unsupported_actions 必须是字符串列表。".to_string())?;
        if SUPPORTED_UNSUPPORTED_ACTIONS.contains(&name) {
            out.insert(name.to_string());
        } else {
            return Err(format!(
                "unsupported_actions 仅支持 install, repair, uninstall, update；收到: '{}'。",
                name
            ));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn get_pkg_name_matches_python_regex() {
        // re.search(r"(?:install|uninstall)(?:\s+--cask)?\s+([^\s@]+)")
        assert_eq!(get_pkg_name("brew install --cask claude-code"), Some("claude-code".to_string()));
        assert_eq!(get_pkg_name("brew install homebrew/cask/foo"), Some("foo".to_string()));
        assert_eq!(get_pkg_name("brew uninstall brew/pkg"), Some("pkg".to_string()));
        assert_eq!(get_pkg_name("true"), None);
        assert_eq!(get_pkg_name(""), None);
    }

    #[test]
    fn shell_split_quotes_and_escapes() {
        assert_eq!(shell_split("npm install -g @fake/agent"), vec!["npm", "install", "-g", "@fake/agent"]);
        assert_eq!(shell_split("echo 'a b' \"c d\""), vec!["echo", "a b", "c d"]);
    }

    #[test]
    fn npm_package_name_scoped_and_versioned() {
        assert_eq!(extract_npm_package_name("npm install -g @anthropic-ai/claude-code"), Some("@anthropic-ai/claude-code".to_string()));
        assert_eq!(extract_npm_package_name("npm install -g @scope/pkg@1.2.3"), Some("@scope/pkg".to_string()));
        assert_eq!(extract_npm_package_name("npm install -g codex@0.1.0"), Some("codex".to_string()));
        assert_eq!(extract_npm_package_name("brew install x"), None);
    }

    #[test]
    fn platform_command_resolution() {
        let value = json!({"darwin": "mac-cmd", "linux": "linux-cmd", "posix": "posix-cmd", "default": "default-cmd"});
        assert_eq!(resolve_provider_command_for(&value, "darwin"), Some("mac-cmd".to_string()));
        assert_eq!(resolve_provider_command_for(&value, "macos"), Some("mac-cmd".to_string()));
        assert_eq!(resolve_provider_command_for(&value, "linux"), Some("linux-cmd".to_string()));
        assert_eq!(resolve_provider_command_for(&json!({"posix": "p"}), "freebsd"), Some("p".to_string()));
        assert_eq!(resolve_provider_command_for(&json!({"windows": "w"}), "windows"), Some("w".to_string()));
        assert_eq!(resolve_provider_command_for(&json!({"darwin": "d"}), "windows"), None);
        assert_eq!(resolve_provider_command_for(&json!("plain"), "linux"), Some("plain".to_string()));
        assert_eq!(resolve_provider_command_for(&json!(3), "linux"), None);
    }

    #[test]
    fn classify_state_buckets() {
        let definition = json!({"providers": {"npm": {"managed": true}, "brew": {"managed": false}}});
        let state = classify_provider_state(
            &["npm".to_string(), "brew-cask".to_string(), "system".to_string(), "mystery".to_string()],
            definition.as_object().unwrap(),
        );
        assert_eq!(state.managed, vec!["npm"]);
        assert_eq!(state.deprecated, vec!["brew-cask"]);
        assert_eq!(state.system_only, vec!["system"]);
        assert_eq!(state.unsupported, vec!["mystery"]);
    }

    #[test]
    fn version_token_extraction() {
        assert_eq!(extract_version_token("2.1.261 (Claude Code)"), Some("2.1.261".to_string()));
        assert_eq!(extract_version_token("可用"), None);
        assert_eq!(extract_version_token("v0.153.4"), Some("0.153.4".to_string()));
    }

    #[test]
    fn unsupported_actions_fail_close() {
        assert!(get_unsupported_actions(Some(&json!({"unsupported_actions": ["uninstall"]}))).unwrap().contains("uninstall"));
        assert!(get_unsupported_actions(Some(&json!({"unsupported_actions": "uninstall"}))).is_err());
        assert!(get_unsupported_actions(Some(&json!({"unsupported_actions": ["bogus"]}))).is_err());
    }
}
