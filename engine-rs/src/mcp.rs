//! mcp domain kernels — a faithful port of build_mcp_status / build_mcp_plan
//! from src/engine_v1.py (Python engine). Every state string, diagnostic code
//! and message template must match the Python implementation byte-for-byte;
//! tests/test_engine_rs_parity.py enforces that on a shared corpus.

use crate::canon::{json_equal, sha256_canonical, sha256_file};
use crate::envelope::{diagnostic, Diagnostic};
use crate::ids::{canonicalize_agent_list, canonicalize_agent_mapping};
use crate::upstream;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::Path;

const MCP_APPLY_ACTION: &str =
    "{\"id\": \"apply\", \"risk\": \"write-local\", \"needs_lock\": true, \"offline_ok\": true}";

/// engine_v1 的配置/定义层错误（exit 1, code CONFIG_INVALID）。
pub struct ConfigInvalid(pub String);

impl std::fmt::Display for ConfigInvalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn load_json_object(path: &Path) -> Result<Map<String, Value>, ConfigInvalid> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    let text = std::fs::read_to_string(path)
        .map_err(|_| ConfigInvalid(format!("{} 缺失", name)))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|exc| ConfigInvalid(format!("{} 无效: {}", name, exc)))?;
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(ConfigInvalid(format!("{} 顶层必须是对象", name))),
    }
}

/// get_typed_mapping_section semantics: missing/null → {}; wrong type → error.
fn typed_mapping(
    data: &Map<String, Value>,
    key: &str,
    source: &str,
    allow_str: bool,
) -> Result<Map<String, Value>, ConfigInvalid> {
    let section = match data.get(key) {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(map)) => map.clone(),
        Some(_) => return Err(ConfigInvalid(format!("{}.{} 必须是对象", source, key))),
    };
    for (_name, value) in &section {
        let ok = value.is_object() || (allow_str && value.is_string());
        if !ok {
            return Err(ConfigInvalid(format!(
                "{}.{} 必须是{}",
                source,
                key,
                if allow_str { "字符串或对象" } else { "对象" }
            )));
        }
    }
    Ok(section)
}

fn expanduser(path: &str) -> String {
    if path == "~" || path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}{}", home, &path[1..]);
        }
    }
    path.to_string()
}

#[derive(Clone, Debug, PartialEq)]
enum EnvKind {
    Placeholder,
    Literal(Value),
}

#[derive(Clone, Debug)]
struct NormalizedServer {
    command: Option<String>,
    args: Option<Vec<String>>,
    env: BTreeMap<String, EnvKind>,
}

fn is_placeholder(value: &Value) -> bool {
    match value {
        Value::String(text) => {
            text.starts_with("${") && text.ends_with('}') && text.len() > 3 && !text[..text.len() - 1].contains('}')
        }
        _ => false,
    }
}

fn placeholder_name(value: &Value) -> Option<String> {
    if is_placeholder(value) {
        if let Value::String(text) = value {
            return Some(text[2..text.len() - 1].to_string());
        }
    }
    None
}

fn normalize_env(env: Option<&Value>) -> BTreeMap<String, EnvKind> {
    let mut out = BTreeMap::new();
    let env = match env.and_then(Value::as_object) {
        Some(map) => map,
        None => return out,
    };
    for (key, value) in env {
        if key == "_MANAGED_BY" {
            continue;
        }
        out.insert(
            key.clone(),
            if is_placeholder(value) {
                EnvKind::Placeholder
            } else {
                EnvKind::Literal(value.clone())
            },
        );
    }
    out
}

/// resolve_mcp_server_args (src/mcp_config.py): base args + per-agent extras.
fn resolve_args(settings: &Map<String, Value>, agent_id: &str) -> Option<Vec<String>> {
    let base = match settings.get("args") {
        None => Vec::new(),
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                out.push(item.as_str()?.to_string());
            }
            out
        }
        Some(_) => return None,
    };
    let extras_raw = match settings.get("extra_args_by_agent") {
        None | Some(Value::Null) => return Some(base),
        Some(Value::Object(map)) => map.clone(),
        Some(_) => return None,
    };
    let extras = canonicalize_agent_mapping(&Value::Object(extras_raw)).ok()?;
    let mut out = base;
    for (target_agent, extra) in &extras {
        let list = extra.as_array()?;
        let mut resolved = Vec::new();
        for item in list {
            resolved.push(item.as_str()?.to_string());
        }
        if *target_agent == agent_id {
            out.extend(resolved);
        }
    }
    Some(out)
}

fn normalize_expected(settings: &Map<String, Value>, agent_id: &str, root_dir: &Path, server_name: &str) -> NormalizedServer {
    let env = settings.get("env");
    let mut args = resolve_args(settings, agent_id);
    // {repo_path} substitution for repo-backed (url) servers.
    if let (Some(ref mut resolved_args), Some(_url)) = (&mut args, settings.get("url")) {
        let repo_path = root_dir.join("agent-mcp").join(server_name);
        let rendered = std::fs::canonicalize(&repo_path)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| repo_path.to_string_lossy().to_string());
        for arg in resolved_args.iter_mut() {
            *arg = arg.replace("{repo_path}", &rendered);
        }
    }
    NormalizedServer {
        command: settings
            .get("command")
            .and_then(Value::as_str)
            .map(str::to_string),
        args,
        env: normalize_env(env),
    }
}

fn normalize_actual(settings: &Map<String, Value>) -> Option<NormalizedServer> {
    let env_value = settings
        .get("env")
        .or_else(|| settings.get("environment"));
    let env_map = env_value.and_then(Value::as_object)?;
    if env_map.get("_MANAGED_BY").and_then(Value::as_str) != Some("agent-manager") {
        return None;
    }
    let args = match settings.get("args") {
        None => Some(Vec::new()),
        Some(value) => {
            let items = value.as_array()?;
            let mut out = Vec::new();
            for item in items {
                out.push(item.as_str()?.to_string());
            }
            Some(out)
        }
    };
    Some(NormalizedServer {
        command: settings
            .get("command")
            .and_then(Value::as_str)
            .map(str::to_string),
        args,
        env: normalize_env(env_value),
    })
}

fn managed_matches(expected: &NormalizedServer, actual: &NormalizedServer) -> bool {
    if expected.command != actual.command {
        return false;
    }
    if expected.args != actual.args {
        return false;
    }
    let expected_keys: std::collections::BTreeSet<&String> = expected.env.keys().collect();
    let actual_keys: std::collections::BTreeSet<&String> = actual.env.keys().collect();
    if expected_keys != actual_keys {
        return false;
    }
    for (key, expected_kind) in &expected.env {
        let actual_kind = &actual.env[key];
        match (expected_kind, actual_kind) {
            (EnvKind::Placeholder, EnvKind::Literal(_)) => {}
            (EnvKind::Literal(left), EnvKind::Literal(right)) if json_equal(left, right) => {}
            _ => return false,
        }
    }
    true
}

fn read_json_servers(text: &str) -> Result<Map<String, Value>, String> {
    let parsed: Value = serde_json::from_str(text).map_err(|exc| exc.to_string())?;
    let top = parsed.as_object().ok_or_else(|| "顶层必须是对象".to_string())?;
    match top.get("mcpServers") {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map.clone()),
        Some(_) => Ok(Map::new()),
    }
}

fn read_toml_servers(text: &str) -> Result<Map<String, Value>, String> {
    let parsed: toml::Value = toml::from_str(text).map_err(|exc| exc.to_string())?;
    let top = parsed.as_table().ok_or_else(|| "顶层必须是对象".to_string())?;
    let servers = match top.get("mcp_servers") {
        None => return Ok(Map::new()),
        Some(value) => value.as_table().cloned().unwrap_or_default(),
    };
    let mut out = Map::new();
    for (name, value) in servers {
        let json_value = toml_value_to_json(&value);
        out.insert(name, json_value);
    }
    Ok(out)
}

fn toml_value_to_json(value: &toml::Value) -> Value {
    match value {
        toml::Value::String(s) => Value::String(s.clone()),
        toml::Value::Integer(i) => Value::Number((*i).into()),
        toml::Value::Float(f) => {
            serde_json::Number::from_f64(*f).map_or(Value::Null, Value::Number)
        }
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Datetime(dt) => Value::String(dt.to_string()),
        toml::Value::Array(items) => Value::Array(items.iter().map(toml_value_to_json).collect()),
        toml::Value::Table(table) => Value::Object(
            table
                .iter()
                .map(|(k, v)| (k.clone(), toml_value_to_json(v)))
                .collect(),
        ),
    }
}

/// managed_mcp_servers (src/manage_doctor.py): JSON + TOML targets.
fn managed_servers(target_path: &str, file_format: &str) -> Result<BTreeMap<String, NormalizedServer>, String> {
    let text = std::fs::read_to_string(target_path).map_err(|exc| exc.to_string())?;
    let mapping = if file_format == "toml" {
        read_toml_servers(&text)?
    } else {
        read_json_servers(&text)?
    };
    let mut servers = BTreeMap::new();
    for (name, settings) in mapping {
        if let Value::Object(settings_map) = settings {
            if let Some(normalized) = normalize_actual(&settings_map) {
                servers.insert(name, normalized);
            }
        }
    }
    Ok(servers)
}

/// expected_managed_mcp_servers for one agent.
fn expected_servers(
    root_dir: &Path,
    policy_cfg: &Map<String, Value>,
    servers: &Map<String, Value>,
    agent_id: &str,
) -> BTreeMap<String, NormalizedServer> {
    let mut expected = BTreeMap::new();
    for (server_name, policy) in policy_cfg {
        let policy_map = match policy.as_object() {
            Some(map) => map,
            None => continue,
        };
        if !policy_map
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            continue;
        }
        let definition = match servers.get(server_name).and_then(Value::as_object) {
            Some(def) => def.clone(),
            None => continue,
        };
        let mut merged = definition;
        for (key, value) in policy_map {
            merged.insert(key.clone(), value.clone());
        }
        let targeted = canonicalize_agent_list(merged.get("agents").unwrap_or(&Value::Null));
        if let Some(ref targeted_list) = targeted {
            if !targeted_list.iter().any(|item| item == agent_id) {
                continue;
            }
        }
        expected.insert(
            server_name.clone(),
            normalize_expected(&merged, agent_id, root_dir, server_name),
        );
    }
    expected
}

struct AgentProbe {
    target: String,
    agent_enabled: bool,
    target_exists: bool,
    servers: BTreeMap<String, NormalizedServer>,
    parse_error: Option<String>,
}

fn probe_agent_targets(
    agents_policy: &Map<String, Value>,
    agent_paths: &Map<String, Value>,
    errors: &mut Vec<Diagnostic>,
) -> BTreeMap<String, AgentProbe> {
    let mut probes = BTreeMap::new();
    let canonical_paths = match canonicalize_agent_mapping(&Value::Object(agent_paths.clone())) {
        Ok(map) => map,
        Err(message) => {
            errors.push(diagnostic("CONFIG_INVALID", None, message));
            return probes;
        }
    };
    for (agent_id, path_config) in canonical_paths {
        let (target, file_format) = match &path_config {
            Value::String(path) => (path.clone(), "json".to_string()),
            Value::Object(map) => (
                map.get("path").and_then(Value::as_str).unwrap_or("").to_string(),
                map.get("format")
                    .and_then(Value::as_str)
                    .unwrap_or("json")
                    .to_string(),
            ),
            _ => ("".to_string(), "json".to_string()),
        };
        let target = expanduser(&target);
        let agent_enabled = agents_policy
            .get(&agent_id)
            .and_then(Value::as_object)
            .and_then(|policy| policy.get("enabled").and_then(Value::as_bool))
            .unwrap_or(true);
        let mut probe = AgentProbe {
            target: target.clone(),
            agent_enabled,
            target_exists: Path::new(&target).exists(),
            servers: BTreeMap::new(),
            parse_error: None,
        };
        if probe.target_exists {
            match managed_servers(&target, &file_format) {
                Ok(servers) => probe.servers = servers,
                Err(exc) => {
                    probe.parse_error = Some(exc.clone());
                    errors.push(diagnostic(
                        "MCP_TARGET_PARSE_ERROR",
                        None,
                        format!(
                            "agent '{}' 的 MCP 配置解析失败 ({}): {}",
                            agent_id, target, exc
                        ),
                    ));
                }
            }
        }
        probes.insert(agent_id, probe);
    }
    probes
}

fn cell_state(
    server_name: &str,
    probe: &AgentProbe,
    expected: Option<&NormalizedServer>,
    server_enabled: bool,
) -> &'static str {
    if !server_enabled {
        return if probe.servers.contains_key(server_name) {
            "leftover"
        } else {
            "absent"
        };
    }
    if !probe.agent_enabled {
        return if probe.servers.contains_key(server_name) {
            "leftover"
        } else {
            "agent-disabled"
        };
    }
    if probe.parse_error.is_some() {
        return "probe-error";
    }
    if !probe.target_exists {
        return "not-materialized";
    }
    let actual = match probe.servers.get(server_name) {
        Some(actual) => actual,
        None => return "missing",
    };
    let expected = match expected {
        Some(expected) => expected,
        None => return "missing",
    };
    if managed_matches(expected, actual) {
        "managed-ok"
    } else {
        "drifted"
    }
}

fn aggregate_state(cell_states: &[&str]) -> &'static str {
    if cell_states
        .iter()
        .any(|state| matches!(*state, "drifted" | "leftover" | "probe-error"))
    {
        return "drifted";
    }
    if cell_states.contains(&"missing") {
        return "missing";
    }
    if !cell_states.is_empty() && cell_states.iter().all(|state| *state == "managed-ok") {
        return "managed-ok";
    }
    if cell_states.contains(&"not-materialized") {
        return "not-materialized";
    }
    "managed-ok"
}

fn env_resolution_status(env: &Map<String, Value>, secrets_env: &Map<String, Value>) -> Map<String, Value> {
    let mut status = Map::new();
    for (key, value) in env {
        let entry = match placeholder_name(value) {
            None => {
                let mut item = Map::new();
                item.insert("placeholder".to_string(), Value::Null);
                item.insert("resolved".to_string(), Value::Bool(true));
                item
            }
            Some(var_name) => {
                let resolved = secrets_env.contains_key(&var_name) || std::env::var_os(&var_name).is_some();
                let mut item = Map::new();
                item.insert("placeholder".to_string(), Value::String(var_name));
                item.insert("resolved".to_string(), Value::Bool(resolved));
                item
            }
        };
        status.insert(key.clone(), Value::Object(entry));
    }
    status
}

fn redact_env_for_spec(env: &Map<String, Value>) -> Map<String, Value> {
    let mut redacted = Map::new();
    for (key, value) in env {
        redacted.insert(
            key.clone(),
            if placeholder_name(value).is_some() {
                value.clone()
            } else {
                Value::String("<literal>".to_string())
            },
        );
    }
    redacted
}

/// _classify_mcp_probe: npx package → ("npm-npx", pkg); uvx git+…@branch →
/// ("git-ls-remote", repo, branch); otherwise None.
fn classify_mcp_probe(merged: &Map<String, Value>) -> Option<(String, Vec<String>)> {
    let command = merged.get("command").and_then(Value::as_str)?;
    let args: Vec<&str> = merged
        .get("args")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if command == "npx" {
        for arg in args {
            if !arg.starts_with('-') {
                return Some(("npm-npx".to_string(), vec![arg.to_string()]));
            }
        }
        return None;
    }
    if command == "uvx" {
        for arg in args {
            if let Some(rest) = arg.strip_prefix("git+") {
                let (repo, branch) = rest.rsplit_once('@')?;
                return Some((
                    "git-ls-remote".to_string(),
                    vec![repo.to_string(), branch.to_string()],
                ));
            }
        }
    }
    None
}

fn freshness_block(
    element_id: &str,
    probeable: bool,
    cache: &Map<String, Value>,
    local_current: Option<String>,
    warnings: &mut Vec<Diagnostic>,
) -> Value {
    let mut block = Map::new();
    if !probeable {
        block.insert("state".to_string(), Value::String("not-applicable".to_string()));
        block.insert(
            "current".to_string(),
            local_current.map_or(Value::Null, Value::String),
        );
        block.insert("upstream".to_string(), Value::Null);
        block.insert("checked_at".to_string(), Value::Null);
        block.insert("probe".to_string(), Value::Null);
        block.insert("cache_age_sec".to_string(), Value::Null);
        return Value::Object(block);
    }
    let entry = cache.get(element_id).and_then(Value::as_object).cloned();
    let entry = match entry {
        Some(entry) => entry,
        None => {
            block.insert("state".to_string(), Value::String("unknown".to_string()));
            block.insert(
                "current".to_string(),
                local_current.map_or(Value::Null, Value::String),
            );
            block.insert("upstream".to_string(), Value::Null);
            block.insert("checked_at".to_string(), Value::Null);
            block.insert("probe".to_string(), Value::Null);
            block.insert("cache_age_sec".to_string(), Value::Null);
            return Value::Object(block);
        }
    };
    let now = chrono::Utc::now().timestamp();
    let entry_value = Value::Object(entry.clone());
    if upstream::is_stale(&entry_value, now) {
        warnings.push(diagnostic(
            "UPSTREAM_PROBE_STALE",
            Some(element_id.to_string()),
            "上游探测缓存超过 TTL，展示 last-known 值；显式 system:refresh-upstream 刷新".to_string(),
        ));
    }
    let current = entry
        .get("current")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or(local_current);
    block.insert(
        "state".to_string(),
        Value::String(
            entry
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
        ),
    );
    block.insert(
        "current".to_string(),
        current.map_or(Value::Null, Value::String),
    );
    block.insert(
        "upstream".to_string(),
        entry
            .get("upstream")
            .and_then(Value::as_str)
            .map_or(Value::Null, |s| Value::String(s.to_string())),
    );
    block.insert(
        "checked_at".to_string(),
        entry
            .get("checked_at")
            .and_then(Value::as_str)
            .map_or(Value::Null, |s| Value::String(s.to_string())),
    );
    block.insert(
        "probe".to_string(),
        entry
            .get("probe")
            .and_then(Value::as_str)
            .map_or(Value::Null, |s| Value::String(s.to_string())),
    );
    block.insert(
        "cache_age_sec".to_string(),
        upstream::cache_age_sec(&entry_value, now).map_or(Value::Null, |age| Value::Number(age.into())),
    );
    Value::Object(block)
}

/// build_mcp_status port. Returns (data, warnings, errors).
pub fn build_mcp_status(
    root_dir: &Path,
    config_file: &Path,
    def_file: &Path,
) -> Result<(Value, Vec<Diagnostic>, Vec<Diagnostic>), ConfigInvalid> {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let config_name = config_file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let def_name = def_file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let config = load_json_object(config_file)?;
    let policy_cfg = typed_mapping(&config, "mcp", &config_name, false)?;
    let def_data = load_json_object(def_file)?;
    let servers = typed_mapping(&def_data, "servers", &def_name, false)?;
    let agent_paths = typed_mapping(&def_data, "agent_paths", &def_name, true)?;

    let agents_policy_raw = config.get("agents").and_then(Value::as_object).cloned().unwrap_or_default();
    let agents_policy = canonicalize_agent_mapping(&Value::Object(agents_policy_raw))
        .map_err(ConfigInvalid)?;

    // Same credential sources as agent-mcp/sync_mcp.py, minus the network sync.
    let secrets_env = upstream::load_env_files(&[
        root_dir.join(".env"),
        root_dir.join("secrets").join("mcp.env"),
    ]);
    let cache = upstream::load_cache();

    let probes = probe_agent_targets(&agents_policy, &agent_paths, &mut errors);
    let mut expected_by_agent: BTreeMap<String, BTreeMap<String, NormalizedServer>> = BTreeMap::new();
    for (agent_id, probe) in &probes {
        if probe.agent_enabled && probe.parse_error.is_none() {
            expected_by_agent.insert(
                agent_id.clone(),
                expected_servers(root_dir, &policy_cfg, &servers, agent_id),
            );
        }
    }

    let mut elements = Vec::new();
    // Python 按 sorted(servers) 迭代；preserve_order 下 Map 是文件顺序，须显式排序。
    let mut server_names: Vec<&String> = servers.keys().collect();
    server_names.sort();
    for server_name in server_names {
        let definition = servers[server_name].as_object().cloned().unwrap_or_default();
        let policy = policy_cfg.get(server_name).and_then(Value::as_object).cloned();
        let declared = policy.is_some();
        let enabled = declared
            && policy
                .as_ref()
                .and_then(|p| p.get("enabled"))
                .and_then(Value::as_bool)
                .unwrap_or(true);
        let mut merged = definition.clone();
        if let Some(ref policy_map) = policy {
            for (key, value) in policy_map {
                merged.insert(key.clone(), value.clone());
            }
        }
        let targeted_agents = canonicalize_agent_list(merged.get("agents").unwrap_or(&Value::Null));
        let raw_env = definition
            .get("env")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let merged_env = merged
            .get("env")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        let mut cells = Map::new();
        for (agent_id, probe) in &probes {
            if let Some(ref targeted) = targeted_agents {
                if !targeted.contains(agent_id) {
                    continue;
                }
            }
            let state = cell_state(
                server_name,
                probe,
                expected_by_agent
                    .get(agent_id)
                    .and_then(|expected| expected.get(server_name)),
                enabled,
            );
            let mut cell = Map::new();
            cell.insert("state".to_string(), Value::String(state.to_string()));
            cell.insert("target".to_string(), Value::String(probe.target.clone()));
            cell.insert("target_exists".to_string(), Value::Bool(probe.target_exists));
            cells.insert(agent_id.clone(), Value::Object(cell));
        }

        let state: &str = if !enabled {
            let has_leftover = cells.values().any(|cell| cell.get("state").and_then(Value::as_str) == Some("leftover"));
            if has_leftover {
                "drifted"
            } else {
                "disabled"
            }
        } else if cells.is_empty() {
            "disabled"
        } else {
            let cell_states: Vec<&str> = cells
                .values()
                .filter_map(|cell| cell.get("state").and_then(Value::as_str))
                .collect();
            aggregate_state(&cell_states)
        };

        let env_status = env_resolution_status(&merged_env, &secrets_env);
        if enabled {
            for (env_key, entry) in &env_status {
                let unresolved = entry.get("resolved").and_then(Value::as_bool) == Some(false);
                if unresolved {
                    let placeholder = entry
                        .get("placeholder")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    warnings.push(diagnostic(
                        "MCP_ENV_UNRESOLVED",
                        Some(format!("mcp:{}", server_name)),
                        format!(
                            "env 键 {} 的占位符 {} 未解析（.env / secrets/mcp.env / 环境变量均无）",
                            env_key, placeholder
                        ),
                    ));
                }
            }
        }

        let mut spec = definition.clone();
        spec.insert("env".to_string(), Value::Object(redact_env_for_spec(&raw_env)));
        let probe_kind = if enabled {
            classify_mcp_probe(&merged)
        } else {
            None
        };
        let local_current = match &probe_kind {
            Some((kind, params)) if kind == "npm-npx" => upstream::npx_cached_version(&params[0]),
            _ => None,
        };
        let element_id = format!("mcp:{}", server_name);
        let freshness = freshness_block(&element_id, probe_kind.is_some(), &cache, local_current, &mut warnings);

        let mut policy_block = Map::new();
        policy_block.insert("enabled".to_string(), Value::Bool(enabled));
        policy_block.insert("declared_in_policy".to_string(), Value::Bool(declared));
        policy_block.insert(
            "agents".to_string(),
            targeted_agents.map_or(Value::Null, |list| {
                Value::Array(list.into_iter().map(Value::String).collect())
            }),
        );
        let mut observed = Map::new();
        observed.insert("agents".to_string(), Value::Object(cells));
        observed.insert("env".to_string(), Value::Object(env_status));

        let mut element = Map::new();
        element.insert("id".to_string(), Value::String(element_id));
        element.insert("domain".to_string(), Value::String("mcp".to_string()));
        element.insert("spec".to_string(), Value::Object(spec));
        element.insert("policy".to_string(), Value::Object(policy_block));
        element.insert("observed".to_string(), Value::Object(observed));
        element.insert("state".to_string(), Value::String(state.to_string()));
        element.insert("freshness".to_string(), freshness);
        element.insert(
            "actions".to_string(),
            serde_json::from_str(MCP_APPLY_ACTION).map(|action| Value::Array(vec![action])).unwrap_or(Value::Array(vec![])),
        );
        elements.push(Value::Object(element));
    }

    let mut data = Map::new();
    data.insert("domain".to_string(), Value::String("mcp".to_string()));
    data.insert("elements".to_string(), Value::Array(elements));
    Ok((Value::Object(data), warnings, errors))
}

/// build_mcp_plan port (digest conventions: before = target file sha256,
/// after = canonical injection-section sha256; leftovers null the after).
pub fn build_mcp_plan(
    root_dir: &Path,
    config_file: &Path,
    def_file: &Path,
) -> Result<(Value, Vec<Diagnostic>, Vec<Diagnostic>), ConfigInvalid> {
    let (status, warnings, errors) = build_mcp_status(root_dir, config_file, def_file)?;
    let mut actions = Vec::new();
    let elements = status
        .get("elements")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for element in elements {
        let element_id = element.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        let spec = element.get("spec").cloned().unwrap_or(Value::Null);
        let agents = element
            .get("observed")
            .and_then(|o| o.get("agents"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (agent_id, cell) in &agents {
            let cell_state = cell.get("state").and_then(Value::as_str).unwrap_or("");
            if matches!(cell_state, "managed-ok" | "agent-disabled" | "absent" | "probe-error") {
                continue;
            }
            let target = cell.get("target").and_then(Value::as_str).unwrap_or("").to_string();
            let target_exists = cell.get("target_exists").and_then(Value::as_bool).unwrap_or(false);
            let before_digest = if target_exists {
                sha256_file(Path::new(&target))
            } else {
                None
            };
            let (after_digest, note) = if cell_state == "leftover" {
                (
                    None,
                    format!("移除 agent '{}' 中的托管残留 {}", agent_id, element_id),
                )
            } else {
                (
                    Some(sha256_canonical(&spec)),
                    format!(
                        "注入/收敛 {} 到 agent '{}'（before=目标文件整体 digest，after=注入段规范形 digest）",
                        element_id, agent_id
                    ),
                )
            };
            let mut action = Map::new();
            action.insert("op".to_string(), Value::String("write-file".to_string()));
            action.insert("element".to_string(), Value::String(element_id.clone()));
            action.insert("target".to_string(), Value::String(target));
            action.insert("risk".to_string(), Value::String("write-local".to_string()));
            action.insert(
                "before_digest".to_string(),
                before_digest.map_or(Value::Null, Value::String),
            );
            action.insert(
                "after_digest".to_string(),
                after_digest.map_or(Value::Null, Value::String),
            );
            action.insert("backup_path".to_string(), Value::Null);
            action.insert("cmd".to_string(), Value::Null);
            action.insert("note".to_string(), Value::String(note));
            actions.push(Value::Object(action));
        }
    }
    let mut summary = Map::new();
    for key in ["read", "write-local", "exec-local", "network", "install-remove"] {
        summary.insert(key.to_string(), Value::Number(0.into()));
    }
    for action in &actions {
        let risk = action.get("risk").and_then(Value::as_str).unwrap_or("read");
        if let Some(Value::Number(count)) = summary.get_mut(risk) {
            let current = count.as_i64().unwrap_or(0);
            summary.insert(risk.to_string(), Value::Number((current + 1).into()));
        }
    }
    let mut data = Map::new();
    data.insert("domain".to_string(), Value::String("mcp".to_string()));
    data.insert("actions".to_string(), Value::Array(actions));
    data.insert("summary".to_string(), Value::Object(summary));
    Ok((Value::Object(data), warnings, errors))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn placeholder_detection_matches_python_pattern() {
        assert!(is_placeholder(&json!("${GROK_API_KEY}")));
        assert!(!is_placeholder(&json!("prefix-${X}")));
        assert!(!is_placeholder(&json!("${}")));
        assert!(!is_placeholder(&json!("${A}}")));
        assert!(!is_placeholder(&json!("${A}x")));
        assert_eq!(placeholder_name(&json!("${A_B}")), Some("A_B".to_string()));
    }

    #[test]
    fn managed_match_placeholder_accepts_any_literal() {
        let expected = NormalizedServer {
            command: Some("cmd".to_string()),
            args: Some(vec!["--x".to_string()]),
            env: BTreeMap::from_iter([("K".to_string(), EnvKind::Placeholder)]),
        };
        let actual = NormalizedServer {
            command: Some("cmd".to_string()),
            args: Some(vec!["--x".to_string()]),
            env: BTreeMap::from_iter([("K".to_string(), EnvKind::Literal(json!("anything")))]),
        };
        assert!(managed_matches(&expected, &actual));
        let drifted = NormalizedServer {
            args: Some(vec!["--y".to_string()]),
            ..actual.clone()
        };
        assert!(!managed_matches(&expected, &drifted));
    }

    #[test]
    fn aggregate_matches_python_rules() {
        assert_eq!(aggregate_state(&["managed-ok", "managed-ok"]), "managed-ok");
        assert_eq!(aggregate_state(&["managed-ok", "not-materialized"]), "not-materialized");
        assert_eq!(aggregate_state(&["not-materialized", "missing"]), "missing");
        assert_eq!(aggregate_state(&["agent-disabled", "absent"]), "managed-ok");
        assert_eq!(aggregate_state(&["agent-disabled", "leftover"]), "drifted");
        assert_eq!(aggregate_state(&[]), "managed-ok");
    }

    #[test]
    fn classify_npx_and_uvx_git() {
        let npx = json!({"command": "npx", "args": ["-y", "chrome-devtools-mcp", "--isolated"]});
        assert_eq!(
            classify_mcp_probe(npx.as_object().unwrap()),
            Some(("npm-npx".to_string(), vec!["chrome-devtools-mcp".to_string()]))
        );
        let uvx = json!({"command": "uvx", "args": ["--from", "git+https://example.invalid/repo.git@main", "srv"]});
        assert_eq!(
            classify_mcp_probe(uvx.as_object().unwrap()),
            Some((
                "git-ls-remote".to_string(),
                vec!["https://example.invalid/repo.git".to_string(), "main".to_string()]
            ))
        );
        let local = json!({"command": "alpha-cmd", "args": []});
        assert_eq!(classify_mcp_probe(local.as_object().unwrap()), None);
    }
}
