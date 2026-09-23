//! hooks 域模型：hooks 目标、结构校验，以及对实际 hooks 的纯函数编辑。
//!
//! 生效 hooks 以各助手的实际配置为准：主 Claude（`~/.claude/settings.json` 的 `hooks` 键）、
//! 主 Codex（`~/.codex/hooks.json`），以及每个实例配置目录下对应的文件。Claude 的
//! settings.json 里还住着接入、运行参数与用户自己的设置，本工具只替换 `hooks` 键；
//! Codex 的 hooks.json 整份归 hooks。内置 removed_signatures 只用于在实际 hooks 里
//! 找出旧版本留下的条目，作为清理建议列出，从不自动移除。
//!
//! hook 的 `type` 不止 command（官方还有 http、mcp_tool、prompt、agent，将来还可能更多）：
//! 结构校验只看「事件名 → 条目数组 → matcher/hooks」这层骨架，command 类型再要求非空
//! command；其他类型整条对象原样保留，清理建议与旧版建议的签名匹配也只作用于 command 类型。
//!
//! 所有校验错误使用固定或仅含字段类别的消息，不回显 JSON / command。

use std::path::PathBuf;

use anyhow::{bail, Result};
use serde_json::{json, Map, Value};

use crate::native::Context;
use crate::native_mcp::model::{sha256_hex, sha256_hex_bytes};
use crate::native_profiles::model as profiles;

pub(crate) const TARGET_CLAUDE: &str = "hooks:claude";
pub(crate) const TARGET_CODEX: &str = "hooks:codex";
const PROFILE_PREFIX: &str = "hooks:profile:";

/// 写入基准的「没有」：Claude 为 settings.json 里没有 hooks 键（含文件不存在），
/// Codex 为 hooks.json 不存在。
pub(crate) const BASE_MISSING: &str = "missing";

/// 旧版本的来源键：只用于识别旧库 `hook_sources` 里保存过的 hooks。
pub(crate) const LEGACY_KEY_CLAUDE: &str = "hooks.claude";
pub(crate) const LEGACY_KEY_CODEX: &str = "hooks.codex";

/// 编译内置的旧版本默认来源：Claude 的 removed_signatures 是清理建议的判据。
const RAW_BUILTIN_CLAUDE: &str = include_str!("../../../config/claude-hooks.json");
const RAW_BUILTIN_CODEX: &str = include_str!("../../../config/codex-hooks.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HookKind {
    /// settings.json 的 `hooks` 键。
    Claude,
    /// hooks.json 整份。
    Codex,
}

impl HookKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            HookKind::Claude => "claude",
            HookKind::Codex => "codex",
        }
    }

    pub(crate) fn legacy_key(self) -> &'static str {
        match self {
            HookKind::Claude => LEGACY_KEY_CLAUDE,
            HookKind::Codex => LEGACY_KEY_CODEX,
        }
    }

    /// 本工具在目标文件里负责的范围（展示用）。
    fn scope(self) -> &'static str {
        match self {
            HookKind::Claude => "hooks-key",
            HookKind::Codex => "whole-file",
        }
    }
}

/// 一个 hooks 目标：路径、归属与能否读写的静态判定（文件内容在 status 探测时读取）。
#[derive(Debug, Clone)]
pub(crate) struct Target {
    pub id: String,
    pub name: String,
    pub group: &'static str,
    pub kind: HookKind,
    /// 主助手为 agent id；实例为实例 id。
    pub agent_id: String,
    pub profile_id: Option<String>,
    /// 展示路径（`~/...`）。
    pub display: String,
    pub path: PathBuf,
    pub enabled: bool,
    /// 目标本身不可用的原因（实例配置目录非法等）。
    pub blocked: Option<String>,
    /// 实例目标：配置目录由实例域创建，目录不存在时 hooks 域不写。
    pub needs_existing_dir: bool,
}

impl Target {
    /// 主助手目标：旧版本保存的 hooks 只对照它们。
    pub(crate) fn is_main(&self) -> bool {
        self.profile_id.is_none()
    }

    pub(crate) fn spec_json(&self) -> Value {
        json!({
            "name": self.name,
            "group": self.group,
            "kind": self.kind.as_str(),
            "scope": self.kind.scope(),
            "profile_id": self.profile_id,
            "target": self.display,
        })
    }
}

/// 目标 id 形状：两个主助手目标，或 `hooks:profile:<实例 id>`。
pub(crate) fn valid_target_id(id: &str) -> bool {
    id == TARGET_CLAUDE
        || id == TARGET_CODEX
        || id
            .strip_prefix(PROFILE_PREFIX)
            .is_some_and(profiles::valid_profile_id)
}

/// 主助手是否启用：定义存在、未删除且 enabled 不为 false。
pub(crate) fn agent_enabled(policy: &Value, agent_id: &str) -> bool {
    let defs = policy
        .get("agents")
        .and_then(|v| v.get("definitions"))
        .and_then(Value::as_object);
    let Some(def) = defs.and_then(|defs| defs.get(agent_id)) else {
        return false;
    };
    if !def.is_object() || def.get("_delete") == Some(&Value::Bool(true)) {
        return false;
    }
    policy
        .get("agents")
        .and_then(|v| v.get(agent_id))
        .and_then(|v| v.get("enabled"))
        .and_then(Value::as_bool)
        != Some(false)
}

fn main_target(ctx: &Context, policy: &Value, kind: HookKind) -> Target {
    let (id, name, agent_id, rel) = match kind {
        HookKind::Claude => (
            TARGET_CLAUDE,
            "Claude Code Hooks",
            "claude-code",
            ".claude/settings.json",
        ),
        HookKind::Codex => (TARGET_CODEX, "Codex Hooks", "codex", ".codex/hooks.json"),
    };
    Target {
        id: id.to_string(),
        name: name.to_string(),
        group: "global-hooks",
        kind,
        agent_id: agent_id.to_string(),
        profile_id: None,
        display: format!("~/{rel}"),
        path: ctx.home_dir.join(rel),
        enabled: agent_enabled(policy, agent_id),
        blocked: None,
        needs_existing_dir: false,
    }
}

/// 全部 hooks 目标：主 Claude、主 Codex 在前，随后是各实例（按实例 id 排序）。
/// 定义损坏的实例由 profiles 域报告，这里跳过。
pub(crate) fn resolve_targets(ctx: &Context, policy: &Value) -> Vec<Target> {
    let mut out = vec![
        main_target(ctx, policy, HookKind::Claude),
        main_target(ctx, policy, HookKind::Codex),
    ];
    let mut defs: Vec<profiles::ProfileDef> = profiles::load_definitions(policy)
        .into_iter()
        .filter_map(Result::ok)
        .collect();
    defs.sort_by(|a, b| a.id.cmp(&b.id));
    for def in defs {
        let (kind, file) = match def.kind {
            profiles::ProfileKind::Claude => (HookKind::Claude, profiles::FILE_CLAUDE_SETTINGS),
            profiles::ProfileKind::Codex => (HookKind::Codex, profiles::FILE_CODEX_HOOKS),
        };
        let (path, blocked) =
            match profiles::validate_config_dir(&def, &ctx.home_dir, &ctx.data_dir) {
                Ok(dir) => (dir.join(file), None),
                Err(err) => (
                    profiles::expand_home(def.config_dir_raw.trim(), &ctx.home_dir).join(file),
                    Some(format!("{err:#}")),
                ),
            };
        out.push(Target {
            id: format!("{PROFILE_PREFIX}{}", def.id),
            name: format!("{} 实例 Hooks", def.display_name),
            group: "profile-hooks",
            kind,
            agent_id: def.id.clone(),
            profile_id: Some(def.id.clone()),
            display: format!("{}/{file}", def.config_dir_raw.trim().trim_end_matches('/')),
            path,
            enabled: def.enabled,
            blocked,
            needs_existing_dir: true,
        });
    }
    out
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
        // 只有 command 类型有专属字段要求；其他类型（http/prompt/agent…）原样放行
        if is_command(hook_map) {
            let command = hook_map
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("");
            if command.trim().is_empty() {
                bail!("hook.command 必须是非空字符串");
            }
        }
    }
    Ok(())
}

/// 这条 hook 是否 command 类型。
fn is_command(hook: &Map<String, Value>) -> bool {
    hook.get("type").and_then(Value::as_str) == Some("command")
}

/// command 类型 hook 的命令；其他类型为 None（不参与签名匹配）。
fn command_of(hook: &Value) -> Option<&str> {
    hook.as_object()
        .filter(|map| is_command(map))
        .and_then(|map| map.get("command"))
        .and_then(Value::as_str)
}

/// hooks 对象（事件 → 条目数组）的结构校验：直接编辑写入前与读旧库来源共用。
pub(crate) fn validate_hooks_map(raw: Option<&Value>) -> Result<HookMap> {
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

/// 校验并拆分旧版本来源对象（旧库行与旧备份）。Claude 只允许 hooks/removed_signatures；
/// Codex 只允许 hooks。错误不回显来源内容。
pub(crate) fn validate_source(
    source_key: &str,
    obj: &Map<String, Value>,
) -> Result<(HookMap, Vec<RemovedRule>)> {
    match source_key {
        LEGACY_KEY_CLAUDE => {
            for key in obj.keys() {
                if key != "hooks" && key != "removed_signatures" {
                    bail!("hooks.claude 来源只允许 hooks 与 removed_signatures 字段");
                }
            }
            let hooks = validate_hooks_map(obj.get("hooks").or(Some(&Value::Object(Map::new()))))?;
            let removed = validate_removed_signatures(obj.get("removed_signatures"))?;
            Ok((hooks, removed))
        }
        LEGACY_KEY_CODEX => {
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

/// 编译内置的旧版本默认来源（去掉 settings_path/hooks_path）。
pub(crate) fn builtin_source(kind: HookKind) -> (HookMap, Vec<RemovedRule>) {
    let raw = match kind {
        HookKind::Claude => RAW_BUILTIN_CLAUDE,
        HookKind::Codex => RAW_BUILTIN_CODEX,
    };
    let mut obj: Map<String, Value> =
        serde_json::from_str(raw).expect("内置 hooks 默认是合法 JSON 对象");
    obj.remove("settings_path");
    obj.remove("hooks_path");
    validate_source(kind.legacy_key(), &obj).expect("内置 hooks 默认结构合法")
}

/// 从 hooks 值容错收集 command 类型 hook 的签名（结构损坏处跳过），保持出现顺序、去重。
pub(crate) fn collect_signatures(hooks: &Value) -> Vec<Signature> {
    let mut signatures = Vec::new();
    let Some(map) = hooks.as_object() else {
        return signatures;
    };
    for (event_name, entries) in map {
        let Some(entries) = entries.as_array() else {
            continue;
        };
        for entry in entries {
            let Some(entry) = entry.as_object() else {
                continue;
            };
            let matcher = entry
                .get("matcher")
                .and_then(Value::as_str)
                .map(str::to_string);
            let Some(hooks_list) = entry.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            for hook in hooks_list {
                if let Some(command) = command_of(hook) {
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

fn rule_matches(rule: &RemovedRule, signature: &Signature) -> bool {
    let (event, matcher, command, command_contains) = rule;
    let (s_event, s_matcher, s_command) = signature;
    if s_event != event {
        return false;
    }
    if let Some(m) = matcher {
        if s_matcher.as_deref() != Some(m.as_str()) {
            return false;
        }
    }
    if let Some(c) = command {
        if s_command != c {
            return false;
        }
    }
    if let Some(sub) = command_contains {
        if !s_command.contains(sub.as_str()) {
            return false;
        }
    }
    true
}

/// 移除满足条件的 command 类型 hook（其他类型不动）；条目或事件因此变空时一并移除，
/// 保证结果仍是合法结构。
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
                .filter(|hook| match command_of(hook) {
                    Some(command) => !drop(event_name, matcher.as_deref(), command),
                    None => true,
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

/// 移除某个签名的全部 hook。
pub(crate) fn remove_signature(hooks: &Value, signature: &Signature) -> Value {
    strip_hooks_where(hooks, &|event, matcher, command| {
        event == signature.0 && matcher == signature.1.as_deref() && command == signature.2
    })
}

/// 加入一个 hook：放进同事件下条目字段（matcher 等）完全相同的第一个条目，没有就新建条目。
pub(crate) fn add_hook(
    hooks: &Value,
    event: &str,
    entry_fields: &Map<String, Value>,
    hook: &Value,
) -> Value {
    let mut map = hooks.as_object().cloned().unwrap_or_default();
    let list = map
        .entry(event.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !list.is_array() {
        *list = Value::Array(Vec::new());
    }
    let entries = list.as_array_mut().expect("数组");
    let same_fields = |entry: &Value| {
        entry.as_object().is_some_and(|obj| {
            let mut fields = obj.clone();
            fields.remove("hooks");
            &fields == entry_fields && obj.get("hooks").is_some_and(Value::is_array)
        })
    };
    if let Some(entry) = entries.iter_mut().find(|entry| same_fields(entry)) {
        entry["hooks"]
            .as_array_mut()
            .expect("数组")
            .push(hook.clone());
    } else {
        let mut entry = entry_fields.clone();
        entry.insert("hooks".to_string(), Value::Array(vec![hook.clone()]));
        entries.push(Value::Object(entry));
    }
    Value::Object(map)
}

/// 实际 hooks 里的 hook 条数（界面展示用）。
pub(crate) fn hook_count(hooks: &Value) -> usize {
    hooks
        .as_object()
        .map(|map| {
            map.values()
                .filter_map(Value::as_array)
                .flatten()
                .filter_map(|entry| entry.get("hooks").and_then(Value::as_array))
                .map(Vec::len)
                .sum()
        })
        .unwrap_or(0)
}

/// 建议 id：前缀 + 内容摘要前 16 位。同一内容每次得到同一个 id，status 与 apply 对得上。
pub(crate) fn suggestion_id(prefix: &str, parts: &[&str]) -> String {
    let digest = sha256_hex(&serde_json::to_string(parts).expect("JSON"));
    format!("{prefix}-{}", &digest[..16])
}

pub(crate) fn valid_suggestion_id(id: &str, prefix: &str) -> bool {
    id.strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|hex| {
            hex.len() == 16
                && hex
                    .chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        })
}

pub(crate) const CLEANUP_PREFIX: &str = "cleanup";
pub(crate) const LEGACY_PREFIX: &str = "legacy";

/// 一条清理建议：实际 hooks 里命中内置 removed_signatures 的 hook（按签名去重）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cleanup {
    pub id: String,
    pub signature: Signature,
}

impl Cleanup {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "event": self.signature.0,
            "matcher": self.signature.1,
            "command": self.signature.2,
        })
    }
}

/// 内置 removed_signatures 在实际 hooks 里命中的条目；没有命中就是空。
pub(crate) fn cleanup_suggestions(kind: HookKind, hooks: &Value) -> Vec<Cleanup> {
    let (_, rules) = builtin_source(kind);
    collect_signatures(hooks)
        .into_iter()
        .filter(|signature| rules.iter().any(|rule| rule_matches(rule, signature)))
        .map(|signature| Cleanup {
            id: suggestion_id(
                CLEANUP_PREFIX,
                &[
                    &signature.0,
                    signature.1.as_deref().unwrap_or(""),
                    &signature.2,
                ],
            ),
            signature,
        })
        .collect()
}

/// 目标文件里的 hooks；没有 hooks 键时为空对象。
pub(crate) fn hooks_of(root: &Value) -> Value {
    root.get("hooks")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()))
}

/// 写入基准：Claude 只对 hooks 子树取摘要（用户改 settings.json 其他键不影响），
/// 没有 hooks 键为 missing；Codex 取 hooks.json 字节摘要，文件不存在为 missing。
pub(crate) fn base_of(kind: HookKind, file: Option<(&[u8], &Value)>) -> String {
    match (kind, file) {
        (_, None) => BASE_MISSING.to_string(),
        (HookKind::Codex, Some((bytes, _))) => sha256_hex_bytes(bytes),
        (HookKind::Claude, Some((_, root))) => match root.get("hooks") {
            Some(hooks) => sha256_hex_bytes(&serde_json::to_vec(hooks).expect("JSON")),
            None => BASE_MISSING.to_string(),
        },
    }
}

/// 把新 hooks 写进目标文件内容：其余根级键原样保留（位置不变）。Claude 的新 hooks 为空时
/// 移除 hooks 键；Codex 保留 `"hooks": {}`。文件不存在且新 hooks 为空时返回 None（不建文件）。
pub(crate) fn render_target(
    kind: HookKind,
    existing: Option<&Value>,
    hooks: &HookMap,
) -> Option<Vec<u8>> {
    if existing.is_none() && hooks.is_empty() {
        return None;
    }
    let mut root = existing
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if hooks.is_empty() && kind == HookKind::Claude {
        root.shift_remove("hooks");
    } else {
        root.insert("hooks".to_string(), Value::Object(hooks.clone()));
    }
    Some(render_bytes(&Value::Object(root)))
}

/// 写盘字节：pretty JSON（2 空格）+ 单个换行。
pub(crate) fn render_bytes(payload: &Value) -> Vec<u8> {
    let mut text = serde_json::to_string_pretty(payload).expect("render");
    text.push('\n');
    text.into_bytes()
}

/// 目标文件 hooks 子树的骨架校验（脱敏、不回显内容）：hooks 缺失视为空
/// 对象可用；存在时必须是对象，每个 event 名非空、值必须是非空数组，
/// 每个 entry 必须是对象且带非空 hooks 数组，每个 hook 必须是对象；
/// command 类型的 command 须为非空字符串，其他类型不看专属字段；
/// matcher 若存在必须是字符串。未知附加字段允许并保留。
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
                if !is_command(hook_map) {
                    continue;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

    fn ctx() -> Context {
        Context {
            data_dir: PathBuf::from("/home/u/Library/Application Support/agent-manager"),
            home_dir: PathBuf::from("/home/u"),
        }
    }

    fn hook(command: &str) -> Value {
        json!({"type": "command", "command": command})
    }

    #[test]
    fn main_targets_first_then_profiles_with_their_own_file() {
        let mut policy = store::builtin_policy();
        policy["agents"]["definitions"]["zz-work"] = json!({
            "display_name": "Codex 工作", "profile": {"base_agent": "codex", "config_dir": "~/.codex-work"}
        });
        policy["agents"]["definitions"]["cc-ly"] = json!({
            "display_name": "Claude ly", "profile": {"base_agent": "claude-code", "config_dir": "~/.claude-ly/"}
        });
        policy["agents"]["zz-work"] = json!({"enabled": false});
        let targets = resolve_targets(&ctx(), &policy);
        let ids: Vec<&str> = targets.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "hooks:claude",
                "hooks:codex",
                "hooks:profile:cc-ly",
                "hooks:profile:zz-work"
            ]
        );
        assert_eq!(
            targets[0].path,
            PathBuf::from("/home/u/.claude/settings.json")
        );
        assert_eq!(targets[1].path, PathBuf::from("/home/u/.codex/hooks.json"));
        assert_eq!(
            targets[2].path,
            PathBuf::from("/home/u/.claude-ly/settings.json")
        );
        assert_eq!(targets[2].display, "~/.claude-ly/settings.json");
        assert_eq!(targets[2].kind, HookKind::Claude);
        assert_eq!(
            targets[3].path,
            PathBuf::from("/home/u/.codex-work/hooks.json")
        );
        assert!(!targets[3].enabled, "停用的实例目标同样停用");
        assert!(targets[0].is_main() && !targets[2].is_main());
    }

    #[test]
    fn target_id_shapes() {
        assert!(valid_target_id("hooks:claude"));
        assert!(valid_target_id("hooks:codex"));
        assert!(valid_target_id("hooks:profile:cc-ly"));
        for bad in [
            "hooks:",
            "hooks:profile:",
            "hooks:profile:Bad",
            "claude",
            "",
        ] {
            assert!(!valid_target_id(bad), "{bad}");
        }
    }

    #[test]
    fn claude_base_only_covers_the_hooks_subtree() {
        let a = json!({"model": "a", "hooks": {"Stop": [{"hooks": [hook("x")]}]}});
        let mut b = a.clone();
        b["model"] = json!("b");
        b["permissions"] = json!({"allow": ["Bash"]});
        let base = |v: &Value| base_of(HookKind::Claude, Some((b"ignored", v)));
        assert_eq!(base(&a), base(&b), "改其他键不影响 base");
        let mut c = a.clone();
        c["hooks"]["Stop"][0]["hooks"][0]["command"] = json!("y");
        assert_ne!(base(&a), base(&c));
        assert_eq!(base(&json!({"model": "a"})), BASE_MISSING);
        assert_eq!(base_of(HookKind::Claude, None), BASE_MISSING);
    }

    #[test]
    fn render_replaces_only_the_hooks_key_in_place() {
        let existing =
            json!({"env": {"A": "1"}, "hooks": {"Stop": [{"hooks": [hook("x")]}]}, "model": "m"});
        let mut hooks = Map::new();
        hooks.insert("SessionStart".into(), json!([{"hooks": [hook("y")]}]));
        let bytes = render_target(HookKind::Claude, Some(&existing), &hooks).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let parsed: Value = serde_json::from_str(&text).unwrap();
        let keys: Vec<&String> = parsed.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["env", "hooks", "model"], "hooks 键原位替换");
        assert_eq!(parsed["hooks"], Value::Object(hooks));

        let emptied = render_target(HookKind::Claude, Some(&existing), &Map::new()).unwrap();
        let parsed: Value = serde_json::from_slice(&emptied).unwrap();
        assert!(parsed.get("hooks").is_none(), "Claude 空 hooks 移除该键");
        assert_eq!(parsed["model"], "m");

        let codex =
            render_target(HookKind::Codex, Some(&json!({"hooks": {}})), &Map::new()).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&codex).unwrap(),
            json!({"hooks": {}})
        );
        assert!(render_target(HookKind::Claude, None, &Map::new()).is_none());
        assert!(render_target(HookKind::Codex, None, &Map::new()).is_none());
    }

    #[test]
    fn cleanup_matches_builtin_removed_signatures_only() {
        let hooks = json!({
            "SubagentStop": [{"hooks": [hook("bash ~/x/agent-rule/hooks/check-rules-build.sh")]}],
            "PreToolUse": [
                {"matcher": "Edit|MultiEdit|Write", "hooks": [hook("sh agent-rule/hooks/block-dist-edits.sh"), hook("keep.sh")]},
                {"matcher": "Bash", "hooks": [hook("sh agent-rule/hooks/block-dist-edits.sh")]}
            ]
        });
        let found = cleanup_suggestions(HookKind::Claude, &hooks);
        let commands: Vec<&str> = found.iter().map(|c| c.signature.2.as_str()).collect();
        assert_eq!(
            commands,
            [
                "bash ~/x/agent-rule/hooks/check-rules-build.sh",
                "sh agent-rule/hooks/block-dist-edits.sh"
            ],
            "matcher 不符的不算"
        );
        assert!(found
            .iter()
            .all(|c| valid_suggestion_id(&c.id, CLEANUP_PREFIX)));
        assert!(cleanup_suggestions(HookKind::Codex, &hooks).is_empty());
    }

    #[test]
    fn non_command_types_pass_validation_and_never_match_signatures() {
        let hooks = json!({
            "PreToolUse": [{"matcher": "Edit|MultiEdit|Write", "hooks": [
                {"type": "http", "url": "https://x", "command": "sh agent-rule/hooks/block-dist-edits.sh"},
                {"type": "prompt", "prompt": "check"},
                {"type": "agent", "prompt": "review", "timeout": 60},
                {"type": "future-kind", "anything": [1, 2]},
                {"type": "command", "command": "keep.sh"}
            ]}]
        });
        assert!(validate_hooks_map(Some(&hooks)).is_ok());
        assert!(validate_target_hooks(&json!({"hooks": hooks.clone()})).is_ok());
        assert_eq!(hook_count(&hooks), 5);
        let signatures = collect_signatures(&hooks);
        assert_eq!(signatures.len(), 1, "只有 command 类型参与签名");
        assert!(
            cleanup_suggestions(HookKind::Claude, &hooks).is_empty(),
            "http 类型即使带 command 字段也不算清理建议"
        );
        let removed = remove_signature(&hooks, &signatures[0]);
        assert_eq!(
            removed["PreToolUse"][0]["hooks"].as_array().unwrap().len(),
            4
        );
        assert_eq!(
            removed["PreToolUse"][0]["hooks"][0],
            hooks["PreToolUse"][0]["hooks"][0]
        );
        // command 类型仍要求非空 command
        let bad = json!({"Stop": [{"hooks": [{"type": "command", "command": " "}]}]});
        assert!(validate_hooks_map(Some(&bad)).is_err());
        assert!(validate_target_hooks(&json!({"hooks": bad})).is_err());
    }

    #[test]
    fn remove_and_add_keep_structure_valid() {
        let hooks = json!({"Stop": [{"matcher": "A", "hooks": [hook("x"), hook("y")]}]});
        let removed = remove_signature(&hooks, &("Stop".into(), Some("A".into()), "x".into()));
        assert_eq!(
            removed,
            json!({"Stop": [{"matcher": "A", "hooks": [hook("y")]}]})
        );
        let gone = remove_signature(&removed, &("Stop".into(), Some("A".into()), "y".into()));
        assert_eq!(gone, json!({}), "条目与事件变空时一并移除");

        let mut fields = Map::new();
        fields.insert("matcher".into(), json!("A"));
        let added = add_hook(&removed, "Stop", &fields, &hook("z"));
        assert_eq!(
            added,
            json!({"Stop": [{"matcher": "A", "hooks": [hook("y"), hook("z")]}]}),
            "同 matcher 并进已有条目"
        );
        let added = add_hook(&added, "Start", &Map::new(), &hook("w"));
        assert_eq!(added["Start"], json!([{"hooks": [hook("w")]}]));
        assert!(validate_hooks_map(Some(&added)).is_ok());
        assert_eq!(hook_count(&added), 3);
    }

    #[test]
    fn suggestion_ids_are_stable_and_shaped() {
        let a = suggestion_id(LEGACY_PREFIX, &["hooks.claude", "Stop", "", "x"]);
        assert_eq!(
            a,
            suggestion_id(LEGACY_PREFIX, &["hooks.claude", "Stop", "", "x"])
        );
        assert_ne!(
            a,
            suggestion_id(LEGACY_PREFIX, &["hooks.codex", "Stop", "", "x"])
        );
        assert!(valid_suggestion_id(&a, LEGACY_PREFIX));
        assert!(!valid_suggestion_id(&a, CLEANUP_PREFIX));
        for bad in [
            "legacy-",
            "legacy-XYZ",
            "legacy-0123456789abcdef0",
            "cleanup0123456789abcdef",
        ] {
            assert!(
                !valid_suggestion_id(bad, LEGACY_PREFIX)
                    && !valid_suggestion_id(bad, CLEANUP_PREFIX),
                "{bad}"
            );
        }
    }
}
