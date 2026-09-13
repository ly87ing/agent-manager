//! Tauri bridge for the shared native engine.

use serde::Serialize;
use tauri::Manager;

pub const CONTRACT_ID: &str = agent_manager_engine::CONTRACT_ID;

/// 序列化后经 invoke 返回给前端的结构化错误。
#[derive(Debug, Clone, Serialize)]
pub struct EngineError {
    pub kind: String,
    pub message: String,
    pub exit_code: Option<i32>,
    pub stderr: Option<String>,
}

impl EngineError {
    fn new(kind: &str, message: impl Into<String>) -> Self {
        Self {
            kind: kind.to_string(),
            message: message.into(),
            exit_code: None,
            stderr: None,
        }
    }
}

// ---------- 命令面白名单 ----------

/// 支持 status + plan 的域（secrets 只有 status；system 只有 audit / refresh-upstream）。
/// 支持 status/plan/apply 的域（对齐引擎 APPLY_DOMAINS；secrets 只读）。
const READ_WRITE_DOMAINS: &[&str] = &["agents", "mcp", "skills", "rules", "hooks", "aliases"];

pub fn validate_command(command: &str) -> Result<(), EngineError> {
    let rejected = || {
        EngineError::new(
            "unknown_command",
            format!(
                "命令 '{command}' 不在契约命令面内（允许 <domain>:status|plan|apply / system:audit|refresh-upstream / runs:* / policy:* / sources:* / db:*）"
            ),
        )
    };
    let Some((domain, verb)) = command.split_once(':') else {
        return Err(rejected());
    };
    if domain.is_empty() || verb.is_empty() || verb.contains(':') {
        return Err(rejected());
    }
    let allowed = match (domain, verb) {
        ("system", "audit" | "refresh-upstream") => true,
        ("secrets", "status") => true,
        ("runs", "list" | "get" | "clear") => true,
        ("logs", "clear") => true,
        ("policy", "get" | "set" | "delete") => true,
        ("sources", "get" | "set") => true,
        ("db", "status" | "migrate") => true,
        (d, "status" | "plan" | "apply") => READ_WRITE_DOMAINS.contains(&d),
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(rejected())
    }
}

/// 参数取值类型（按类型校验；不经 shell，argv 直传引擎）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArgValueKind {
    /// 纯开关（--dry-run / --allow-uninstall），不带值。
    Flag,
    /// 元素 id，如 mcp:grok-search / skill:agent-skills/find-skills。
    ElementId,
    /// 运行 id（runs:get 的 --run），字母数字与 _/-。
    RunId,
    /// policy:set 落点层。
    PolicyLayer,
    /// policy:set 点分键路径（对齐引擎 key 段规则：字母/数字/_/-）。
    DotKey,
    /// policy:set 的 JSON 值——合法性即 JSON 可解析。
    JsonValue,
    /// agents:plan/apply 的显式生命周期动作：仅 install/upgrade/uninstall。
    AgentAction,
    SkillAction,
    McpAction,
}

/// 每个命令允许的 flag 集合（命令感知白名单）。
fn allowed_arg_specs(command: &str) -> Vec<(&'static str, ArgValueKind)> {
    let (domain, verb) = command.split_once(':').unwrap_or(("", ""));
    let mut specs: Vec<(&'static str, ArgValueKind)> = Vec::new();
    match (domain, verb) {
        ("sources", "get") => specs.push(("--key", ArgValueKind::DotKey)),
        ("sources", "set") => {
            specs.push(("--key", ArgValueKind::DotKey));
            specs.push(("--value", ArgValueKind::JsonValue));
            specs.push(("--dry-run", ArgValueKind::Flag));
        }
        ("system", "refresh-upstream") => specs.push(("--only", ArgValueKind::ElementId)),
        ("runs", "get") => {
            specs.push(("--run", ArgValueKind::RunId));
        }
        ("policy", "set") => {
            specs.push(("--layer", ArgValueKind::PolicyLayer));
            specs.push(("--key", ArgValueKind::DotKey));
            specs.push(("--value", ArgValueKind::JsonValue));
            specs.push(("--dry-run", ArgValueKind::Flag));
        }
        ("policy", "delete") => {
            specs.push(("--layer", ArgValueKind::PolicyLayer));
            specs.push(("--key", ArgValueKind::DotKey));
        }
        (_, "plan") => {
            specs.push(("--only", ArgValueKind::ElementId));
            if domain == "agents" {
                specs.push(("--action", ArgValueKind::AgentAction));
            }
            if domain == "skills" {
                specs.push(("--agent", ArgValueKind::ElementId));
                specs.push(("--action", ArgValueKind::SkillAction));
            }
            if domain == "mcp" {
                specs.push(("--agent", ArgValueKind::ElementId));
                specs.push(("--action", ArgValueKind::McpAction));
                specs.push(("--allow-takeover", ArgValueKind::Flag));
            }
        }
        (_, "apply") => {
            specs.push(("--dry-run", ArgValueKind::Flag));
            specs.push(("--only", ArgValueKind::ElementId));
            if domain == "agents" {
                specs.push(("--allow-uninstall", ArgValueKind::Flag));
                specs.push(("--action", ArgValueKind::AgentAction));
            }
            if domain == "skills" {
                specs.push(("--agent", ArgValueKind::ElementId));
                specs.push(("--action", ArgValueKind::SkillAction));
            }
            if domain == "mcp" {
                specs.push(("--agent", ArgValueKind::ElementId));
                specs.push(("--action", ArgValueKind::McpAction));
                specs.push(("--allow-takeover", ArgValueKind::Flag));
            }
        }
        _ => {}
    }
    specs
}

fn validate_element_id(value: &str) -> Result<(), EngineError> {
    let valid = !value.is_empty()
        && value
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_:/@+.-=".contains(c));
    if valid {
        Ok(())
    } else {
        Err(EngineError::new(
            "invalid_args",
            format!("非法元素 id '{value}'"),
        ))
    }
}

/// 前端生成的 run id 字符集（透传 AGENT_MANAGER_RUN_ID）。
pub fn validate_run_id(value: &str) -> Result<(), EngineError> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(EngineError::new(
            "invalid_args",
            format!("非法 run id '{value}'"),
        ))
    }
}

fn validate_arg_value(kind: ArgValueKind, value: &str) -> Result<(), EngineError> {
    match kind {
        ArgValueKind::Flag => Ok(()),
        ArgValueKind::ElementId => validate_element_id(value),
        ArgValueKind::RunId => validate_run_id(value),
        ArgValueKind::PolicyLayer => {
            if value == "local" {
                Ok(())
            } else {
                Err(EngineError::new(
                    "invalid_args",
                    format!("非法策略层 '{value}'（仅 local）"),
                ))
            }
        }
        ArgValueKind::DotKey => {
            let ok = !value.is_empty()
                && value.split('.').all(|seg| {
                    !seg.is_empty()
                        && seg
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                });
            if ok {
                Ok(())
            } else {
                Err(EngineError::new(
                    "invalid_args",
                    format!("非法 key 路径 '{value}'"),
                ))
            }
        }
        ArgValueKind::JsonValue => serde_json::from_str::<serde_json::Value>(value)
            .map(|_| ())
            .map_err(|err| {
                EngineError::new("invalid_args", format!("--value 不是合法 JSON: {err}"))
            }),
        ArgValueKind::AgentAction => match value {
            "install" | "upgrade" | "uninstall" => Ok(()),
            _ => Err(EngineError::new(
                "invalid_args",
                format!("非法动作 '{value}'（仅 install|upgrade|uninstall）"),
            )),
        },
        ArgValueKind::SkillAction => match value {
            "add" | "update" | "enable" | "disable" | "remove" => Ok(()),
            _ => Err(EngineError::new("invalid_args", "非法 skills action")),
        },
        ArgValueKind::McpAction => match value {
            "add" | "update" | "enable" | "disable" | "remove" => Ok(()),
            _ => Err(EngineError::new(
                "invalid_args",
                format!("非法 mcp action '{value}'（仅 add|update|enable|disable|remove）"),
            )),
        },
    }
}

pub fn validate_args(command: &str, args: &[String]) -> Result<(), EngineError> {
    let specs = allowed_arg_specs(command);
    let (domain, _) = command.split_once(':').unwrap_or(("", ""));
    let is_rules = domain == "rules";
    // sources 与 native 严格参数保持一致：拒绝重复 flag。
    let reject_duplicates = matches!(command, "sources:get" | "sources:set");
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    // Rules 严格契约：--only 的 flag 可重复但值不可重复；--dry-run 不可重复。
    let mut rules_only_seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut rules_dry_run_seen = false;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let Some((_, kind)) = specs.iter().find(|(flag, _)| flag == &arg.as_str()) else {
            return Err(EngineError::new(
                "invalid_args",
                format!("命令 '{command}' 不允许参数 '{arg}'"),
            ));
        };
        if reject_duplicates && !seen.insert(arg.as_str()) {
            return Err(EngineError::new(
                "invalid_args",
                format!("命令 '{command}' 重复提供参数 '{arg}'"),
            ));
        }
        if is_rules && arg == "--dry-run" {
            if rules_dry_run_seen {
                return Err(EngineError::new(
                    "invalid_args",
                    format!("命令 '{command}' 重复提供参数 '--dry-run'"),
                ));
            }
            rules_dry_run_seen = true;
        }
        if *kind != ArgValueKind::Flag {
            index += 1;
            let Some(value) = args.get(index) else {
                return Err(EngineError::new(
                    "invalid_args",
                    format!("参数 '{arg}' 缺少取值"),
                ));
            };
            validate_arg_value(*kind, value)?;
            if is_rules && arg == "--only" {
                if value != "rules:codex_global" && value != "rules:claude_global" {
                    return Err(EngineError::new(
                        "invalid_args",
                        format!(
                            "非法 rules 目标 '{value}'（仅 rules:codex_global|rules:claude_global）"
                        ),
                    ));
                }
                if !rules_only_seen.insert(value.as_str()) {
                    return Err(EngineError::new(
                        "invalid_args",
                        format!("命令 '{command}' 重复提供 --only 目标 '{value}'"),
                    ));
                }
            }
        }
        index += 1;
    }
    Ok(())
}

fn effective_args(mut args: Vec<String>, offline: bool) -> Vec<String> {
    if offline && !args.iter().any(|arg| arg == "--offline") {
        args.push("--offline".into());
    }
    args
}
fn dispatch_native(
    ctx: agent_manager_engine::Context,
    command: String,
    args: Vec<String>,
    run_id: String,
    offline: bool,
) -> Result<agent_manager_engine::Envelope, EngineError> {
    validate_command(&command)?;
    validate_args(&command, &args)?;
    validate_run_id(&run_id)?;
    // sources 只读写本地 SQLite 且 native 严格参数拒绝 --offline，不做离线注入。
    let args = if command.starts_with("sources:") {
        args
    } else {
        effective_args(args, offline)
    };
    Ok(agent_manager_engine::execute(
        &ctx,
        &agent_manager_engine::Request {
            command,
            args,
            run_id,
        },
    ))
}
#[tauri::command]
pub async fn engine_invoke(
    app: tauri::AppHandle,
    command: String,
    args: Option<Vec<String>>,
    run_id: Option<String>,
) -> Result<agent_manager_engine::Envelope, EngineError> {
    let data_dir = std::env::var_os("AGENT_MANAGER_DATA_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| app.path().app_data_dir().ok())
        .ok_or_else(|| EngineError::new("data_dir_unavailable", "无法定位应用数据目录"))?;
    let home_dir = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| EngineError::new("home_unavailable", "HOME 未设置"))?;
    let offline = crate::settings::current_preferences(&app)
        .map_err(|e| EngineError::new("settings_unavailable", e))?
        .offline;
    tauri::async_runtime::spawn_blocking(move || {
        dispatch_native(
            agent_manager_engine::Context { data_dir, home_dir },
            command,
            args.unwrap_or_default(),
            run_id.unwrap_or_else(|| format!("tauri-{}", std::process::id())),
            offline,
        )
    })
    .await
    .map_err(|e| EngineError::new("execution_failed", e.to_string()))?
}

pub fn maintenance_cleanup(app: &tauri::AppHandle) {
    let data_dir = std::env::var_os("AGENT_MANAGER_DATA_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| app.path().app_data_dir().ok());
    let home_dir = std::env::var_os("HOME").map(std::path::PathBuf::from);
    if let (Some(data_dir), Some(home_dir)) = (data_dir, home_dir) {
        let ctx = agent_manager_engine::Context { data_dir, home_dir };
        if let Err(err) = agent_manager_engine::native_logs::startup_maintenance(&ctx) {
            let sanitized =
                agent_manager_engine::native_logs::sanitize_diagnostic_message(&err.to_string());
            crate::settings::set_startup_maintenance_warning(Some(format!(
                "启动维护清理失败：{sanitized}"
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_whitelist_covers_contract_surface_and_rejects_injection() {
        for command in [
            "agents:status",
            "agents:plan",
            "agents:apply",
            "mcp:status",
            "mcp:plan",
            "mcp:apply",
            "skills:status",
            "skills:plan",
            "skills:apply",
            "rules:status",
            "rules:plan",
            "rules:apply",
            "hooks:status",
            "hooks:plan",
            "hooks:apply",
            "aliases:status",
            "aliases:plan",
            "aliases:apply",
            "secrets:status",
            "system:audit",
            "system:refresh-upstream",
            "runs:list",
            "runs:get",
            "runs:clear",
            "logs:clear",
            "policy:get",
            "policy:set",
            "policy:delete",
            "sources:get",
            "sources:set",
            "db:status",
            "db:migrate",
        ] {
            assert!(validate_command(command).is_ok(), "{command} should pass");
        }

        for command in [
            "agents:update",
            "secrets:plan",
            "secrets:apply",
            "system:status",
            // 2026-09-12 裁决退役：tools 域、snapshots 域、system:converge 全部拒绝
            "tools:status",
            "tools:plan",
            "tools:apply",
            "system:converge",
            "snapshots:list",
            "snapshots:restore",
            "snapshots:status",
            "runs:delete",
            "policy:drop",
            "sources:delete",
            "profile:init",
            "profile:status",
            "profile:pull",
            "profile:push",
            "db:drop",
            "bad:status",
            "unknown:apply",
            "update",
            "",
            "mcp:",
            ":status",
            "mcp:status:extra",
            "mcp:status --offline",
            "mcp;status",
            "mcp:status\n--offline",
        ] {
            assert!(
                validate_command(command).is_err(),
                "{command:?} should fail"
            );
        }
    }

    #[test]
    fn args_cover_command_aware_whitelist_and_reject_invalid_values() {
        for action in ["install", "upgrade", "uninstall"] {
            assert!(validate_args(
                "agents:plan",
                &[
                    "--only".into(),
                    "agents:codex".into(),
                    "--action".into(),
                    action.into()
                ]
            )
            .is_ok());
            assert!(validate_args(
                "agents:apply",
                &[
                    "--only".into(),
                    "agents:codex".into(),
                    "--action".into(),
                    action.into(),
                    "--allow-uninstall".into(),
                ]
            )
            .is_ok());
        }
        for action in ["add", "update", "enable", "disable", "remove"] {
            assert!(validate_args(
                "skills:plan",
                &[
                    "--only".into(),
                    "skill:repo/name".into(),
                    "--agent".into(),
                    "codex".into(),
                    "--action".into(),
                    action.into(),
                ]
            )
            .is_ok());
            assert!(validate_args(
                "skills:apply",
                &[
                    "--agent".into(),
                    "codex".into(),
                    "--action".into(),
                    action.into()
                ]
            )
            .is_ok());
        }

        for (command, args) in [
            (
                "system:refresh-upstream",
                vec!["--only".into(), "mcp:grok-search".into()],
            ),
            ("mcp:plan", vec!["--only".into(), "mcp:grok-search".into()]),
            (
                "mcp:plan",
                vec![
                    "--only".into(),
                    "mcp:grok-search".into(),
                    "--agent".into(),
                    "codex".into(),
                    "--action".into(),
                    "update".into(),
                ],
            ),
            (
                "mcp:apply",
                vec![
                    "--only".into(),
                    "mcp:grok-search".into(),
                    "--action".into(),
                    "disable".into(),
                    "--dry-run".into(),
                ],
            ),
            (
                "agents:apply",
                vec!["--allow-uninstall".into(), "--dry-run".into()],
            ),
            (
                "runs:get",
                vec!["--run".into(), "gui-20260906T010203Z-ab12cd".into()],
            ),
            (
                "policy:set",
                vec![
                    "--layer".into(),
                    "local".into(),
                    "--key".into(),
                    "mcp.enabled".into(),
                    "--value".into(),
                    "{\"a\":true}".into(),
                    "--dry-run".into(),
                ],
            ),
            (
                "policy:delete",
                vec![
                    "--layer".into(),
                    "local".into(),
                    "--key".into(),
                    "mcp.enabled".into(),
                ],
            ),
            ("sources:get", vec!["--key".into(), "rules.shared".into()]),
            (
                "sources:set",
                vec![
                    "--key".into(),
                    "rules.shared".into(),
                    "--value".into(),
                    "{}".into(),
                    "--dry-run".into(),
                ],
            ),
        ] {
            assert!(
                validate_args(command, &args).is_ok(),
                "{command} {args:?} should pass"
            );
        }

        for (command, args) in [
            ("mcp:status", vec!["--dry-run".into()]),
            ("mcp:apply", vec!["--allow-uninstall".into()]),
            ("mcp:plan", vec!["--unexpected".into(), "codex".into()]),
            ("agents:status", vec!["--action".into(), "install".into()]),
            ("skills:apply", vec!["--action".into(), "install".into()]),
            ("runs:list", vec!["--run".into(), "r1".into()]),
            ("policy:get", vec!["--dry-run".into()]),
            ("sources:set", vec!["--def-file".into(), "elsewhere".into()]),
            ("system:refresh-upstream", vec!["--only".into()]),
            // 已退役命令不再携带任何参数白名单
            ("system:converge", vec!["--dry-run".into()]),
            (
                "snapshots:restore",
                vec!["--run".into(), "20260905103000_12345".into()],
            ),
            ("runs:get", vec!["--run".into()]),
            (
                "runs:get",
                vec![
                    "--run".into(),
                    "r1".into(),
                    "--unexpected".into(),
                    "value".into(),
                ],
            ),
            ("sources:set", vec!["--key".into(), "../../secrets".into()]),
            ("sources:set", vec!["--key".into(), "a..b".into()]),
            ("sources:set", vec!["--value".into(), "{not json".into()]),
            (
                "policy:set",
                vec![
                    "--layer".into(),
                    "synced".into(),
                    "--key".into(),
                    "a".into(),
                    "--value".into(),
                    "true".into(),
                ],
            ),
            (
                "policy:delete",
                vec![
                    "--layer".into(),
                    "synced".into(),
                    "--key".into(),
                    "a".into(),
                ],
            ),
            ("policy:set", vec!["--layer".into(), "baseline".into()]),
            ("agents:plan", vec!["--action".into(), "reinstall".into()]),
            (
                "agents:apply",
                vec!["--action".into(), "install; rm -rf /".into()],
            ),
            ("skills:plan", vec!["--action".into(), "INSTALL".into()]),
            ("mcp:status", vec!["; rm -rf /".into()]),
            ("mcp:status", vec!["$(reboot)".into()]),
            ("mcp:status", vec!["mcp:status --offline".into()]),
        ] {
            assert!(
                validate_args(command, &args).is_err(),
                "{command} {args:?} should fail"
            );
        }

        for action in ["add", "update", "enable", "disable", "remove"] {
            assert!(
                validate_args(
                    "mcp:apply",
                    &[
                        "--only".into(),
                        "mcp:s1".into(),
                        "--agent".into(),
                        "claude-code".into(),
                        "--action".into(),
                        action.into(),
                    ]
                )
                .is_ok(),
                "mcp:apply with action {action} should pass"
            );
        }
        let bad_action_err = validate_args(
            "mcp:apply",
            &[
                "--only".into(),
                "mcp:s1".into(),
                "--action".into(),
                "install".into(),
            ],
        )
        .unwrap_err();
        assert!(
            bad_action_err.message.contains("非法 mcp action"),
            "MCP action error must mention mcp action, got: {}",
            bad_action_err.message
        );
        assert!(!bad_action_err.message.contains("skills"));

        // --allow-takeover：仅 mcp:plan / mcp:apply 放行（显式接管通道）
        for command in ["mcp:plan", "mcp:apply"] {
            assert!(
                validate_args(
                    command,
                    &[
                        "--only".into(),
                        "mcp:s1".into(),
                        "--agent".into(),
                        "claude-code".into(),
                        "--action".into(),
                        "add".into(),
                        "--allow-takeover".into(),
                    ]
                )
                .is_ok(),
                "{command} --allow-takeover should pass"
            );
        }
        for (command, args) in [
            ("mcp:status", vec!["--allow-takeover".into()]),
            ("skills:apply", vec!["--allow-takeover".into()]),
            ("agents:apply", vec!["--allow-takeover".into()]),
            ("rules:apply", vec!["--allow-takeover".into()]),
        ] {
            assert!(
                validate_args(command, &args).is_err(),
                "{command} --allow-takeover should fail"
            );
        }

        // Rules 严格契约：无 only / 单 only / 两个不同 only（顺序不限）均通过。
        for (command, args) in [
            ("rules:plan", vec![]),
            ("rules:apply", vec![]),
            (
                "rules:plan",
                vec!["--only".into(), "rules:codex_global".into()],
            ),
            (
                "rules:apply",
                vec!["--only".into(), "rules:claude_global".into()],
            ),
            (
                "rules:plan",
                vec![
                    "--only".into(),
                    "rules:claude_global".into(),
                    "--only".into(),
                    "rules:codex_global".into(),
                ],
            ),
            (
                "rules:apply",
                vec![
                    "--only".into(),
                    "rules:codex_global".into(),
                    "--only".into(),
                    "rules:claude_global".into(),
                    "--dry-run".into(),
                ],
            ),
        ] {
            assert!(
                validate_args(command, &args).is_ok(),
                "{command} {args:?} should pass"
            );
        }

        // Rules 拒绝：status 任意 raw arg（含 --offline，只能由 bridge 注入）、
        // 重复 only、未知/别名 target、缺值、plan --dry-run、apply 重复 --dry-run。
        for (command, args) in [
            ("rules:status", vec!["--offline".into()]),
            (
                "rules:status",
                vec!["--only".into(), "rules:codex_global".into()],
            ),
            ("rules:status", vec!["--dry-run".into()]),
            ("rules:status", vec!["positional".into()]),
            ("rules:plan", vec!["--offline".into()]),
            ("rules:apply", vec!["--offline".into()]),
            (
                "rules:plan",
                vec![
                    "--only".into(),
                    "rules:codex_global".into(),
                    "--only".into(),
                    "rules:codex_global".into(),
                ],
            ),
            ("rules:plan", vec!["--only".into(), "rules:unknown".into()]),
            ("rules:plan", vec!["--only".into(), "rules:codex".into()]),
            ("rules:plan", vec!["--only".into(), "codex".into()]),
            ("rules:plan", vec!["--only".into()]),
            ("rules:plan", vec!["--dry-run".into()]),
            ("rules:apply", vec!["--dry-run".into(), "--dry-run".into()]),
        ] {
            assert!(
                validate_args(command, &args).is_err(),
                "{command} {args:?} should fail"
            );
        }
    }

    #[test]
    fn run_id_validation_covers_valid_and_typical_invalid_values() {
        for value in [
            "read",
            "gui-20260906T010203Z-ab12cd",
            "20260905103000_12345",
            "A1_valid-run",
        ] {
            assert!(validate_run_id(value).is_ok(), "{value} should pass");
        }
        for value in [
            "",
            "with space",
            "slash/inside",
            "-leading-dash",
            "_leading-underscore",
            "$(evil)",
            "中文标识",
            "a.b",
            "a\nb",
            &"a".repeat(65),
        ] {
            assert!(validate_run_id(value).is_err(), "{value:?} should fail");
        }
    }

    #[test]
    fn offline_injection_is_idempotent() {
        assert_eq!(effective_args(vec![], true), vec!["--offline"]);
        assert_eq!(
            effective_args(vec!["--offline".into()], true),
            vec!["--offline"]
        );
        assert_eq!(
            effective_args(vec!["--dry-run".into()], true),
            vec!["--dry-run", "--offline"]
        );
        assert_eq!(
            effective_args(vec!["--dry-run".into()], false),
            vec!["--dry-run"]
        );
        assert_eq!(
            effective_args(vec!["--offline".into()], false),
            vec!["--offline"]
        );
    }

    #[test]
    fn system_audit_bridge_injects_offline_and_raw_args_stay_strict() {
        // raw validate_args 仍拒绝用户提供的 --offline（只能由 bridge 注入）
        assert!(validate_args("system:audit", &["--offline".into()]).is_err());
        assert!(validate_args("system:audit", &[]).is_ok());
        // offline=true 时 bridge 注入 --offline 后原生 system:audit 成功且零写
        let base = std::env::temp_dir().join(format!(
            "am-tauri-audit-test-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let ctx = agent_manager_engine::Context {
            data_dir: base.join("data"),
            home_dir: base.join("home"),
        };
        let envelope = dispatch_native(
            ctx,
            "system:audit".into(),
            vec![],
            "audit-test".into(),
            true,
        )
        .unwrap();
        assert!(envelope.ok, "{:?}", envelope.errors);
        assert!(envelope.mode.offline);
        assert_eq!(envelope.data["status"], "warning");
        assert!(envelope.data["sections"].as_object().unwrap().len() >= 8);
        assert!(!base.join("data").exists(), "audit 不得创建数据目录");
        assert!(!base.join("home").exists(), "audit 不得创建 home 目录");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn secrets_status_bridge_injects_offline_and_raw_args_stay_strict() {
        // raw validate_args 仍拒绝用户提供的 --offline（只能由 bridge 注入）
        assert!(validate_args("secrets:status", &["--offline".into()]).is_err());
        assert!(validate_args("secrets:status", &[]).is_ok());
        // offline=true 时 bridge 注入 --offline 后原生 secrets:status 成功且零写
        let base = std::env::temp_dir().join(format!(
            "am-tauri-secrets-test-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let ctx = agent_manager_engine::Context {
            data_dir: base.join("data"),
            home_dir: base.join("home"),
        };
        let envelope = dispatch_native(
            ctx,
            "secrets:status".into(),
            vec![],
            "secrets-test".into(),
            true,
        )
        .unwrap();
        assert!(envelope.ok, "{:?}", envelope.errors);
        assert!(envelope.mode.offline);
        assert_eq!(envelope.data["domain"], "secrets");
        assert_eq!(envelope.data["elements"][0]["id"], "secrets:mcp-env");
        assert!(!base.join("data").exists(), "status 不得创建数据目录");
        assert!(!base.join("home").exists(), "status 不得创建 home 目录");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn sources_args_match_native_strictness_and_skip_offline_injection() {
        assert!(validate_args("sources:get", &["--key".into(), "rules".into()]).is_ok());
        assert!(validate_args(
            "sources:set",
            &[
                "--key".into(),
                "rules.shared".into(),
                "--value".into(),
                "{\"revision\":\"r\",\"value\":\"v\"}".into(),
                "--dry-run".into(),
            ]
        )
        .is_ok());
        // 重复 flag 与 native 严格参数一致拒绝
        for (command, args) in [
            (
                "sources:get",
                vec![
                    "--key".into(),
                    "rules".into(),
                    "--key".into(),
                    "rules".into(),
                ],
            ),
            (
                "sources:set",
                vec![
                    "--key".into(),
                    "rules.shared".into(),
                    "--value".into(),
                    "{}".into(),
                    "--value".into(),
                    "{}".into(),
                ],
            ),
            (
                "sources:set",
                vec![
                    "--key".into(),
                    "rules.shared".into(),
                    "--value".into(),
                    "{}".into(),
                    "--dry-run".into(),
                    "--dry-run".into(),
                ],
            ),
        ] {
            let err = validate_args(command, &args).unwrap_err();
            assert!(
                err.message.contains("重复"),
                "{command} {args:?} should mention duplicate, got: {}",
                err.message
            );
        }
        assert!(validate_args("sources:get", &["--offline".into()]).is_err());
        assert!(validate_args("sources:set", &["positional".into()]).is_err());

        // dispatch_native 对 sources 不注入 --offline：离线偏好下仍直连成功且零写
        let root = std::env::temp_dir().join(format!("am-sources-bridge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let c = || agent_manager_engine::Context {
            data_dir: root.join("data"),
            home_dir: root.join("home"),
        };
        let env = dispatch_native(
            c(),
            "sources:get".into(),
            vec!["--key".into(), "rules".into()],
            "sources-offline".into(),
            true,
        )
        .unwrap();
        assert!(env.ok, "{:?}", env.errors);
        assert!(!env.mode.offline, "sources 不注入 --offline");
        assert_eq!(env.data["sources"].as_array().unwrap().len(), 3);
        assert!(!root.join("data").exists(), "sources:get 零写");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rules_status_and_plan_inject_offline_via_bridge() {
        // raw --offline 仍被 validate_args 拒绝，只能由 bridge 注入。
        assert!(validate_args("rules:status", &["--offline".into()]).is_err());
        assert!(validate_args("rules:plan", &["--offline".into()]).is_err());

        let root = std::env::temp_dir().join(format!(
            "am-rules-bridge-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let c = || agent_manager_engine::Context {
            data_dir: root.join("data"),
            home_dir: root.join("home"),
        };
        let env = dispatch_native(
            c(),
            "rules:status".into(),
            vec![],
            "rules-bridge".into(),
            true,
        )
        .unwrap();
        assert!(env.ok, "{:?}", env.errors);
        assert!(env.mode.offline, "bridge 必须注入 --offline");
        assert_eq!(env.data["elements"].as_array().unwrap().len(), 2);
        assert!(!root.join("data").exists(), "rules:status 零写");
        assert!(!root.join("home").exists(), "rules:status 零写");

        let env = dispatch_native(
            c(),
            "rules:plan".into(),
            vec![],
            "rules-bridge".into(),
            true,
        )
        .unwrap();
        assert!(env.ok, "{:?}", env.errors);
        assert!(env.mode.offline);
        assert_eq!(env.data["actions"].as_array().unwrap().len(), 2);
        assert!(!root.join("data").exists(), "rules:plan 零写");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn dispatch_read_dry_write_read() {
        let root = std::env::temp_dir().join(format!("am-dispatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let c = || agent_manager_engine::Context {
            data_dir: root.join("data"),
            home_dir: root.join("home"),
        };
        assert!(
            dispatch_native(c(), "policy:get".into(), vec![], "read".into(), false)
                .unwrap()
                .ok
        );
        assert!(
            dispatch_native(
                c(),
                "policy:set".into(),
                vec![
                    "--layer".into(),
                    "local".into(),
                    "--key".into(),
                    "agents.x".into(),
                    "--value".into(),
                    "true".into(),
                    "--dry-run".into()
                ],
                "dry".into(),
                false
            )
            .unwrap()
            .ok
        );
        assert!(!root.join("data").exists());
        assert!(
            dispatch_native(
                c(),
                "policy:set".into(),
                vec![
                    "--layer".into(),
                    "local".into(),
                    "--key".into(),
                    "agents.x".into(),
                    "--value".into(),
                    "true".into()
                ],
                "write".into(),
                false
            )
            .unwrap()
            .ok
        );
        assert_eq!(
            dispatch_native(c(), "policy:get".into(), vec![], "read".into(), false)
                .unwrap()
                .data["effective"]["agents"]["x"],
            true
        );

        let serialized = serde_json::to_value(EngineError::new("test", "message")).unwrap();
        assert_eq!(serialized["kind"], "test");
        assert_eq!(serialized["message"], "message");
        assert!(serialized["exit_code"].is_null());
        assert!(serialized["stderr"].is_null());
        assert_eq!(serialized.as_object().unwrap().len(), 4);
        let _ = std::fs::remove_dir_all(root);
    }
}
