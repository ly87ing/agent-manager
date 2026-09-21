//! 助手适配器的只读能力描述。
//!
//! 后端既负责解释配置，也负责告诉界面“有哪些字段、怎么展示”。前端只按这份
//! schema 渲染，避免每新增一个客户端参数都同时修改 Rust 与 React 枚举。

use serde_json::{json, Value};

pub(crate) const CLAUDE_EFFORT_LEVELS: &[&str] = &["auto", "low", "medium", "high", "xhigh", "max"];
pub(crate) const CODEX_REASONING_EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh"];
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

fn shared_yolo(kind: AdapterKind) -> Value {
    let targets = match kind {
        AdapterKind::Claude => json!([
            {"key": "permissions.defaultMode", "enabled_value": "\"bypassPermissions\""}
        ]),
        AdapterKind::Codex => json!([
            {"key": "approval_policy", "enabled_value": "\"never\""},
            {"key": "sandbox_mode", "enabled_value": "\"danger-full-access\""}
        ]),
    };
    json!({
        "key": "yolo",
        "type": "boolean",
        "section": "权限与执行",
        "section_description": "高风险设置仍会在保存后要求二次确认。",
        "label": "跳过权限确认",
        "description": "开启后执行命令和修改文件不再逐次询问；关闭时撤销本工具写入的权限覆盖。",
        "danger": true,
        "requires_yolo_confirmation": true,
        "targets": targets,
        "summary": {"label": "权限确认", "enabled": "已跳过", "default": "客户端默认"}
    })
}

fn claude_fields() -> Vec<Value> {
    vec![
        shared_yolo(AdapterKind::Claude),
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
            "label": "推理强度", "description": "留空即恢复 Claude 自己的默认值。",
            "options": CLAUDE_EFFORT_LEVELS.iter().map(|value| json!({"value": value, "label": value})).collect::<Vec<_>>(),
            "targets": [{"key": "env.CLAUDE_CODE_EFFORT_LEVEL"}],
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
            "key": "claude_disable_artifact", "type": "boolean", "section": "高级能力",
            "label": "禁用 Artifact", "description": "不让 Claude Code 创建 Artifact；适合只需要代码与终端输出的环境。",
            "targets": [{"key": "env.CLAUDE_CODE_DISABLE_ARTIFACT", "enabled_value": "\"1\""}]
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
        shared_yolo(AdapterKind::Codex),
        json!({
            "key": "codex_reasoning_effort", "type": "enum", "section": "模型与上下文",
            "section_description": "留空即删除本工具的覆盖，恢复 Codex 自己的默认值。",
            "label": "推理强度", "description": "选择 Codex 的模型推理强度。",
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
        assert_eq!(values, CLAUDE_EFFORT_LEVELS);
    }
}
