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
/// 支持 status/plan/apply 的域（对齐引擎 APPLY_DOMAINS；secrets 只读、profile 单列）。
const READ_WRITE_DOMAINS: &[&str] = &[
    "agents", "mcp", "skills", "rules", "hooks", "tools", "aliases",
];

pub fn validate_command(command: &str) -> Result<(), EngineError> {
    let rejected = || {
        EngineError::new(
            "unknown_command",
            format!(
                "命令 '{command}' 不在契约命令面内（允许 <domain>:status|plan|apply / system:audit|refresh-upstream|converge / snapshots:* / runs:* / policy:* / profile:*）"
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
        ("system", "audit" | "refresh-upstream" | "converge") => true,
        ("secrets", "status") => true,
        ("snapshots", "list" | "restore") => true,
        ("runs", "list" | "get") => true,
        ("policy", "get" | "set" | "delete") => true,
        ("sources", "get" | "set") => true,
        ("profile", "status" | "pull" | "push") => true,
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
    /// 运行 id（runs:get / snapshots:restore 的 --run），字母数字与 _/-。
    RunId,
    /// runs:get 的 --lines，非负整数。
    Lines,
    /// policy:set 落点层。
    PolicyLayer,
    /// policy:set 点分键路径（对齐引擎 key 段规则：字母/数字/_/-）。
    DotKey,
    /// policy:set 的 JSON 值——合法性即 JSON 可解析。
    JsonValue,
    /// agents:plan/apply 的显式生命周期动作：仅 install/upgrade/uninstall。
    AgentAction,
    SkillAction,
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
            specs.push(("--lines", ArgValueKind::Lines));
        }
        ("snapshots", "restore") => specs.push(("--run", ArgValueKind::RunId)),
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
        ("profile", "pull") => {
            specs.push(("--accept-remote", ArgValueKind::Flag));
            // 键级以远端为准：--only <顶层键>（点分键字符集，与 policy key 同规则）
            specs.push(("--only", ArgValueKind::DotKey));
        }
        ("profile", "push") => {
            specs.push(("--dry-run", ArgValueKind::Flag));
            specs.push(("--force", ArgValueKind::Flag));
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
        }
        ("system", "converge") => specs.push(("--dry-run", ArgValueKind::Flag)),
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
        ArgValueKind::Lines => {
            let ok = !value.is_empty()
                && value.bytes().all(|b| b.is_ascii_digit())
                && value.parse::<u32>().is_ok_and(|n| n <= 100_000);
            if ok {
                Ok(())
            } else {
                Err(EngineError::new(
                    "invalid_args",
                    format!("非法行数 '{value}'"),
                ))
            }
        }
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
    }
}

pub fn validate_args(command: &str, args: &[String]) -> Result<(), EngineError> {
    let specs = allowed_arg_specs(command);
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let Some((_, kind)) = specs.iter().find(|(flag, _)| flag == &arg.as_str()) else {
            return Err(EngineError::new(
                "invalid_args",
                format!("命令 '{command}' 不允许参数 '{arg}'"),
            ));
        };
        if *kind != ArgValueKind::Flag {
            index += 1;
            let Some(value) = args.get(index) else {
                return Err(EngineError::new(
                    "invalid_args",
                    format!("参数 '{arg}' 缺少取值"),
                ));
            };
            validate_arg_value(*kind, value)?;
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
    let args = effective_args(args, offline);
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
            "tools:status",
            "tools:plan",
            "tools:apply",
            "aliases:status",
            "aliases:plan",
            "aliases:apply",
            "secrets:status",
            "system:audit",
            "system:refresh-upstream",
            "system:converge",
            "snapshots:list",
            "snapshots:restore",
            "runs:list",
            "runs:get",
            "policy:get",
            "policy:set",
            "policy:delete",
            "sources:get",
            "sources:set",
            "profile:status",
            "profile:pull",
            "profile:push",
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
            "snapshots:status",
            "runs:delete",
            "policy:drop",
            "sources:delete",
            "profile:init",
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
                "mcp:apply",
                vec![
                    "--only".into(),
                    "mcp:grok-search".into(),
                    "--dry-run".into(),
                ],
            ),
            (
                "agents:apply",
                vec!["--allow-uninstall".into(), "--dry-run".into()],
            ),
            ("system:converge", vec!["--dry-run".into()]),
            (
                "runs:get",
                vec![
                    "--run".into(),
                    "gui-20260906T010203Z-ab12cd".into(),
                    "--lines".into(),
                    "200".into(),
                ],
            ),
            (
                "snapshots:restore",
                vec!["--run".into(), "20260905103000_12345".into()],
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
            (
                "profile:pull",
                vec!["--accept-remote".into(), "--only".into(), "mcp".into()],
            ),
            ("profile:push", vec!["--dry-run".into(), "--force".into()]),
        ] {
            assert!(
                validate_args(command, &args).is_ok(),
                "{command} {args:?} should pass"
            );
        }

        for (command, args) in [
            ("mcp:status", vec!["--dry-run".into()]),
            ("mcp:apply", vec!["--allow-uninstall".into()]),
            ("mcp:plan", vec!["--agent".into(), "codex".into()]),
            ("agents:status", vec!["--action".into(), "install".into()]),
            ("skills:apply", vec!["--action".into(), "install".into()]),
            ("profile:push", vec!["--accept-remote".into()]),
            ("profile:pull", vec!["--force".into()]),
            ("runs:list", vec!["--run".into(), "r1".into()]),
            ("policy:get", vec!["--dry-run".into()]),
            ("sources:set", vec!["--def-file".into(), "elsewhere".into()]),
            ("system:refresh-upstream", vec!["--only".into()]),
            ("runs:get", vec!["--lines".into()]),
            ("snapshots:restore", vec!["--run".into(), "../etc".into()]),
            ("runs:get", vec!["--lines".into(), "abc".into()]),
            ("runs:get", vec!["--lines".into(), "100001".into()]),
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
