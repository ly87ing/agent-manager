//! 助手适配器的只读能力描述。
//!
//! 后端既负责解释配置，也负责告诉界面“有哪些字段、怎么展示”。前端只按这份
//! schema 渲染，避免每新增一个客户端参数都同时修改 Rust 与 React 枚举。

use serde_json::{json, Value};

/// 旧版本存进库里的取值（含只对单次会话有效的 max 与表示「清除」的 auto）：解析库内旧记录时仍认。
pub(crate) const CLAUDE_EFFORT_LEVELS: &[&str] = &["auto", "low", "medium", "high", "xhigh", "max"];
/// 官方允许持久保存的推理强度（settings.json 的 effortLevel 只接受这四个；max 只在单次会话有效）。
pub(crate) const CLAUDE_EFFORT_PERSISTED: &[&str] = &["low", "medium", "high", "xhigh"];
/// Codex 客户端已知的推理强度（`rust-v0.156.1` `protocol/src/openai_models.rs` 的 `ReasoningEffort`）；
/// 具体模型支持哪几档由模型决定。
pub(crate) const CODEX_REASONING_EFFORTS: &[&str] = &[
    "none",
    "minimal",
    "low",
    "medium",
    "high",
    "xhigh",
    "max",
    "ultra",
    "persistent",
];
pub(crate) const CODEX_WEB_SEARCH_MODES: &[&str] = &["disabled", "cached", "indexed", "live"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdapterKind {
    Claude,
    Codex,
}

impl AdapterKind {
    fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

/// 只按实际执行文件名识别适配器。未知助手保持只读，不能误套另一家的写入规则。
pub(crate) fn kind_for_bin(bin: &str) -> Option<AdapterKind> {
    match std::path::Path::new(bin)
        .file_stem()
        .and_then(|name| name.to_str())
    {
        Some("claude") => Some(AdapterKind::Claude),
        Some("codex") => Some(AdapterKind::Codex),
        _ => None,
    }
}

fn automation_mode(kind: AdapterKind) -> Value {
    let (options, section_description) = match kind {
        AdapterKind::Claude => (
            json!([
                {
                    "value": "manual", "label": "手动确认", "description": "由你确认敏感操作，适合陌生项目。",
                    "targets": [{"key": "permissions.defaultMode", "enabled_value": "\"manual\""}]
                },
                {
                    "value": "auto", "label": "智能自动", "description": "由 Claude 的分类器自动判断；仅在当前账号、模型与组织设置支持时可用。",
                    "targets": [{"key": "permissions.defaultMode", "enabled_value": "\"auto\""}]
                },
                {
                    "value": "full", "label": "完全放开", "description": "绕过所有权限检查，只适合已经隔离的可信环境。",
                    "danger": true, "requires_confirmation": true,
                    "targets": [{"key": "permissions.defaultMode", "enabled_value": "\"bypassPermissions\""}]
                }
            ]),
            "Auto 是否可用由 Claude 当前账号、模型与组织设置决定；完全放开会在保存前再次确认。",
        ),
        AdapterKind::Codex => (
            json!([
                {
                    "value": "manual", "label": "手动确认", "description": "保留工作区沙箱，越界操作由你确认。",
                    "targets": [
                        {"key": "approval_policy", "enabled_value": "\"on-request\""},
                        {"key": "sandbox_mode", "enabled_value": "\"workspace-write\""},
                        {"key": "approvals_reviewer", "enabled_value": "\"user\""}
                    ]
                },
                {
                    "value": "auto", "label": "智能自动", "description": "保留工作区沙箱，由独立审查器处理符合条件的越界请求。",
                    "recommended": true,
                    "targets": [
                        {"key": "approval_policy", "enabled_value": "\"on-request\""},
                        {"key": "sandbox_mode", "enabled_value": "\"workspace-write\""},
                        {"key": "approvals_reviewer", "enabled_value": "\"auto_review\""}
                    ]
                },
                {
                    "value": "full", "label": "完全放开", "description": "不询问且移除沙箱，只适合外部已经隔离的环境。",
                    "danger": true, "requires_confirmation": true,
                    "targets": [
                        {"key": "approval_policy", "enabled_value": "\"never\""},
                        {"key": "sandbox_mode", "enabled_value": "\"danger-full-access\""}
                    ]
                }
            ]),
            "建议使用智能自动；完全放开会在保存前再次确认。",
        ),
    };
    json!({
        "key": "automation_mode",
        "type": "enum",
        "presentation": "choice_cards",
        "section": "权限与执行",
        "section_description": section_description,
        "label": "自动化程度",
        "description": "选择助手如何处理需要权限的操作。",
        "options": options,
        "targets": [],
        "summary": {
            "label": "自动化", "default": "手动确认",
            "values": {"manual": "手动确认", "auto": "智能自动", "full": "完全放开"}
        }
    })
}

fn claude_fields() -> Vec<Value> {
    vec![
        automation_mode(AdapterKind::Claude),
        json!({
            "key": "statusline", "type": "enum", "section": "界面与模型行为",
            "label": "状态栏", "description": "选择是否使用 Agent Manager 提供的本地用量状态栏。",
            "options": [
                {"value": "", "label": "保持现有状态栏"},
                {"value": "builtin", "label": "使用 Agent Manager 用量状态栏"}
            ],
            "targets": [{"key": "statusLine.command", "enabled_value": "<Agent Manager statusline.sh>"}],
            "summary": {"label": "状态栏", "values": {"builtin": "用量状态栏"}, "default": "保持现状"}
        }),
        json!({
            "key": "claude_effort_level", "type": "enum", "section": "界面与模型行为",
            "label": "默认推理强度",
            "description": "写入 settings.json 的 effortLevel，作为没有单独保存级别的模型的默认值；在 Claude Code 里用 /effort 按模型保存的级别优先。留空即撤掉。",
            "options": CLAUDE_EFFORT_PERSISTED.iter().map(|value| json!({"value": value, "label": value})).collect::<Vec<_>>(),
            "targets": [{"key": "effortLevel"}],
            "summary": {"label": "推理强度", "default": "客户端默认"}
        }),
        json!({
            "key": "claude_tool_search", "type": "boolean", "section": "高级能力",
            "label": "工具搜索", "description": "让 Claude Code 按需发现工具，减少一次性加载的工具描述。",
            "targets": [{"key": "env.ENABLE_TOOL_SEARCH", "enabled_value": "\"true\""}],
            "summary": {"label": "工具搜索", "enabled": "开启", "default": "默认"}
        }),
        json!({
            "key": "claude_agent_teams", "type": "boolean", "section": "高级能力",
            "label": "Agent Teams", "description": "启用 Claude Code 的实验性多 Agent 协作能力。",
            "targets": [{"key": "env.CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "enabled_value": "\"1\""}],
            "summary": {"label": "Agent Teams", "enabled": "开启", "default": "默认"}
        }),
        json!({
            "key": "claude_disable_autoupdate", "type": "boolean", "section": "高级能力",
            "label": "禁止自动更新",
            "description": "写入 settings.json 的 env.DISABLE_AUTOUPDATER=1，停掉 Claude Code 启动时和运行中的后台自动更新；手动 claude update 仍可用。通过系统包管理器安装的本来就不自动更新。",
            "targets": [{"key": "env.DISABLE_AUTOUPDATER", "enabled_value": "\"1\""}],
            "summary": {"label": "自动更新", "enabled": "已禁止", "default": "默认"}
        }),
        json!({
            "key": "claude_disable_artifact", "type": "boolean", "section": "高级能力",
            "label": "禁用 Artifact", "description": "不让 Claude Code 创建 Artifact；适合只需要代码与终端输出的环境。",
            "targets": [{"key": "enableArtifact", "enabled_value": "false"}]
        }),
        json!({
            "key": "claude_hide_attribution", "type": "boolean", "section": "高级能力",
            "label": "隐藏提交与 PR 署名", "description": "把 Claude 的 commit 和 PR attribution 模板设为空。",
            "targets": [
                {"key": "attribution.commit", "enabled_value": "\"\""},
                {"key": "attribution.pr", "enabled_value": "\"\""}
            ]
        }),
    ]
}

fn codex_fields() -> Vec<Value> {
    vec![
        automation_mode(AdapterKind::Codex),
        json!({
            "key": "codex_reasoning_effort", "type": "enum", "section": "模型与上下文",
            "section_description": "留空即删除本工具的覆盖，恢复 Codex 自己的默认值。",
            "label": "推理强度", "description": "选择 Codex 的模型推理强度；可用档位取决于所用模型。",
            "options": CODEX_REASONING_EFFORTS.iter().map(|value| json!({"value": value, "label": value})).collect::<Vec<_>>(),
            "targets": [{"key": "model_reasoning_effort"}],
            "summary": {"label": "推理强度", "default": "客户端默认"}
        }),
        json!({
            "key": "codex_web_search", "type": "enum", "section": "模型与上下文",
            "label": "Web 搜索模式", "description": "选择 Codex 使用的 Web 搜索策略。",
            "options": CODEX_WEB_SEARCH_MODES.iter().map(|value| json!({"value": value, "label": value})).collect::<Vec<_>>(),
            "targets": [{"key": "web_search"}],
            "summary": {"label": "Web 搜索", "default": "客户端默认"}
        }),
        json!({
            "key": "codex_context_window", "type": "positive_integer", "section": "模型与上下文",
            "label": "上下文窗口（tokens）", "description": "留空使用 Codex 默认上下文窗口。",
            "placeholder": "例如 1000000", "targets": [{"key": "model_context_window"}],
            "summary": {"label": "上下文", "default": "客户端默认"}
        }),
        json!({
            "key": "codex_auto_compact_token_limit", "type": "positive_integer", "section": "模型与上下文",
            "label": "自动压缩阈值（tokens）", "description": "留空使用 Codex 默认自动压缩阈值。",
            "placeholder": "例如 900000", "targets": [{"key": "model_auto_compact_token_limit"}],
            "summary": {"label": "自动压缩", "default": "客户端默认"}
        }),
    ]
}

/// `agents:status` 输出的管理能力。它只描述能力，不读写客户端文件。
pub(crate) fn management_for_bin(bin: &str) -> Value {
    let Some(kind) = kind_for_bin(bin) else {
        return Value::Null;
    };
    let fields = match kind {
        AdapterKind::Claude => claude_fields(),
        AdapterKind::Codex => codex_fields(),
    };
    json!({
        "adapter": kind.id(),
        "schema_version": 1,
        "connection_kind": kind.id(),
        "supports_profiles": true,
        "runtime_fields": fields
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 适配器能力按执行文件识别_未知助手保持空能力() {
        assert_eq!(
            kind_for_bin("/usr/local/bin/claude"),
            Some(AdapterKind::Claude)
        );
        assert_eq!(kind_for_bin("codex.exe"), Some(AdapterKind::Codex));
        assert_eq!(kind_for_bin("opencode"), None);
        assert!(management_for_bin("opencode").is_null());
    }

    #[test]
    fn schema_枚举与后端校验词汇表同源() {
        let claude = management_for_bin("claude");
        let fields = claude["runtime_fields"].as_array().unwrap();
        let effort = fields
            .iter()
            .find(|field| field["key"] == "claude_effort_level")
            .unwrap();
        let values: Vec<&str> = effort["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|option| option["value"].as_str().unwrap())
            .collect();
        // 界面可选值 = 运行参数补丁校验的词汇表 = 官方允许持久保存的级别
        assert_eq!(values, CLAUDE_EFFORT_PERSISTED);
    }

    #[test]
    fn 自动化档位暴露真实写入键且不把受账号限制的_claude_auto_标为推荐() {
        let claude = management_for_bin("claude");
        let automation = claude["runtime_fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["key"] == "automation_mode")
            .unwrap();
        let claude_auto = automation["options"]
            .as_array()
            .unwrap()
            .iter()
            .find(|option| option["value"] == "auto")
            .unwrap();
        assert!(claude_auto.get("recommended").is_none());
        assert_eq!(
            claude_auto["targets"][0],
            json!({"key": "permissions.defaultMode", "enabled_value": "\"auto\""})
        );

        let codex = management_for_bin("codex");
        let automation = codex["runtime_fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["key"] == "automation_mode")
            .unwrap();
        let codex_auto = automation["options"]
            .as_array()
            .unwrap()
            .iter()
            .find(|option| option["value"] == "auto")
            .unwrap();
        assert_eq!(codex_auto["recommended"], true);
        assert_eq!(codex_auto["targets"][2]["key"], "approvals_reviewer");
    }
}
