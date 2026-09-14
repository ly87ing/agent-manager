//! MCP 未受管条目发现（mcp:status import_candidates，只读、零写、零网络）
//!
//! 扫描各已配置 agent target（Claude JSON / Codex TOML / OpenCode JSON）的目标
//! 文件，提取库中无同名定义、mcp_bindings 无记录、名称合法的 server 条目，
//! 输出掩码后的导入候选。条目级或 env 内 `_MANAGED_BY` 标记不再整体排除：
//! marker 与 mcp_bindings 记录同时存在（真正受管）的条目经 binding 条件排除；
//! 有 marker 但无 binding（旧体系遗留）的条目进入候选并标注
//! `"previously_managed": true`，无 marker（纯外部）的候选标注 false。
//!
//! 任何 env/header 明文值绝不进入输出：stdio env 与静态 headers 只保留键名、
//! 值恒为 <literal>；Codex env_http_headers 是 header→变量名引用，按 ${VAR}
//! 形式直通展示。目标文件不存在/损坏/路径校验拒绝时该 agent 候选为空，
//! 不影响其他 agent（与 status 现有降级语义一致），诊断与日志不含明文。

use serde_json::{json, Map, Value};

use super::model::{is_valid_server_name, Bindings, MARKER_KEY, MARKER_VALUE};
use super::policy_view::LibraryView;
use super::targets::{validate_target_path, AgentTarget, Format};

const LITERAL_MASK: &str = "<literal>";

/// 单个条目的最小提取结果：只持有键名与引用变量名，绝不持有明文值。
struct RawEntry {
    managed: bool,
    remote: bool,
    command: String,
    args: Vec<String>,
    env_keys: Vec<String>,
    url: String,
    /// (header 名, 引用变量名)：变量名仅来自 Codex env_http_headers；
    /// 其余来源（Codex 静态 http_headers、Claude/OpenCode headers）为 None → <literal>
    headers: Vec<(String, Option<String>)>,
}

/// 掩码后的定义形态：stdio {command, args, env:{KEY:<literal>}}；
/// remote {url, headers:{NAME:<literal>|${VAR}}}。
fn masked_spec(entry: &RawEntry) -> Value {
    if entry.remote {
        let mut headers = Map::new();
        for (name, var) in &entry.headers {
            let value = match var {
                Some(var) => format!("${{{}}}", var),
                None => LITERAL_MASK.to_string(),
            };
            headers.insert(name.clone(), Value::String(value));
        }
        json!({ "url": entry.url, "headers": headers })
    } else {
        let mut env = Map::new();
        for key in &entry.env_keys {
            env.insert(key.clone(), Value::String(LITERAL_MASK.to_string()));
        }
        json!({ "command": entry.command, "args": entry.args, "env": env })
    }
}

fn parse_json_entry(opencode: bool, srv: &Map<String, Value>) -> Option<RawEntry> {
    let env_key = if opencode { "environment" } else { "env" };
    let env_map = match srv.get(env_key) {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(map)) => map.clone(),
        // 条目形态非法：跳过该条目，不拖累同文件其他条目
        Some(_) => return None,
    };
    let managed = srv.get(MARKER_KEY).and_then(Value::as_str) == Some(MARKER_VALUE)
        || env_map.get(MARKER_KEY).and_then(Value::as_str) == Some(MARKER_VALUE);
    if let Some(url_val) = srv.get("url") {
        let url = url_val.as_str()?;
        let mut headers = Vec::new();
        match srv.get("headers") {
            None | Some(Value::Null) => {}
            Some(Value::Object(map)) => {
                for key in map.keys() {
                    headers.push((key.clone(), None));
                }
            }
            Some(_) => return None,
        }
        return Some(RawEntry {
            managed,
            remote: true,
            command: String::new(),
            args: Vec::new(),
            env_keys: Vec::new(),
            url: url.to_string(),
            headers,
        });
    }
    let env_keys = env_map
        .keys()
        .filter(|k| *k != MARKER_KEY)
        .cloned()
        .collect();
    let (command, args) = if opencode {
        match srv.get("command") {
            Some(Value::Array(arr)) if !arr.is_empty() => {
                let mut items = Vec::with_capacity(arr.len());
                for item in arr {
                    items.push(item.as_str()?.to_string());
                }
                if items[0].trim().is_empty() {
                    return None;
                }
                (items.remove(0), items)
            }
            _ => return None,
        }
    } else {
        let cmd = match srv.get("command") {
            Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
            _ => return None,
        };
        let args = match srv.get("args") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(arr)) => {
                let mut res = Vec::with_capacity(arr.len());
                for item in arr {
                    res.push(item.as_str()?.to_string());
                }
                res
            }
            _ => return None,
        };
        (cmd, args)
    };
    Some(RawEntry {
        managed,
        remote: false,
        command,
        args,
        env_keys,
        url: String::new(),
        headers: Vec::new(),
    })
}

fn scan_json(doc: &Map<String, Value>, format: &Format) -> Vec<(String, RawEntry)> {
    let opencode = *format == Format::OpenCodeJson;
    let container_key = if opencode { "mcp" } else { "mcpServers" };
    let container = match doc.get(container_key) {
        None | Some(Value::Null) => return Vec::new(),
        Some(Value::Object(map)) => map,
        // 容器损坏：该 agent 候选为空
        Some(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for (name, raw) in container {
        let Some(srv) = raw.as_object() else {
            continue;
        };
        if let Some(entry) = parse_json_entry(opencode, srv) {
            out.push((name.clone(), entry));
        }
    }
    out
}

fn parse_toml_entry(table: &toml::value::Table) -> Option<RawEntry> {
    let env_table = match table.get("env") {
        None => toml::value::Table::new(),
        Some(toml::Value::Table(t)) => t.clone(),
        Some(_) => return None,
    };
    let managed = table.get(MARKER_KEY).and_then(toml::Value::as_str) == Some(MARKER_VALUE)
        || env_table.get(MARKER_KEY).and_then(toml::Value::as_str) == Some(MARKER_VALUE);
    if let Some(url_val) = table.get("url") {
        let url = url_val.as_str()?;
        let mut headers: Vec<(String, Option<String>)> = Vec::new();
        // env_http_headers：header → 变量名引用，按 ${VAR} 直通展示
        match table.get("env_http_headers") {
            None => {}
            Some(toml::Value::Table(t)) => {
                for (hk, hv) in t {
                    headers.push((hk.clone(), hv.as_str().map(str::to_string)));
                }
            }
            Some(_) => return None,
        }
        // 静态 http_headers：值恒 <literal>；同名引用条目优先保留
        match table.get("http_headers") {
            None => {}
            Some(toml::Value::Table(t)) => {
                for hk in t.keys() {
                    if !headers.iter().any(|(k, _)| k == hk) {
                        headers.push((hk.clone(), None));
                    }
                }
            }
            Some(_) => return None,
        }
        return Some(RawEntry {
            managed,
            remote: true,
            command: String::new(),
            args: Vec::new(),
            env_keys: Vec::new(),
            url: url.to_string(),
            headers,
        });
    }
    let env_keys = env_table
        .keys()
        .filter(|k| *k != MARKER_KEY)
        .cloned()
        .collect();
    let command = match table.get("command") {
        Some(toml::Value::String(s)) if !s.trim().is_empty() => s.clone(),
        _ => return None,
    };
    let args = match table.get("args") {
        None => Vec::new(),
        Some(toml::Value::Array(arr)) => {
            let mut res = Vec::with_capacity(arr.len());
            for item in arr {
                res.push(item.as_str()?.to_string());
            }
            res
        }
        Some(_) => return None,
    };
    Some(RawEntry {
        managed,
        remote: false,
        command,
        args,
        env_keys,
        url: String::new(),
        headers: Vec::new(),
    })
}

fn scan_toml(text: &str) -> Vec<(String, RawEntry)> {
    let parsed: toml::Value = match toml::from_str(text) {
        Ok(v) => v,
        // TOML 损坏：该 agent 候选为空
        Err(_) => return Vec::new(),
    };
    let servers = match parsed.get("mcp_servers") {
        None => return Vec::new(),
        Some(toml::Value::Table(t)) => t,
        Some(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for (name, raw) in servers {
        let Some(table) = raw.as_table() else {
            continue;
        };
        if let Some(entry) = parse_toml_entry(table) {
            out.push((name.clone(), entry));
        }
    }
    out
}

/// 汇总全部 agent target 的导入候选，按 (name, agent) 排序保证确定性输出。
pub fn import_candidates(
    library: &LibraryView,
    bindings: &Bindings,
    targets: &[AgentTarget],
) -> Vec<Value> {
    let mut out = Vec::new();
    for target in targets {
        if matches!(target.format, Format::Unsupported(_)) || target.path.as_os_str().is_empty() {
            continue;
        }
        // 与单元格一致的路径安全校验（symlink/目录等拒绝）；被拒时该 agent 候选为空
        if validate_target_path(&target.path).is_err() {
            continue;
        }
        let entries = match target.format {
            Format::ClaudeJson | Format::OpenCodeJson => {
                match super::codecs::read_json_doc(&target.path) {
                    Ok(Some(doc)) => scan_json(&doc, &target.format),
                    // 不存在/读取失败/解析失败 → 空
                    _ => Vec::new(),
                }
            }
            Format::CodexToml => match std::fs::read_to_string(&target.path) {
                Ok(text) => scan_toml(&text),
                Err(_) => Vec::new(),
            },
            Format::Unsupported(_) => continue,
        };
        for (name, entry) in entries {
            // 真正受管（marker + binding）经 binding 条件排除；marker 无 binding
            // （旧体系遗留）与无 marker（纯外部）都进入候选。
            if library.definitions.contains_key(&name)
                || bindings.contains_key(&name)
                || !is_valid_server_name(&name)
            {
                continue;
            }
            out.push(json!({
                "name": name,
                "agent": target.agent,
                "transport": if entry.remote { "remote" } else { "stdio" },
                "spec": masked_spec(&entry),
                "previously_managed": entry.managed,
            }));
        }
    }
    out.sort_by(|a, b| {
        let name_ord = a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""));
        name_ord.then_with(|| {
            a["agent"]
                .as_str()
                .unwrap_or("")
                .cmp(b["agent"].as_str().unwrap_or(""))
        })
    });
    out
}
