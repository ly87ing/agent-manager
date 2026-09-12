//! hooks 域固定目标、来源结构校验与渲染语义。
//!
//! Claude（~/.claude/settings.json）按签名集合合并：保留根级用户字段与
//! 非托管 hook，先剥旧同签名项，再移除 removed_signatures 命中项，再合入
//! 当前 managed hooks。Codex（~/.codex/hooks.json）整文件渲染
//! {"hooks": <source hooks>}。所有校验错误使用固定或仅含字段类别的消息，
//! 不回显 source JSON / command / marker。

use anyhow::{bail, Result};
use serde_json::{Map, Value};

use crate::native_mcp::model::sha256_hex_bytes;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetSpec {
    pub target_id: &'static str,
    pub agent_id: &'static str,
    pub source_key: &'static str,
    pub merge: &'static str,
    pub name: &'static str,
    /// HOME 相对目标路径。
    pub target: &'static str,
}

pub(crate) const TARGET_CLAUDE: TargetSpec = TargetSpec {
    target_id: "hooks:claude",
    agent_id: "claude-code",
    source_key: "hooks.claude",
    merge: "signature-set",
    name: "Claude Hooks",
    target: ".claude/settings.json",
};

pub(crate) const TARGET_CODEX: TargetSpec = TargetSpec {
    target_id: "hooks:codex",
    agent_id: "codex",
    source_key: "hooks.codex",
    merge: "whole-file",
    name: "Codex Hooks",
    target: ".codex/hooks.json",
};

/// 固定目标列表，稳定顺序：claude 在前，codex 在后。
pub(crate) fn targets() -> [TargetSpec; 2] {
    [TARGET_CLAUDE, TARGET_CODEX]
}

pub(crate) type HookMap = Map<String, Value>; // event -> [entry]
pub(crate) type Signature = (String, Option<String>, String); // event, matcher, command
pub(crate) type RemovedRule = (String, Option<String>, Option<String>, Option<String>);

fn validate_hook_entry(entry: &Value) -> Result<()> {
    let entry = entry
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("hook 条目必须是对象"))?;
    let hooks_list = match entry.get("hooks").and_then(Value::as_array) {
        Some(list) if !list.is_empty() => list,
        _ => bail!("hook 条目的 hooks 必须是非空数组"),
    };
    if let Some(matcher) = entry.get("matcher") {
        if !matcher.is_string() {
            bail!("hook 条目的 matcher 必须是字符串");
        }
    }
    for hook in hooks_list {
        let hook_map = hook
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("hook 必须是对象"))?;
        if hook_map.get("type").and_then(Value::as_str) != Some("command") {
            bail!("hook.type 必须是 command");
        }
        let command = hook_map
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("");
        if command.trim().is_empty() {
            bail!("hook.command 必须是非空字符串");
        }
    }
    Ok(())
}

fn validate_hooks_map(raw: Option<&Value>) -> Result<HookMap> {
    let map = raw
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("hooks 必须是对象"))?;
    let mut validated = Map::new();
    for (event_name, entries) in map {
        if event_name.trim().is_empty() {
            bail!("hooks 的事件名必须是非空字符串");
        }
        let list = entries
            .as_array()
            .filter(|list| !list.is_empty())
            .ok_or_else(|| anyhow::anyhow!("hooks 的每个事件必须是非空数组"))?;
        for entry in list {
            validate_hook_entry(entry)?;
        }
        validated.insert(event_name.clone(), Value::Array(list.clone()));
    }
    Ok(validated)
}

fn validate_removed_signatures(raw: Option<&Value>) -> Result<Vec<RemovedRule>> {
    let mut validated = Vec::new();
    let raw = match raw {
        None | Some(Value::Null) => return Ok(validated),
        Some(value) => value,
    };
    let items = raw
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("removed_signatures 必须是数组"))?;
    for entry in items {
        let entry = entry
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("removed_signatures 条目必须是对象"))?;
        let event = entry.get("event").and_then(Value::as_str).unwrap_or("");
        if event.trim().is_empty() {
            bail!("removed_signatures 条目的 event 必须是非空字符串");
        }
        if entry.get("matcher").is_some() && !entry["matcher"].is_string() {
            bail!("removed_signatures 条目的 matcher 必须是字符串或省略");
        }
        let command = entry
            .get("command")
            .and_then(Value::as_str)
            .map(str::to_string);
        let command_contains = entry
            .get("command_contains")
            .and_then(Value::as_str)
            .map(str::to_string);
        if entry.get("command").is_some() && command.is_none() {
            bail!("removed_signatures 条目的 command 必须是字符串");
        }
        if entry.get("command_contains").is_some() && command_contains.is_none() {
            bail!("removed_signatures 条目的 command_contains 必须是字符串");
        }
        if command.is_none() && command_contains.is_none() {
            bail!("removed_signatures 条目必须提供 command 或 command_contains");
        }
        if command.as_deref().is_some_and(|c| c.trim().is_empty())
            || command_contains
                .as_deref()
                .is_some_and(|c| c.trim().is_empty())
        {
            bail!("removed_signatures 条目的 command/command_contains 不能为空白");
        }
        validated.push((
            event.to_string(),
            entry
                .get("matcher")
                .and_then(Value::as_str)
                .map(str::to_string),
            command,
            command_contains,
        ));
    }
    Ok(validated)
}

/// 校验并拆分来源对象。Claude 只允许 hooks/removed_signatures；
/// Codex 只允许 hooks。错误不回显来源内容。
pub(crate) fn validate_source(
    source_key: &str,
    obj: &Map<String, Value>,
) -> Result<(HookMap, Vec<RemovedRule>)> {
    match source_key {
        "hooks.claude" => {
            for key in obj.keys() {
                if key != "hooks" && key != "removed_signatures" {
                    bail!("hooks.claude 来源只允许 hooks 与 removed_signatures 字段");
                }
            }
            let hooks = validate_hooks_map(obj.get("hooks").or(Some(&Value::Object(Map::new()))))?;
            let removed = validate_removed_signatures(obj.get("removed_signatures"))?;
            Ok((hooks, removed))
        }
        "hooks.codex" => {
            for key in obj.keys() {
                if key != "hooks" {
                    bail!("hooks.codex 来源只允许 hooks 字段");
                }
            }
            let hooks = validate_hooks_map(obj.get("hooks").or(Some(&Value::Object(Map::new()))))?;
            Ok((hooks, Vec::new()))
        }
        _ => bail!("未知 hooks 来源 key"),
    }
}

pub(crate) fn collect_signatures(hooks: &HookMap) -> Vec<Signature> {
    let mut signatures = Vec::new();
    for (event_name, entries) in hooks {
        if let Some(list) = entries.as_array() {
            for entry in list {
                let matcher = entry
                    .as_object()
                    .and_then(|e| e.get("matcher"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let Some(hooks_list) = entry
                    .as_object()
                    .and_then(|e| e.get("hooks"))
                    .and_then(Value::as_array)
                {
                    for hook in hooks_list {
                        if let Some(command) = hook
                            .as_object()
                            .and_then(|h| h.get("command"))
                            .and_then(Value::as_str)
                        {
                            let signature =
                                (event_name.clone(), matcher.clone(), command.to_string());
                            if !signatures.contains(&signature) {
                                signatures.push(signature);
                            }
                        }
                    }
                }
            }
        }
    }
    signatures
}

/// 从目标文件现有 hooks 值容错收集签名（结构损坏处跳过）。
pub(crate) fn collect_present_signatures(existing_hooks: &Value) -> Vec<Signature> {
    let mut signatures = Vec::new();
    let map = match existing_hooks.as_object() {
        Some(map) => map,
        None => return signatures,
    };
    for (event_name, entries) in map {
        let entries = match entries.as_array() {
            Some(list) => list,
            None => continue,
        };
        for entry in entries {
            let entry = match entry.as_object() {
                Some(entry) => entry,
                None => continue,
            };
            let matcher = entry
                .get("matcher")
                .and_then(Value::as_str)
                .map(str::to_string);
            let hooks_list = match entry.get("hooks").and_then(Value::as_array) {
                Some(list) => list,
                None => continue,
            };
            for hook in hooks_list {
                if let Some(command) = hook
                    .as_object()
                    .and_then(|h| h.get("command"))
                    .and_then(Value::as_str)
                {
                    let signature = (event_name.clone(), matcher.clone(), command.to_string());
                    if !signatures.contains(&signature) {
                        signatures.push(signature);
                    }
                }
            }
        }
    }
    signatures
}

fn removed_rule_present(rule: &RemovedRule, present: &[Signature]) -> bool {
    let (event, matcher, command, command_contains) = rule;
    for (p_event, p_matcher, p_command) in present {
        if p_event != event {
            continue;
        }
        if let Some(m) = matcher {
            if p_matcher.as_deref() != Some(m.as_str()) {
                continue;
            }
        }
        if let Some(c) = command {
            if p_command != c {
                continue;
            }
        }
        if let Some(sub) = command_contains {
            if !p_command.contains(sub.as_str()) {
                continue;
            }
        }
        return true;
    }
    false
}

pub(crate) fn count_removed_present(removed: &[RemovedRule], present: &[Signature]) -> usize {
    removed
        .iter()
        .filter(|rule| removed_rule_present(rule, present))
        .count()
}

fn strip_hooks_where(
    existing_hooks: &Value,
    drop: &dyn Fn(&str, Option<&str>, &str) -> bool,
) -> Value {
    let map = match existing_hooks.as_object() {
        Some(map) => map,
        None => return Value::Object(Map::new()),
    };
    let mut cleaned = Map::new();
    for (event_name, entries) in map {
        let list = match entries.as_array() {
            Some(list) => list,
            None => {
                cleaned.insert(event_name.clone(), entries.clone());
                continue;
            }
        };
        let mut kept_entries = Vec::new();
        for entry in list {
            let entry_map = match entry.as_object() {
                Some(map) => map,
                None => {
                    kept_entries.push(entry.clone());
                    continue;
                }
            };
            let hooks_list = match entry_map.get("hooks").and_then(Value::as_array) {
                Some(list) => list,
                None => {
                    kept_entries.push(entry.clone());
                    continue;
                }
            };
            let matcher = entry_map
                .get("matcher")
                .and_then(Value::as_str)
                .map(str::to_string);
            let kept_hooks: Vec<Value> = hooks_list
                .iter()
                .filter(|hook| {
                    let command = hook
                        .as_object()
                        .and_then(|h| h.get("command"))
                        .and_then(Value::as_str);
                    match command {
                        Some(command) => !drop(event_name, matcher.as_deref(), command),
                        None => true,
                    }
                })
                .cloned()
                .collect();
            if !kept_hooks.is_empty() {
                let mut new_entry = entry_map.clone();
                new_entry.insert("hooks".to_string(), Value::Array(kept_hooks));
                kept_entries.push(Value::Object(new_entry));
            }
        }
        if !kept_entries.is_empty() {
            cleaned.insert(event_name.clone(), Value::Array(kept_entries));
        }
    }
    Value::Object(cleaned)
}

fn strip_removed_signatures(existing_hooks: &Value, removed: &[RemovedRule]) -> Value {
    if removed.is_empty() {
        return existing_hooks.clone();
    }
    strip_hooks_where(existing_hooks, &|event, matcher, command| {
        removed.iter().any(|rule| {
            removed_rule_present(
                rule,
                &[(
                    event.to_string(),
                    matcher.map(str::to_string),
                    command.to_string(),
                )],
            )
        })
    })
}

fn strip_managed_hooks(existing_hooks: &Value, managed_signatures: &[Signature]) -> Value {
    strip_hooks_where(existing_hooks, &|event, matcher, command| {
        managed_signatures.contains(&(
            event.to_string(),
            matcher.map(str::to_string),
            command.to_string(),
        ))
    })
}

/// Claude 渲染：保留 settings 根级用户字段；hooks 先剥 removed 命中项、
/// 再剥旧同签名托管项，最后合入当前 managed hooks。
pub(crate) fn claude_render(
    settings: &Value,
    managed_hooks: &HookMap,
    removed: &[RemovedRule],
) -> Value {
    let managed_signatures = collect_signatures(managed_hooks);
    let hooks_value = settings
        .get("hooks")
        .cloned()
        .unwrap_or(Value::Object(Map::new()));
    let existing_hooks = strip_removed_signatures(&hooks_value, removed);
    let mut merged_hooks = strip_managed_hooks(&existing_hooks, &managed_signatures)
        .as_object()
        .cloned()
        .unwrap_or_default();
    for (event_name, entries) in managed_hooks {
        let entry_list = merged_hooks
            .entry(event_name.clone())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let (Value::Array(target), Value::Array(items)) = (entry_list, entries) {
            target.extend(items.iter().cloned());
        }
    }
    let mut merged = settings.as_object().cloned().unwrap_or_default();
    merged.insert("hooks".to_string(), Value::Object(merged_hooks));
    Value::Object(merged)
}

/// Codex 渲染：整文件 {"hooks": <source hooks>}。
pub(crate) fn codex_render(managed_hooks: &HookMap) -> Value {
    let mut out = Map::new();
    out.insert("hooks".to_string(), Value::Object(managed_hooks.clone()));
    Value::Object(out)
}

/// 写盘字节：pretty JSON（2 空格）+ 单个换行。
pub(crate) fn render_bytes(payload: &Value) -> Vec<u8> {
    let mut text = serde_json::to_string_pretty(payload).expect("render");
    text.push('\n');
    text.into_bytes()
}

pub(crate) fn digest_bytes(bytes: &[u8]) -> String {
    sha256_hex_bytes(bytes)
}

/// 目标文件 hooks 子树的严格校验（脱敏、不回显内容）：hooks 缺失视为空
/// 对象可用；存在时必须是对象，每个 event 名非空、值必须是非空数组，
/// 每个 entry 必须是对象且带非空 hooks 数组，每个 hook 必须是对象、
/// type=="command"、command 为非空字符串；matcher 若存在必须是字符串。
/// 未知附加字段允许并保留。
pub(crate) fn validate_target_hooks(target: &Value) -> Result<()> {
    let Some(hooks) = target.get("hooks") else {
        return Ok(());
    };
    let map = hooks
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("目标文件 hooks 字段不是对象"))?;
    for (event, entries) in map {
        if event.trim().is_empty() {
            bail!("目标文件 hooks 事件名不能为空");
        }
        let list = entries
            .as_array()
            .filter(|list| !list.is_empty())
            .ok_or_else(|| anyhow::anyhow!("目标文件 hooks 事件值必须是非空数组"))?;
        for entry in list {
            let entry_map = entry
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("目标文件 hooks 条目不是对象"))?;
            if let Some(matcher) = entry_map.get("matcher") {
                if !matcher.is_string() {
                    bail!("目标文件 hooks 条目的 matcher 不是字符串");
                }
            }
            let hooks_list = entry_map
                .get("hooks")
                .and_then(Value::as_array)
                .filter(|list| !list.is_empty())
                .ok_or_else(|| anyhow::anyhow!("目标文件 hooks 条目的 hooks 必须是非空数组"))?;
            for hook in hooks_list {
                let hook_map = hook
                    .as_object()
                    .ok_or_else(|| anyhow::anyhow!("目标文件 hook 不是对象"))?;
                if hook_map.get("type").and_then(Value::as_str) != Some("command") {
                    bail!("目标文件 hook.type 不是 command");
                }
                match hook_map.get("command").and_then(Value::as_str) {
                    Some(command) if !command.trim().is_empty() => {}
                    _ => bail!("目标文件 hook.command 缺失或为空"),
                }
            }
        }
    }
    Ok(())
}
