//! rules 域模型：规则目标。
//!
//! 生效规则以各助手的规则文件为准：主 Claude（`~/.claude/CLAUDE.md`）、主 Codex
//! （`~/.codex/AGENTS.md`），以及每个实例配置目录下对应的规则文件。本工具不再拼接
//! 共享与助手专用来源，只保存用户模板（见 templates.rs）；推荐的规则内容来自技能来源
//! （见 sources.rs），不编进程序。

use std::path::PathBuf;

use serde_json::Value;

use crate::native::Context;
use crate::native_profiles::model as profiles;

pub(crate) const TARGET_CODEX: &str = "rules:codex_global";
pub(crate) const TARGET_CLAUDE: &str = "rules:claude_global";
const PROFILE_PREFIX: &str = "rules:profile:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleKind {
    Claude,
    Codex,
}

impl RuleKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            RuleKind::Claude => "claude",
            RuleKind::Codex => "codex",
        }
    }
}

/// 一个规则目标：路径、归属与能否读写的静态判定（文件内容在 status 探测时读取）。
#[derive(Debug, Clone)]
pub(crate) struct Target {
    pub id: String,
    pub name: String,
    pub group: &'static str,
    pub kind: RuleKind,
    /// 主助手为 agent id；实例为实例 id。
    pub agent_id: String,
    pub profile_id: Option<String>,
    /// 展示路径（`~/...`）。
    pub display: String,
    pub path: PathBuf,
    pub enabled: bool,
    /// 目标本身不可用的原因（实例配置目录非法等）。
    pub blocked: Option<String>,
    /// 实例目标：配置目录由实例域创建，目录不存在时规则域不写。
    pub needs_existing_dir: bool,
}

/// 目标 id 形状校验：两个全局目标，或 `rules:profile:<实例 id>`。
pub(crate) fn valid_target_id(id: &str) -> bool {
    id == TARGET_CODEX
        || id == TARGET_CLAUDE
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

fn global(ctx: &Context, policy: &Value, id: &str) -> Target {
    let (name, kind, agent_id, rel) = if id == TARGET_CODEX {
        (
            "Codex 全局规则",
            RuleKind::Codex,
            "codex",
            ".codex/AGENTS.md",
        )
    } else {
        (
            "Claude Code 全局规则",
            RuleKind::Claude,
            "claude-code",
            ".claude/CLAUDE.md",
        )
    };
    Target {
        id: id.to_string(),
        name: name.to_string(),
        group: "global-rules",
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

/// 全部规则目标：Codex、Claude 两个全局目标在前，随后是各实例（按实例 id 排序）。
/// 定义损坏的实例由 profiles 域报告，这里跳过。
pub(crate) fn resolve_targets(ctx: &Context, policy: &Value) -> Vec<Target> {
    let mut out = vec![
        global(ctx, policy, TARGET_CODEX),
        global(ctx, policy, TARGET_CLAUDE),
    ];
    let mut defs: Vec<profiles::ProfileDef> = profiles::load_definitions(policy)
        .into_iter()
        .filter_map(Result::ok)
        .collect();
    defs.sort_by(|a, b| a.id.cmp(&b.id));
    for def in defs {
        let (kind, file) = match def.kind {
            profiles::ProfileKind::Claude => (RuleKind::Claude, profiles::FILE_CLAUDE_RULES),
            profiles::ProfileKind::Codex => (RuleKind::Codex, profiles::FILE_CODEX_RULES),
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
            name: format!("{} 实例规则", def.display_name),
            group: "profile-rules",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;
    use serde_json::json;

    fn ctx() -> Context {
        Context {
            data_dir: PathBuf::from("/home/u/Library/Application Support/agent-manager"),
            home_dir: PathBuf::from("/home/u"),
        }
    }

    #[test]
    fn global_targets_come_first_in_codex_then_claude_order() {
        let targets = resolve_targets(&ctx(), &store::builtin_policy());
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].id, "rules:codex_global");
        assert_eq!(targets[0].agent_id, "codex");
        assert_eq!(targets[0].path, PathBuf::from("/home/u/.codex/AGENTS.md"));
        assert_eq!(targets[1].id, "rules:claude_global");
        assert_eq!(targets[1].agent_id, "claude-code");
        assert_eq!(targets[1].path, PathBuf::from("/home/u/.claude/CLAUDE.md"));
        assert!(targets.iter().all(|t| t.enabled && t.blocked.is_none()));
    }

    #[test]
    fn profiles_become_targets_sorted_by_id_with_their_own_rules_file() {
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
                "rules:codex_global",
                "rules:claude_global",
                "rules:profile:cc-ly",
                "rules:profile:zz-work"
            ]
        );
        assert_eq!(
            targets[2].path,
            PathBuf::from("/home/u/.claude-ly/CLAUDE.md")
        );
        assert_eq!(targets[2].display, "~/.claude-ly/CLAUDE.md");
        assert_eq!(targets[2].name, "Claude ly 实例规则");
        assert!(targets[2].enabled);
        assert_eq!(
            targets[3].path,
            PathBuf::from("/home/u/.codex-work/AGENTS.md")
        );
        assert_eq!(targets[3].kind, RuleKind::Codex);
        assert!(!targets[3].enabled, "停用的实例目标同样停用");
    }

    #[test]
    fn invalid_profile_dir_is_a_blocked_target() {
        let mut policy = store::builtin_policy();
        policy["agents"]["definitions"]["cc-bad"] = json!({
            "profile": {"base_agent": "claude-code", "config_dir": "~/.claude"}
        });
        let targets = resolve_targets(&ctx(), &policy);
        assert_eq!(targets[2].id, "rules:profile:cc-bad");
        assert!(targets[2].blocked.is_some(), "主实例目录不能当实例目录");
    }

    #[test]
    fn target_id_shapes() {
        assert!(valid_target_id("rules:codex_global"));
        assert!(valid_target_id("rules:claude_global"));
        assert!(valid_target_id("rules:profile:cc-ly"));
        for bad in [
            "rules:codex",
            "rules:",
            "rules:profile:",
            "rules:profile:Bad",
            "rules:profile:a_b",
            "profile:cc-ly",
            "",
        ] {
            assert!(!valid_target_id(bad), "{bad}");
        }
    }
}
